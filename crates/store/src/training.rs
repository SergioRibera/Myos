//! Training log: sessions and sets.

use std::collections::HashMap;
use std::time::Duration;

use jiff::Timestamp;
use jiff::civil::Date;
use myos_core::code::Code;
use myos_core::exercise::ExerciseId;
use myos_core::id::Id;
use myos_core::session::{
    Effort, HeartRateZones, Rir, Rpe, Session, SessionId, SessionOrigin, SetId, SetKind, SetRecord,
    Side, Stroke,
};
use myos_core::units::{Distance, HeartRate, Mass};
use myos_core::volume::DaySpan;
use rusqlite::{Row, Transaction, params};
use uuid::Uuid;

use crate::convert::{day_from_number, day_number, duration, millis};
use crate::sync::{self, Entity, Op};
use crate::tlv::Tlv;
use crate::{Error, Store};

/// A session about to be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionDraft {
    /// Start instant.
    pub started_at: Timestamp,
    /// Training day on the user's clock.
    pub day: Date,
    /// Offset of the user's clock from UTC, in minutes.
    pub tz_offset_min: i16,
    /// How the session was created.
    pub origin: SessionOrigin,
}

/// A logged set with its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSet {
    /// Set id.
    pub id: SetId,
    /// What was logged.
    pub record: SetRecord,
}

/// Sets of one session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetsRead {
    /// Live sets, in order.
    pub sets: Vec<StoredSet>,
    /// Sets written by a newer app version with values this build does not
    /// know; left out of `sets`.
    pub undecodable: u32,
}

/// Sessions read for a span of days.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionsRead {
    /// Live sessions, oldest first, with their live sets in order.
    pub sessions: Vec<Session>,
    /// Sets left out because this build cannot decode them.
    pub undecodable_sets: u32,
    /// Sessions with values this build cannot read; left out with their sets.
    pub undecodable_sessions: u32,
}

/// Exercise source recorded in `exercise_ref`, from the id's UUID version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExerciseSource {
    /// Bundled catalog (UUID v5 from the slug).
    Catalog,
    /// User or organization exercise (UUID v7).
    Custom,
    /// Any other version: kept, resolved later.
    Unresolved,
}

impl ExerciseSource {
    const fn of(id: Uuid) -> Self {
        match id.get_version_num() {
            5 => Self::Catalog,
            7 => Self::Custom,
            _ => Self::Unresolved,
        }
    }

    const fn code(self) -> u8 {
        match self {
            Self::Unresolved => 0,
            Self::Catalog => 1,
            Self::Custom => 2,
        }
    }
}

/// Columns of a set, in the order [`decode_set`] reads them.
const SET_COLUMNS: &str = "s.exercise_id, s.kind, s.side, s.load_g, s.reps, s.rir, s.rpe_half,
     s.duration_ms, s.distance_m, s.hr_avg_bpm, s.superset, s.assist_g, s.bodyweight_g,
     s.hr_max_bpm, s.stroke, s.rest_ms, s.hr_zones";

/// Largest stored load (`set_entry.load_g` check).
const MAX_LOAD_G: u32 = 4_000_000;
/// Largest assistance or body weight (`assist_g`, `bodyweight_g` checks).
const MAX_BODY_G: u32 = 400_000;
/// Largest rep count.
const MAX_REPS: u16 = 10_000;
/// Longest set.
const MAX_DURATION: Duration = Duration::from_secs(7 * 24 * 3600);
/// Longest rest.
const MAX_REST: Duration = Duration::from_secs(24 * 3600);
/// Size limit of a stored zones record.
const ZONES_LIMIT: usize = 128;

const fn blob(id: Uuid) -> [u8; 16] {
    *id.as_bytes()
}

fn id_from<T>(bytes: [u8; 16]) -> Id<T> {
    Id::from(Uuid::from_bytes(bytes))
}

/// A set as stored columns, validated against the table's checks.
struct SetColumns {
    exercise: [u8; 16],
    kind: u16,
    side: u16,
    load_g: Option<u32>,
    reps: Option<u16>,
    rir: Option<u8>,
    rpe_half: Option<u8>,
    duration_ms: Option<i64>,
    distance_m: Option<u32>,
    hr_avg_bpm: Option<u16>,
    superset: Option<u16>,
    assist_g: Option<u32>,
    bodyweight_g: Option<u32>,
    hr_max_bpm: Option<u16>,
    stroke: Option<u16>,
    rest_ms: Option<i64>,
    hr_zones: Option<Vec<u8>>,
}

impl SetColumns {
    fn new(set: &SetRecord) -> Result<Self, Error> {
        let grams = |mass: Option<Mass>, max: u32, what| match mass.map(Mass::grams) {
            Some(grams) if grams > max => Err(Error::OutOfRange(what)),
            grams => Ok(grams),
        };
        let bounded = |value: Option<Duration>, max: Duration, what| match value {
            Some(value) if value > max => Err(Error::OutOfRange(what)),
            value => value.map(millis).transpose(),
        };
        if set.reps.is_some_and(|reps| reps > MAX_REPS) {
            return Err(Error::OutOfRange("reps"));
        }
        if let (Some(avg), Some(max)) = (set.heart_rate, set.max_heart_rate)
            && max < avg
        {
            return Err(Error::OutOfRange("max_heart_rate"));
        }
        let (rir, rpe_half) = match set.effort {
            None => (None, None),
            Some(Effort::Rir(rir)) => (Some(rir.get()), None),
            Some(Effort::Rpe(rpe)) => (None, Some(rpe.half_points())),
        };
        if let Some(zones) = set.zones {
            let total = zones
                .0
                .iter()
                .try_fold(Duration::ZERO, |total, zone| total.checked_add(*zone))
                .ok_or(Error::OutOfRange("zones"))?;
            if total > set.duration.unwrap_or(MAX_DURATION) {
                return Err(Error::OutOfRange("zones"));
            }
        }
        let hr_zones = set.zones.map(encode_zones).transpose()?;
        Ok(Self {
            exercise: blob(set.exercise.uuid()),
            kind: set.kind.code(),
            side: set.side.code(),
            load_g: grams(set.load, MAX_LOAD_G, "load")?,
            reps: set.reps,
            rir,
            rpe_half,
            duration_ms: bounded(set.duration, MAX_DURATION, "duration")?,
            distance_m: set.distance.map(Distance::meters),
            hr_avg_bpm: set.heart_rate.map(HeartRate::bpm),
            superset: set.superset,
            assist_g: grams(set.assist, MAX_BODY_G, "assist")?,
            bodyweight_g: grams(set.bodyweight, MAX_BODY_G, "bodyweight")?,
            hr_max_bpm: set.max_heart_rate.map(HeartRate::bpm),
            stroke: set.stroke.map(Stroke::code),
            rest_ms: bounded(set.rest, MAX_REST, "rest")?,
            hr_zones,
        })
    }
}

/// Zones as a TLV record: tag `n` (1..=5) holds milliseconds in zone `n`.
fn encode_zones(zones: HeartRateZones) -> Result<Vec<u8>, Error> {
    let mut record = Tlv::default();
    for (tag, time) in (1_u16..).zip(zones.0) {
        let ms = u64::try_from(time.as_millis()).map_err(|_| Error::OutOfRange("zones"))?;
        record.put_uint(tag, ms);
    }
    Ok(record.encode())
}

fn decode_zones(bytes: &[u8]) -> Option<HeartRateZones> {
    let record = Tlv::decode(bytes, ZONES_LIMIT).ok()?;
    let mut zones = HeartRateZones::default();
    for (tag, slot) in (1_u16..).zip(zones.0.iter_mut()) {
        *slot = Duration::from_millis(record.uint(tag).ok()?.unwrap_or(0));
    }
    Some(zones)
}

/// An integer column narrowed to `T`; `Err(())` when the stored value does
/// not fit (written by a newer app version).
fn narrow<T: TryFrom<i64>>(row: &Row<'_>, at: usize) -> Result<Result<Option<T>, ()>, Error> {
    Ok(row
        .get::<_, Option<i64>>(at)?
        .map(T::try_from)
        .transpose()
        .map_err(|_| ()))
}

/// Decode a set row starting at column `at`; `None` when a code or value is
/// outside what this build understands.
fn decode_set(row: &Row<'_>, at: usize) -> Result<Option<SetRecord>, Error> {
    // Early-return `Ok(None)` on a value this build cannot represent.
    macro_rules! field {
        ($ty:ty, $offset:expr) => {
            match narrow::<$ty>(row, at + $offset)? {
                Ok(value) => value,
                Err(()) => return Ok(None),
            }
        };
    }
    macro_rules! known {
        ($value:expr) => {
            match $value {
                Some(value) => value,
                None => return Ok(None),
            }
        };
    }
    macro_rules! optional {
        ($value:expr, $decode:expr) => {
            match $value {
                Some(raw) => Some(known!($decode(raw))),
                None => None,
            }
        };
    }
    let exercise: ExerciseId = id_from(row.get(at)?);
    let kind = known!(field!(u16, 1).and_then(SetKind::from_code));
    let side = known!(field!(u16, 2).and_then(Side::from_code));
    let effort = match (field!(u8, 5), field!(u8, 6)) {
        (None, None) => None,
        (Some(rir), None) => Some(Effort::Rir(known!(Rir::new(rir)))),
        (None, Some(half)) => Some(Effort::Rpe(known!(Rpe::from_half_points(half)))),
        (Some(_), Some(_)) => return Ok(None),
    };
    let heart_rate = optional!(field!(u16, 9), HeartRate::from_bpm);
    let max_heart_rate = optional!(field!(u16, 13), HeartRate::from_bpm);
    let stroke = optional!(field!(u16, 14), Stroke::from_code);
    let zones = optional!(row.get::<_, Option<Vec<u8>>>(at + 16)?, |bytes: Vec<u8>| {
        decode_zones(&bytes)
    });
    let duration_of = |value: Option<i64>| value.map(duration).transpose();
    Ok(Some(SetRecord {
        exercise,
        kind,
        side,
        load: field!(u32, 3).map(Mass::from_grams),
        reps: field!(u16, 4),
        effort,
        duration: duration_of(field!(i64, 7))?,
        distance: field!(u32, 8).map(Distance::from_meters),
        heart_rate,
        superset: field!(u16, 10),
        assist: field!(u32, 11).map(Mass::from_grams),
        bodyweight: field!(u32, 12).map(Mass::from_grams),
        max_heart_rate,
        stroke,
        rest: duration_of(field!(i64, 15))?,
        zones,
    }))
}

/// Record that `exercise` is referenced (local index, not synced).
fn reference_exercise(tx: &Transaction<'_>, exercise: [u8; 16]) -> Result<(), Error> {
    let source = ExerciseSource::of(Uuid::from_bytes(exercise));
    tx.execute(
        "INSERT INTO exercise_ref (id, src) VALUES (?1, ?2) ON CONFLICT (id) DO NOTHING",
        params![exercise, source.code()],
    )?;
    Ok(())
}

impl Store {
    /// Start a session.
    pub fn start_session(&mut self, draft: SessionDraft) -> Result<SessionId, Error> {
        let id = Uuid::now_v7();
        let day = day_number(draft.day)?;
        if !(-840..=840).contains(&draft.tz_offset_min) {
            return Err(Error::OutOfRange("tz_offset_min"));
        }
        self.write(|tx, stamp| {
            tx.execute(
                "INSERT INTO workout_session
                   (id, started_at, day, tz_offset_min, origin, hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    blob(id),
                    draft.started_at.as_millisecond(),
                    day,
                    draft.tz_offset_min,
                    draft.origin.code(),
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::WorkoutSession, id, Op::Upsert, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// Set (or correct) the end of a live session. Sets can still be added
    /// after it: users often finish logging once they leave the gym.
    pub fn end_session(&mut self, session: SessionId, ended_at: Timestamp) -> Result<(), Error> {
        let id = session.uuid();
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE workout_session SET ended_at = ?2, hlc = ?3, node = ?4, local_seq = ?5
                 WHERE id = ?1 AND deleted_at IS NULL AND ?2 >= started_at",
                params![
                    blob(id),
                    ended_at.as_millisecond(),
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            if changed == 0 {
                return Err(Error::NotFound("live session started before ended_at"));
            }
            sync::enqueue(tx, Entity::WorkoutSession, id, Op::Upsert, stamp)
        })
    }

    /// Tombstone a session; its sets stop counting with it.
    pub fn delete_session(&mut self, session: SessionId) -> Result<(), Error> {
        let id = session.uuid();
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE workout_session SET deleted_at = ?2, hlc = ?3, node = ?4, local_seq = ?5
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![
                    blob(id),
                    stamp.now_millis,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            if changed == 0 {
                return Err(Error::NotFound("live session"));
            }
            sync::enqueue(tx, Entity::WorkoutSession, id, Op::Delete, stamp)
        })
    }

    /// Append a set to a live session.
    pub fn log_set(&mut self, session: SessionId, set: &SetRecord) -> Result<SetId, Error> {
        let id = Uuid::now_v7();
        let session_id = session.uuid();
        let columns = SetColumns::new(set)?;
        self.write(|tx, stamp| {
            let (day, position): (i64, i64) = tx
                .query_row(
                    "SELECT w.day, coalesce(
                       (SELECT max(position) + 1 FROM set_entry WHERE session_id = w.id), 0)
                     FROM workout_session w WHERE w.id = ?1 AND w.deleted_at IS NULL",
                    [blob(session_id)],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|err| match err {
                    rusqlite::Error::QueryReturnedNoRows => Error::NotFound("live session"),
                    other => Error::Sqlite(other),
                })?;
            tx.execute(
                "INSERT INTO set_entry
                   (id, session_id, day, position, exercise_id, kind, side, load_g, reps,
                    rir, rpe_half, duration_ms, distance_m, hr_avg_bpm, superset, assist_g,
                    bodyweight_g, hr_max_bpm, stroke, rest_ms, hr_zones, completed_at,
                    hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                    ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
                params![
                    blob(id),
                    blob(session_id),
                    day,
                    position,
                    columns.exercise,
                    columns.kind,
                    columns.side,
                    columns.load_g,
                    columns.reps,
                    columns.rir,
                    columns.rpe_half,
                    columns.duration_ms,
                    columns.distance_m,
                    columns.hr_avg_bpm,
                    columns.superset,
                    columns.assist_g,
                    columns.bodyweight_g,
                    columns.hr_max_bpm,
                    columns.stroke,
                    columns.rest_ms,
                    columns.hr_zones,
                    stamp.now_millis,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            reference_exercise(tx, columns.exercise)?;
            sync::enqueue(tx, Entity::SetEntry, id, Op::Upsert, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// Replace what a live set of a live session records (typo fixes, late
    /// effort ratings).
    pub fn update_set(&mut self, set_id: SetId, set: &SetRecord) -> Result<(), Error> {
        let id = set_id.uuid();
        let columns = SetColumns::new(set)?;
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE set_entry SET exercise_id = ?2, kind = ?3, side = ?4, load_g = ?5,
                   reps = ?6, rir = ?7, rpe_half = ?8, duration_ms = ?9, distance_m = ?10,
                   hr_avg_bpm = ?11, superset = ?12, assist_g = ?13, bodyweight_g = ?14,
                   hr_max_bpm = ?15, stroke = ?16, rest_ms = ?17, hr_zones = ?18,
                   hlc = ?19, node = ?20, local_seq = ?21
                 WHERE id = ?1 AND deleted_at IS NULL AND EXISTS (
                   SELECT 1 FROM workout_session w
                   WHERE w.id = set_entry.session_id AND w.deleted_at IS NULL)",
                params![
                    blob(id),
                    columns.exercise,
                    columns.kind,
                    columns.side,
                    columns.load_g,
                    columns.reps,
                    columns.rir,
                    columns.rpe_half,
                    columns.duration_ms,
                    columns.distance_m,
                    columns.hr_avg_bpm,
                    columns.superset,
                    columns.assist_g,
                    columns.bodyweight_g,
                    columns.hr_max_bpm,
                    columns.stroke,
                    columns.rest_ms,
                    columns.hr_zones,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            if changed == 0 {
                return Err(Error::NotFound("live set"));
            }
            reference_exercise(tx, columns.exercise)?;
            sync::enqueue(tx, Entity::SetEntry, id, Op::Upsert, stamp)
        })
    }

    /// Tombstone a live set of a live session.
    pub fn delete_set(&mut self, set_id: SetId) -> Result<(), Error> {
        let id = set_id.uuid();
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE set_entry SET deleted_at = ?2, hlc = ?3, node = ?4, local_seq = ?5
                 WHERE id = ?1 AND deleted_at IS NULL AND EXISTS (
                   SELECT 1 FROM workout_session w
                   WHERE w.id = set_entry.session_id AND w.deleted_at IS NULL)",
                params![
                    blob(id),
                    stamp.now_millis,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            if changed == 0 {
                return Err(Error::NotFound("live set"));
            }
            sync::enqueue(tx, Entity::SetEntry, id, Op::Delete, stamp)
        })
    }

    /// Live sets of a live session, in order.
    pub fn sets_of(&self, session: SessionId) -> Result<SetsRead, Error> {
        let mut statement = self.conn().prepare_cached(&format!(
            "SELECT s.id, {SET_COLUMNS}
             FROM set_entry s JOIN workout_session w ON w.id = s.session_id
             WHERE s.session_id = ?1 AND s.deleted_at IS NULL AND w.deleted_at IS NULL
             ORDER BY s.position, s.id"
        ))?;
        let mut rows = statement.query([blob(session.uuid())])?;
        let mut read = SetsRead::default();
        while let Some(row) = rows.next()? {
            match decode_set(row, 1)? {
                Some(record) => read.sets.push(StoredSet {
                    id: id_from(row.get(0)?),
                    record,
                }),
                None => read.undecodable += 1,
            }
        }
        Ok(read)
    }

    /// Live sessions whose training day falls in `span`, with their sets.
    pub fn sessions_in(&self, span: DaySpan) -> Result<SessionsRead, Error> {
        let first = day_number(span.first())?;
        let last = day_number(span.last())?;

        let mut statement = self.conn().prepare_cached(
            "SELECT id, started_at, day FROM workout_session
             WHERE deleted_at IS NULL AND day BETWEEN ?1 AND ?2
             ORDER BY started_at, id",
        )?;
        let mut rows = statement.query([first, last])?;
        let mut sessions = Vec::new();
        let mut index = HashMap::new();
        let mut undecodable_sessions = 0;
        while let Some(row) = rows.next()? {
            let id: [u8; 16] = row.get(0)?;
            let (Ok(started_at), Ok(day)) = (
                Timestamp::from_millisecond(row.get(1)?),
                day_from_number(row.get(2)?),
            ) else {
                undecodable_sessions += 1;
                continue;
            };
            index.insert(id, sessions.len());
            sessions.push(Session {
                id: id_from(id),
                started_at,
                day,
                sets: Vec::new(),
            });
        }

        let mut statement = self.conn().prepare_cached(&format!(
            "SELECT s.session_id, {SET_COLUMNS}
             FROM set_entry s JOIN workout_session w ON w.id = s.session_id
             WHERE s.deleted_at IS NULL AND w.deleted_at IS NULL AND w.day BETWEEN ?1 AND ?2
             ORDER BY s.session_id, s.position, s.id"
        ))?;
        let mut rows = statement.query([first, last])?;
        let mut undecodable_sets = 0;
        while let Some(row) = rows.next()? {
            let session: [u8; 16] = row.get(0)?;
            let Some(&at) = index.get(&session) else {
                continue;
            };
            match decode_set(row, 1)? {
                Some(record) => sessions[at].sets.push(record),
                None => undecodable_sets += 1,
            }
        }
        Ok(SessionsRead {
            sessions,
            undecodable_sets,
            undecodable_sessions,
        })
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use myos_core::session::{SessionOrigin, SetKind, SetRecord};

    use super::*;
    use crate::DatabaseKey;

    fn store_with_set() -> Result<(Store, SessionId), Error> {
        let mut store = Store::open_in_memory(&DatabaseKey::generate()?)?;
        let session = store.start_session(SessionDraft {
            started_at: Timestamp::UNIX_EPOCH,
            day: date(2026, 11, 18),
            tz_offset_min: 0,
            origin: SessionOrigin::Manual,
        })?;
        store.log_set(
            session,
            &SetRecord::new(Id::generate()).with_kind(SetKind::Working),
        )?;
        Ok((store, session))
    }

    #[test]
    fn values_a_newer_version_wrote_are_counted_not_fatal() -> Result<(), Error> {
        for (column, value) in [
            ("kind", 99_i64),
            ("side", 77),
            ("stroke", 200),
            ("distance_m", 4_294_967_295),
        ] {
            let (store, session) = store_with_set()?;
            let sql = format!("UPDATE set_entry SET {column} = ?1");
            store.conn.execute(&sql, [value])?;
            let read = store.sets_of(session)?;
            let decodable = column == "distance_m";
            assert_eq!(read.undecodable, u32::from(!decodable), "{column}");
            assert_eq!(read.sets.len(), usize::from(decodable), "{column}");
        }
        Ok(())
    }

    #[test]
    fn exercise_refs_follow_the_uuid_version() -> Result<(), Error> {
        let (store, _) = store_with_set()?;
        let src: i64 = store
            .conn
            .query_row("SELECT src FROM exercise_ref", [], |row| row.get(0))?;
        assert_eq!(src, 2, "v7 ids are user exercises");
        Ok(())
    }

    #[test]
    fn read_only_refuses_writes() -> Result<(), Error> {
        let (mut store, session) = store_with_set()?;
        store
            .conn
            .execute("UPDATE sync_state SET read_only = 1", [])?;
        assert!(matches!(
            store.delete_session(session),
            Err(Error::ReadOnly)
        ));
        Ok(())
    }
}
