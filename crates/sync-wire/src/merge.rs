//! Conflict rules, shared by devices and the server so both converge.
//!
//! A row version is its payload (TLV) plus the `(hlc, node)` of its last
//! write and the operation that produced it. Payload tag [`TAG_DELETED_AT`]
//! marks a tombstone.
//!
//! - **Rows with field clocks** (tag [`TAG_FIELD_CLOCKS`]): every tag,
//!   `deleted_at` included, is a last-writer-wins register decided by its own
//!   clock, ties broken by the value bytes. Concurrent edits of different
//!   fields both survive; a delete stays until a restore clears `deleted_at`
//!   with a newer clock, so a stale tombstone never re-deletes a restored
//!   row. Tags the reader does not know merge too.
//! - **Rows without field clocks**: the greater `(hlc, node)` wins whole
//!   (ties: greater payload bytes), except that deletes win unless restored:
//!   a live version replaces a tombstone only as a newer explicit restore,
//!   and a tombstone replaces a live version unless that version is a newer
//!   restore. A restored row keeps its restore mark through later upserts
//!   ([`op_after`]).

use std::cmp::Ordering;
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
    /// which fields changed since. A clock read without one counts as 0, so
    /// the next write restamps that field.
    pub hash: u64,
}

impl FieldClock {
    const fn key(self) -> (u64, i64) {
        (self.hlc, self.node)
    }
}

/// FNV-1a 64 of a field value; absent and empty values hash differently.
/// Stable across platforms.
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

/// Field clocks after a local write at `(hlc, node)`.
///
/// Fields whose value
/// hash changed (or that have no clock yet) get the new clock, the others
/// keep theirs. `known` lists the tags the writer's schema defines, so absent
/// fields get a clock too (a cleared field then beats a stale value).
#[must_use]
pub fn restamp(
    payload: &Tlv,
    previous: &BTreeMap<u16, FieldClock>,
    hlc: u64,
    node: i64,
    known: &[u16],
) -> BTreeMap<u16, FieldClock> {
    let tags: BTreeSet<u16> = payload
        .tags()
        .chain(previous.keys().copied())
        .chain(known.iter().copied())
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
        /// Clock of the merged row (never older than either input).
        hlc: u64,
        /// Its node.
        node: i64,
    },
}

/// Order of two versions by clock, then payload bytes (deterministic ties).
fn version_order(stored: &Version<'_>, incoming: &Version<'_>) -> Ordering {
    incoming
        .key()
        .cmp(&stored.key())
        .then_with(|| incoming.payload.encode().cmp(&stored.payload.encode()))
}

/// Whole-row decision, deletes winning unless explicitly restored.
fn row_winner(stored: &Version<'_>, incoming: &Version<'_>) -> Outcome {
    let newer = version_order(stored, incoming) == Ordering::Greater;
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

/// Operation to store with the winner of a row-level decision: a restored
/// row keeps its restore mark when a later live upsert replaces it, so a
/// tombstone older than the restore still loses.
#[must_use]
pub fn op_after(stored: &Version<'_>, incoming: &Version<'_>) -> u16 {
    if incoming.op == op::UPSERT && stored.op == op::RESTORE && !incoming.deleted() {
        op::RESTORE
    } else {
        incoming.op
    }
}

/// Decide between the stored and an incoming version of the same row. The
/// result is the same whichever side is "stored" (only the outcome label
/// differs), and applying a version twice changes nothing.
pub fn resolve(stored: &Version<'_>, incoming: &Version<'_>) -> Result<Outcome, TlvError> {
    let per_field =
        stored.payload.contains(TAG_FIELD_CLOCKS) || incoming.payload.contains(TAG_FIELD_CLOCKS);
    if !per_field {
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
    let (mut incoming_won, mut stored_won) = (false, false);
    for tag in tags {
        let mine = clock_of(&stored_clocks, stored, tag);
        let theirs = clock_of(&incoming_clocks, incoming, tag);
        let mine_value = stored.payload.bytes(tag);
        let theirs_value = incoming.payload.bytes(tag);
        let order = theirs
            .key()
            .cmp(&mine.key())
            .then_with(|| theirs_value.cmp(&mine_value));
        let (value, clock) = match order {
            Ordering::Greater => {
                incoming_won = true;
                (theirs_value, theirs)
            }
            Ordering::Less => {
                stored_won = true;
                (mine_value, mine)
            }
            Ordering::Equal => (mine_value, mine),
        };
        if let Some(value) = value {
            merged.put_bytes(tag, value);
        }
        clocks.insert(tag, clock);
    }
    if !incoming_won {
        return Ok(Outcome::KeepStored);
    }
    if !stored_won {
        return Ok(Outcome::TakeIncoming);
    }
    let (hlc, node) = clocks
        .values()
        .map(|clock| clock.key())
        .chain([stored.key(), incoming.key()])
        .max()
        .unwrap_or_else(|| incoming.key());
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

    const KNOWN: &[u16] = &[TAG_DELETED_AT, 10, 11, 12];

    /// A row with fields `tag → value` all written at `(hlc, node)`.
    fn row(fields: &[(u16, u64)], hlc: u64, node: i64) -> Tlv {
        let mut payload = Tlv::default();
        for (tag, value) in fields {
            payload.put_uint(*tag, *value);
        }
        let clocks = restamp(&payload, &BTreeMap::new(), hlc, node, KNOWN);
        payload.put_record(TAG_FIELD_CLOCKS, &write_clocks(&clocks));
        payload
    }

    /// Rewrite `payload` locally at `(hlc, node)` with `change` applied.
    fn local(payload: &Tlv, hlc: u64, node: i64, change: impl FnOnce(&mut Tlv)) -> Tlv {
        let previous = read_clocks(payload).unwrap_or_default();
        let mut next = payload.clone();
        change(&mut next);
        let clocks = restamp(&next, &previous, hlc, node, KNOWN);
        next.put_record(TAG_FIELD_CLOCKS, &write_clocks(&clocks));
        next
    }

    fn edit(payload: &Tlv, tag: u16, value: u64, hlc: u64, node: i64) -> Tlv {
        local(payload, hlc, node, |next| {
            next.put_uint(tag, value);
        })
    }

    fn version(payload: &Tlv) -> Version<'_> {
        let clocks = read_clocks(payload).unwrap_or_default();
        let (hlc, node) = clocks
            .values()
            .map(|clock| (clock.hlc, clock.node))
            .max()
            .unwrap_or((0, 0));
        Version {
            op: op::UPSERT,
            hlc,
            node,
            payload,
        }
    }

    /// Apply `incoming` onto `stored` and return the resulting payload.
    fn apply(stored: &Tlv, incoming: &Tlv) -> Tlv {
        match resolve(&version(stored), &version(incoming)) {
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
    fn equal_clocks_with_different_values_pick_the_same_value_on_both_sides() {
        let mut a = Tlv::default();
        a.put_uint(10, 1);
        let mut b = Tlv::default();
        b.put_uint(10, 2);
        let clocks = |payload: &Tlv| {
            let mut map = BTreeMap::new();
            map.insert(
                10,
                FieldClock {
                    hlc: 5,
                    node: 1,
                    hash: value_hash(payload.bytes(10)),
                },
            );
            write_clocks(&map)
        };
        a.put_record(TAG_FIELD_CLOCKS, &clocks(&a));
        b.put_record(TAG_FIELD_CLOCKS, &clocks(&b));
        assert_eq!(values(&apply(&a, &b)), values(&apply(&b, &a)));
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
    fn a_restore_survives_a_stale_tombstone_in_any_order() {
        let base = row(&[(10, 1)], 100, 1);
        let deleted = local(&base, 200, 1, |next| {
            next.put_uint(TAG_DELETED_AT, 1);
        });
        let restored = local(&deleted, 300, 2, |next| {
            next.remove(TAG_DELETED_AT);
        });
        let edited = edit(&restored, 10, 9, 400, 2);
        // The stale tombstone arrives last, or first.
        let late = apply(&apply(&base, &edited), &deleted);
        let early = apply(&apply(&base, &deleted), &edited);
        assert!(!late.contains(TAG_DELETED_AT));
        assert_eq!(values(&late), values(&early));
        assert_eq!(late.uint(10), Ok(Some(9)));
    }

    #[test]
    fn a_delete_beats_a_concurrent_field_edit() {
        let base = row(&[(10, 1)], 100, 1);
        let deleted = local(&base, 200, 1, |next| {
            next.put_uint(TAG_DELETED_AT, 1);
        });
        let edited = edit(&base, 10, 5, 300, 2);
        assert!(apply(&deleted, &edited).contains(TAG_DELETED_AT));
        assert!(apply(&edited, &deleted).contains(TAG_DELETED_AT));
    }

    #[test]
    fn a_cleared_field_beats_an_older_value() {
        let base = row(&[(10, 1), (11, 1)], 100, 1);
        let cleared = local(&base, 200, 1, |next| {
            next.remove(10);
        });
        let other = edit(&base, 11, 2, 300, 2);
        let merged = apply(&other, &cleared);
        assert_eq!(values(&merged), vec![(11, Some(2))]);
        assert_eq!(values(&apply(&cleared, &other)), values(&merged));
    }

    #[test]
    fn row_level_deletes_win_unless_restored() {
        let mut live = Tlv::default();
        live.put_uint(10, 1);
        let mut tombstone = live.clone();
        tombstone.put_uint(TAG_DELETED_AT, 1);
        let at = |payload, op, hlc| Version {
            op,
            hlc,
            node: 1,
            payload,
        };
        assert_eq!(
            resolve(&at(&live, op::UPSERT, 100), &at(&tombstone, op::DELETE, 50)),
            Ok(Outcome::TakeIncoming)
        );
        assert_eq!(
            resolve(&at(&tombstone, op::DELETE, 50), &at(&live, op::UPSERT, 100)),
            Ok(Outcome::KeepStored)
        );
        assert_eq!(
            resolve(
                &at(&tombstone, op::DELETE, 50),
                &at(&live, op::RESTORE, 100)
            ),
            Ok(Outcome::TakeIncoming)
        );
        // A restored row keeps its mark through an upsert and survives an
        // older tombstone.
        let restored = at(&live, op::RESTORE, 100);
        let upsert = at(&live, op::UPSERT, 150);
        let kept = op_after(&restored, &upsert);
        assert_eq!(kept, op::RESTORE);
        assert_eq!(
            resolve(&at(&live, kept, 150), &at(&tombstone, op::DELETE, 50)),
            Ok(Outcome::KeepStored)
        );
    }

    #[test]
    fn row_level_ties_are_deterministic() {
        let mut a = Tlv::default();
        a.put_uint(10, 1);
        let mut b = Tlv::default();
        b.put_uint(10, 2);
        let at = |payload| Version {
            op: op::UPSERT,
            hlc: 5,
            node: 1,
            payload,
        };
        let one = resolve(&at(&a), &at(&b));
        let two = resolve(&at(&b), &at(&a));
        assert_ne!(one, two, "exactly one side wins");
        assert_eq!(resolve(&at(&a), &at(&a)), Ok(Outcome::KeepStored));
    }

    #[test]
    fn absent_and_empty_values_hash_differently() {
        assert_ne!(value_hash(None), value_hash(Some(&[])));
        let base = row(&[(10, 1)], 100, 1);
        let back = edit(&edit(&base, 10, 2, 200, 1), 10, 1, 300, 1);
        assert_eq!(read_clocks(&back).unwrap_or_default()[&10].hlc, 300);
    }
}
