//! Conflict rules, shared by devices and the server so both converge.
//!
//! A row version is its payload (TLV) plus the `(hlc, node)` of its last
//! write and the operation that produced it. Payload tag [`TAG_DELETED_AT`]
//! marks a tombstone.
//!
//! - **Deletes win**: a live version never replaces a tombstone unless it is
//!   an explicit restore newer than the tombstone; a tombstone replaces a live
//!   version unless that version is a restore newer than the tombstone.
//! - **Rows without field clocks** (no tag [`TAG_FIELD_CLOCKS`]): the greater
//!   `(hlc, node)` wins whole.
//! - **Rows with field clocks**, both live: every tag is decided on its own by
//!   its field clock (missing entry = the row's clock), so concurrent edits of
//!   different fields both survive. Tags the reader does not know merge too.

use std::collections::{BTreeMap, BTreeSet};

use crate::protocol::op;
use crate::tlv::{Tlv, TlvError};

/// Tombstone marker (`deleted_at`).
pub const TAG_DELETED_AT: u16 = 1;
/// Field clocks: tag → [`FieldClock`] record.
pub const TAG_FIELD_CLOCKS: u16 = 999;

/// Size limit of a field-clock record.
const CLOCKS_LIMIT: usize = 64 * 1024;

/// Last write of one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldClock {
    /// Clock of the write.
    pub hlc: u64,
    /// Writer node.
    pub node: i64,
    /// Hash of the value written ([`value_hash`]); lets the writer tell
    /// which fields changed since.
    pub hash: u64,
}

impl FieldClock {
    const fn key(self) -> (u64, i64) {
        (self.hlc, self.node)
    }
}

/// FNV-1a 64 of a field value (absent = hash of nothing, distinct from
/// every present value by a leading presence byte). Stable across platforms.
#[must_use]
pub fn value_hash(value: Option<&[u8]>) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    let mut feed = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    };
    match value {
        None => feed(0),
        Some(bytes) => {
            feed(1);
            bytes.iter().copied().for_each(&mut feed);
        }
    }
    hash
}

/// Field clocks of a payload.
pub fn read_clocks(payload: &Tlv) -> Result<BTreeMap<u16, FieldClock>, TlvError> {
    let Some(record) = payload.record(TAG_FIELD_CLOCKS, CLOCKS_LIMIT)? else {
        return Ok(BTreeMap::new());
    };
    let mut clocks = BTreeMap::new();
    for tag in record.tags() {
        let entry = record
            .record(tag, CLOCKS_LIMIT)?
            .ok_or(TlvError::Malformed { tag })?;
        clocks.insert(
            tag,
            FieldClock {
                hlc: entry.uint(1)?.ok_or(TlvError::Malformed { tag })?,
                node: entry.int(2)?.ok_or(TlvError::Malformed { tag })?,
                hash: entry.uint(3)?.unwrap_or(0),
            },
        );
    }
    Ok(clocks)
}

/// Encode field clocks as the [`TAG_FIELD_CLOCKS`] record.
#[must_use]
pub fn write_clocks(clocks: &BTreeMap<u16, FieldClock>) -> Tlv {
    let mut record = Tlv::default();
    for (tag, clock) in clocks {
        let mut entry = Tlv::default();
        entry
            .put_uint(1, clock.hlc)
            .put_int(2, clock.node)
            .put_uint(3, clock.hash);
        record.put_record(*tag, &entry);
    }
    record
}

/// Field clocks after a local write at `(hlc, node)`: fields whose value
/// hash changed (or that have no clock yet) get the new clock, the others
/// keep theirs. `previous` are the clocks stored with the row.
#[must_use]
pub fn restamp(
    payload: &Tlv,
    previous: &BTreeMap<u16, FieldClock>,
    hlc: u64,
    node: i64,
) -> BTreeMap<u16, FieldClock> {
    let tags: BTreeSet<u16> = payload
        .tags()
        .chain(previous.keys().copied())
        .filter(|tag| *tag != TAG_FIELD_CLOCKS)
        .collect();
    tags.into_iter()
        .map(|tag| {
            let hash = value_hash(payload.bytes(tag));
            let clock = match previous.get(&tag) {
                Some(clock) if clock.hash == hash => *clock,
                _ => FieldClock { hlc, node, hash },
            };
            (tag, clock)
        })
        .collect()
}

/// One version of a row.
#[derive(Debug, Clone, Copy)]
pub struct Version<'a> {
    /// Operation that produced it ([`op`]).
    pub op: u16,
    /// Clock of its last write.
    pub hlc: u64,
    /// Writer node of its last write.
    pub node: i64,
    /// Its payload.
    pub payload: &'a Tlv,
}

impl Version<'_> {
    const fn key(&self) -> (u64, i64) {
        (self.hlc, self.node)
    }

    fn deleted(&self) -> bool {
        self.payload.contains(TAG_DELETED_AT)
    }
}

/// What to keep when two versions of a row meet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The stored version stays as is.
    KeepStored,
    /// The incoming version replaces it as is.
    TakeIncoming,
    /// A field-by-field merge of both.
    Merged {
        /// Merged payload (with merged field clocks).
        payload: Tlv,
        /// Greatest field clock.
        hlc: u64,
        /// Its node.
        node: i64,
    },
}

/// Whole-row decision, deletes winning unless explicitly restored.
fn row_winner(stored: &Version<'_>, incoming: &Version<'_>) -> Outcome {
    let newer = incoming.key() > stored.key();
    let incoming_wins = match (stored.deleted(), incoming.deleted()) {
        (false, true) => stored.op != op::RESTORE || newer,
        (true, false) => incoming.op == op::RESTORE && newer,
        _ => newer,
    };
    if incoming_wins {
        Outcome::TakeIncoming
    } else {
        Outcome::KeepStored
    }
}

/// Decide between the stored and an incoming version of the same row.
/// Ties keep the stored version, so applying a version twice changes
/// nothing.
pub fn resolve(stored: &Version<'_>, incoming: &Version<'_>) -> Result<Outcome, TlvError> {
    let per_field =
        stored.payload.contains(TAG_FIELD_CLOCKS) || incoming.payload.contains(TAG_FIELD_CLOCKS);
    if !per_field || stored.deleted() || incoming.deleted() {
        return Ok(row_winner(stored, incoming));
    }
    let stored_clocks = read_clocks(stored.payload)?;
    let incoming_clocks = read_clocks(incoming.payload)?;
    // A field without a clock: present = written with the row (older
    // writers); absent = never written, older than any real write.
    let clock_of = |clocks: &BTreeMap<u16, FieldClock>, version: &Version<'_>, tag: u16| {
        clocks.get(&tag).copied().unwrap_or_else(|| {
            let value = version.payload.bytes(tag);
            let (hlc, node) = if value.is_some() {
                version.key()
            } else {
                (0, i64::MIN)
            };
            FieldClock {
                hlc,
                node,
                hash: value_hash(value),
            }
        })
    };
    let tags: BTreeSet<u16> = stored
        .payload
        .tags()
        .chain(incoming.payload.tags())
        .chain(stored_clocks.keys().copied())
        .chain(incoming_clocks.keys().copied())
        .filter(|tag| *tag != TAG_FIELD_CLOCKS)
        .collect();

    let mut merged = Tlv::default();
    let mut clocks = BTreeMap::new();
    let (mut from_stored, mut from_incoming) = (true, true);
    for tag in tags {
        let mine = clock_of(&stored_clocks, stored, tag);
        let theirs = clock_of(&incoming_clocks, incoming, tag);
        let (winner, clock) = if theirs.key() > mine.key() {
            from_stored = false;
            (incoming.payload, theirs)
        } else {
            from_incoming &= mine.key() == theirs.key()
                && stored.payload.bytes(tag) == incoming.payload.bytes(tag);
            (stored.payload, mine)
        };
        if let Some(value) = winner.bytes(tag) {
            merged.put_bytes(tag, value);
        }
        clocks.insert(tag, clock);
    }
    if from_stored {
        return Ok(Outcome::KeepStored);
    }
    if from_incoming {
        return Ok(Outcome::TakeIncoming);
    }
    let (hlc, node) = clocks
        .values()
        .map(|clock| clock.key())
        .max()
        .unwrap_or((incoming.hlc, incoming.node));
    merged.put_record(TAG_FIELD_CLOCKS, &write_clocks(&clocks));
    Ok(Outcome::Merged {
        payload: merged,
        hlc,
        node,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row with fields `tag → value` all written at `(hlc, node)`.
    fn row(fields: &[(u16, u64)], hlc: u64, node: i64) -> Tlv {
        let mut payload = Tlv::default();
        for (tag, value) in fields {
            payload.put_uint(*tag, *value);
        }
        let clocks = restamp(&payload, &BTreeMap::new(), hlc, node);
        payload.put_record(TAG_FIELD_CLOCKS, &write_clocks(&clocks));
        payload
    }

    /// Edit `tag` of `payload` locally at `(hlc, node)`.
    fn edit(payload: &Tlv, tag: u16, value: u64, hlc: u64, node: i64) -> Tlv {
        let previous = read_clocks(payload).unwrap_or_default();
        let mut next = payload.clone();
        next.put_uint(tag, value);
        let clocks = restamp(&next, &previous, hlc, node);
        next.put_record(TAG_FIELD_CLOCKS, &write_clocks(&clocks));
        next
    }

    fn version(payload: &Tlv, op: u16) -> Version<'_> {
        let clocks = read_clocks(payload).unwrap_or_default();
        let (hlc, node) = clocks
            .values()
            .map(|clock| (clock.hlc, clock.node))
            .max()
            .unwrap_or((0, 0));
        Version {
            op,
            hlc,
            node,
            payload,
        }
    }

    /// Apply `incoming` onto `stored` and return the resulting payload.
    fn apply(stored: &Tlv, incoming: &Tlv) -> Tlv {
        match resolve(&version(stored, op::UPSERT), &version(incoming, op::UPSERT)) {
            Ok(Outcome::KeepStored) => stored.clone(),
            Ok(Outcome::TakeIncoming) => incoming.clone(),
            Ok(Outcome::Merged { payload, .. }) => payload,
            Err(err) => panic!("{err}"),
        }
    }

    fn values(payload: &Tlv) -> Vec<(u16, Option<u64>)> {
        payload
            .tags()
            .filter(|tag| *tag != TAG_FIELD_CLOCKS)
            .map(|tag| (tag, payload.uint(tag).ok().flatten()))
            .collect()
    }

    #[test]
    fn concurrent_edits_of_different_fields_both_survive() {
        let base = row(&[(10, 1), (11, 1)], 100, 1);
        let phone = edit(&base, 10, 2, 200, 1);
        let tablet = edit(&base, 11, 3, 300, 2);
        let on_server = apply(&apply(&base, &phone), &tablet);
        assert_eq!(values(&on_server), vec![(10, Some(2)), (11, Some(3))]);
        // Same result in the other order, and applying twice changes nothing.
        let other_order = apply(&apply(&base, &tablet), &phone);
        assert_eq!(values(&other_order), values(&on_server));
        assert_eq!(values(&apply(&on_server, &phone)), values(&on_server));
        assert_eq!(values(&apply(&on_server, &on_server)), values(&on_server));
    }

    #[test]
    fn same_field_edited_twice_keeps_the_later_write() {
        let base = row(&[(10, 1)], 100, 1);
        let early = edit(&base, 10, 5, 200, 2);
        let late = edit(&base, 10, 7, 300, 1);
        assert_eq!(values(&apply(&early, &late)), vec![(10, Some(7))]);
        assert_eq!(values(&apply(&late, &early)), vec![(10, Some(7))]);
    }

    #[test]
    fn unchanged_values_keep_their_clock() {
        let base = row(&[(10, 1), (11, 1)], 100, 1);
        let clocks = read_clocks(&edit(&base, 10, 1, 500, 9)).unwrap_or_default();
        assert_eq!(clocks[&10].hlc, 100, "same value: not a change");
        let clocks = read_clocks(&edit(&base, 11, 4, 500, 9)).unwrap_or_default();
        assert_eq!((clocks[&10].hlc, clocks[&11].hlc), (100, 500));
    }

    #[test]
    fn three_replicas_converge_in_any_order() {
        let base = row(&[(10, 0), (11, 0), (12, 0)], 10, 0);
        let edits = [
            edit(&base, 10, 1, 20, 1),
            edit(&base, 11, 2, 30, 2),
            edit(&edit(&base, 12, 3, 25, 3), 10, 4, 40, 3),
        ];
        let orders = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        let results: Vec<_> = orders
            .iter()
            .map(|order| {
                let merged = order
                    .iter()
                    .fold(base.clone(), |acc, index| apply(&acc, &edits[*index]));
                values(&merged)
            })
            .collect();
        assert!(results.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(
            results[0],
            vec![(10, Some(4)), (11, Some(2)), (12, Some(3))]
        );
    }

    #[test]
    fn deletes_win_unless_restored() {
        let live = row(&[(10, 1)], 100, 1);
        let mut tombstone = row(&[(10, 1)], 50, 2);
        tombstone.put_uint(TAG_DELETED_AT, 1);
        let older_delete = version(&tombstone, op::DELETE);
        assert_eq!(
            resolve(&version(&live, op::UPSERT), &older_delete),
            Ok(Outcome::TakeIncoming)
        );
        assert_eq!(
            resolve(&older_delete, &version(&live, op::UPSERT)),
            Ok(Outcome::KeepStored)
        );
        assert_eq!(
            resolve(&older_delete, &version(&live, op::RESTORE)),
            Ok(Outcome::TakeIncoming)
        );
    }

    #[test]
    fn rows_without_clocks_use_the_row_clock() {
        let mut a = Tlv::default();
        a.put_uint(10, 1);
        let mut b = Tlv::default();
        b.put_uint(10, 2);
        let older = Version {
            op: op::UPSERT,
            hlc: 1,
            node: 1,
            payload: &a,
        };
        let newer = Version {
            op: op::UPSERT,
            hlc: 2,
            node: 1,
            payload: &b,
        };
        assert_eq!(resolve(&older, &newer), Ok(Outcome::TakeIncoming));
        assert_eq!(resolve(&newer, &older), Ok(Outcome::KeepStored));
        assert_eq!(resolve(&newer, &newer), Ok(Outcome::KeepStored));
    }
}

#[cfg(test)]
mod never_written {
    use super::*;

    #[test]
    fn a_field_never_written_loses_to_a_real_write() {
        // Both start without tag 11; one side sets it, the other edits tag 10
        // later (higher row clock).
        let mut base = Tlv::default();
        base.put_uint(10, 1);
        let base_clocks = restamp(&base, &BTreeMap::new(), 100, 1);

        let mut phone = base.clone();
        phone.put_uint(11, 175);
        let phone_clocks = restamp(&phone, &base_clocks, 200, 1);
        phone.put_record(TAG_FIELD_CLOCKS, &write_clocks(&phone_clocks));

        let mut tablet = base;
        tablet.put_uint(10, 2);
        let tablet_clocks = restamp(&tablet, &base_clocks, 300, 2);
        tablet.put_record(TAG_FIELD_CLOCKS, &write_clocks(&tablet_clocks));

        let stored = Version {
            op: op::UPSERT,
            hlc: 200,
            node: 1,
            payload: &phone,
        };
        let incoming = Version {
            op: op::UPSERT,
            hlc: 300,
            node: 2,
            payload: &tablet,
        };
        let Ok(Outcome::Merged { payload, .. }) = resolve(&stored, &incoming) else {
            panic!("expected a merge");
        };
        assert_eq!(payload.uint(10), Ok(Some(2)));
        assert_eq!(payload.uint(11), Ok(Some(175)));
    }
}
