//! Training log: sessions and sets.

use std::collections::HashMap;

use jiff::Timestamp;
use jiff::civil::Date;
use myos_core::code::Code;
use myos_core::exercise::ExerciseId;
use myos_core::id::Id;
use myos_core::session::{
    Effort, Rir, Rpe, Session, SessionId, SessionOrigin, SetId, SetKind, SetRecord, Side,
};
use myos_core::units::{Distance, HeartRate, Mass};
use myos_core::volume::DaySpan;
use rusqlite::{Row, params};
use uuid::Uuid;

use crate::convert::{day_from_number, day_number, duration, millis};
use crate::sync::{self, Entity, Op};
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

/// Sessions read for a span of days.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionsRead {
    /// Live sessions, oldest first, with their live sets in order.
    pub sessions: Vec<Session>,
    /// Sets written by a newer app version with codes this build does not
    /// know; left out of `sessions`.
    pub undecodable_sets: u32,
}

/// Columns of a set, in the order [`decode_set`] reads them.
const SET_COLUMNS: &str = "s.exercise_id, s.kind, s.side, s.load_g, s.reps, s.rir, s.rpe_half,
     s.duration_ms, s.distance_m, s.hr_avg_bpm, s.superset";

const fn blob(id: Uuid) -> [u8; 16] {
    *id.as_bytes()
}

fn id_from<T>(bytes: [u8; 16]) -> Id<T> {
    Id::from(Uuid::from_bytes(bytes))
}

/// Stored integer columns of a set's effort: `(rir, rpe_half)`.
const fn effort_columns(effort: Option<Effort>) -> (Option<u8>, Option<u8>) {
    match effort {
        None => (None, None),
        Some(Effort::Rir(rir)) => (Some(rir.get()), None),
        Some(Effort::Rpe(rpe)) => (None, Some(rpe.half_points())),
    }
}

/// Decode a set row starting at column `at`; `None` when a code or value is
/// unknown to this build.
fn decode_set(row: &Row<'_>, at: usize) -> Result<Option<SetRecord>, Error> {
    let exercise: ExerciseId = id_from(row.get(at)?);
    let (Some(kind), Some(side)) = (
        SetKind::from_code(row.get(at + 1)?),
        Side::from_code(row.get(at + 2)?),
    ) else {
        return Ok(None);
    };
    let rir: Option<u8> = row.get(at + 5)?;
    let rpe_half: Option<u8> = row.get(at + 6)?;
    let effort = match (rir, rpe_half) {
        (Some(rir), _) => Rir::new(rir).map(Effort::Rir),
        (None, Some(half)) => Rpe::from_half_points(half).map(Effort::Rpe),
        (None, None) => None,
    };
    let heart_rate = match row.get::<_, Option<u16>>(at + 9)? {
        Some(bpm) => match HeartRate::from_bpm(bpm) {
            Some(rate) => Some(rate),
            None => return Ok(None),
        },
        None => None,
    };
    Ok(Some(SetRecord {
        exercise,
        kind,
        side,
        load: row.get::<_, Option<u32>>(at + 3)?.map(Mass::from_grams),
        reps: row.get(at + 4)?,
        effort,
        duration: row
            .get::<_, Option<i64>>(at + 7)?
            .map(duration)
            .transpose()?,
        distance: row
            .get::<_, Option<u32>>(at + 8)?
            .map(Distance::from_meters),
        heart_rate,
        superset: row.get(at + 10)?,
    }))
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

    /// Close a live session at `ended_at`.
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
        self.tombstone("workout_session", Entity::WorkoutSession, session.uuid())
    }

    /// Append a set to a live session.
    pub fn log_set(&mut self, session: SessionId, set: &SetRecord) -> Result<SetId, Error> {
        let id = Uuid::now_v7();
        let session_id = session.uuid();
        let (rir, rpe_half) = effort_columns(set.effort);
        let duration_ms = set.duration.map(millis).transpose()?;
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
                    rir, rpe_half, duration_ms, distance_m, hr_avg_bpm, superset, completed_at,
                    hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                    ?17, ?18, ?19)",
                params![
                    blob(id),
                    blob(session_id),
                    day,
                    position,
                    blob(set.exercise.uuid()),
                    set.kind.code(),
                    set.side.code(),
                    set.load.map(Mass::grams),
                    set.reps,
                    rir,
                    rpe_half,
                    duration_ms,
                    set.distance.map(Distance::meters),
                    set.heart_rate.map(HeartRate::bpm),
                    set.superset,
                    stamp.now_millis,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::SetEntry, id, Op::Upsert, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// Replace what a live set records (typo fixes, late effort ratings).
    pub fn update_set(&mut self, set_id: SetId, set: &SetRecord) -> Result<(), Error> {
        let id = set_id.uuid();
        let (rir, rpe_half) = effort_columns(set.effort);
        let duration_ms = set.duration.map(millis).transpose()?;
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE set_entry SET exercise_id = ?2, kind = ?3, side = ?4, load_g = ?5,
                   reps = ?6, rir = ?7, rpe_half = ?8, duration_ms = ?9, distance_m = ?10,
                   hr_avg_bpm = ?11, superset = ?12, hlc = ?13, node = ?14, local_seq = ?15
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![
                    blob(id),
                    blob(set.exercise.uuid()),
                    set.kind.code(),
                    set.side.code(),
                    set.load.map(Mass::grams),
                    set.reps,
                    rir,
                    rpe_half,
                    duration_ms,
                    set.distance.map(Distance::meters),
                    set.heart_rate.map(HeartRate::bpm),
                    set.superset,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            if changed == 0 {
                return Err(Error::NotFound("live set"));
            }
            sync::enqueue(tx, Entity::SetEntry, id, Op::Upsert, stamp)
        })
    }

    /// Tombstone a set.
    pub fn delete_set(&mut self, set_id: SetId) -> Result<(), Error> {
        self.tombstone("set_entry", Entity::SetEntry, set_id.uuid())
    }

    fn tombstone(&mut self, table: &'static str, entity: Entity, id: Uuid) -> Result<(), Error> {
        // `table` is one of this module's constants, never user input.
        let sql = format!(
            "UPDATE {table} SET deleted_at = ?2, hlc = ?3, node = ?4, local_seq = ?5
             WHERE id = ?1 AND deleted_at IS NULL"
        );
        self.write(|tx, stamp| {
            let changed = tx.execute(
                &sql,
                params![
                    blob(id),
                    stamp.now_millis,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            if changed == 0 {
                return Err(Error::NotFound(table));
            }
            sync::enqueue(tx, entity, id, Op::Delete, stamp)
        })
    }

    /// Live sets of a live session, in order. Sets with codes this build does
    /// not know are left out.
    pub fn sets_of(&self, session: SessionId) -> Result<Vec<StoredSet>, Error> {
        let mut statement = self.conn().prepare_cached(&format!(
            "SELECT s.id, {SET_COLUMNS}
             FROM set_entry s JOIN workout_session w ON w.id = s.session_id
             WHERE s.session_id = ?1 AND s.deleted_at IS NULL AND w.deleted_at IS NULL
             ORDER BY s.position, s.id"
        ))?;
        let mut rows = statement.query([blob(session.uuid())])?;
        let mut sets = Vec::new();
        while let Some(row) = rows.next()? {
            if let Some(record) = decode_set(row, 1)? {
                sets.push(StoredSet {
                    id: id_from(row.get(0)?),
                    record,
                });
            }
        }
        Ok(sets)
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
        while let Some(row) = rows.next()? {
            let id: [u8; 16] = row.get(0)?;
            let started_at = Timestamp::from_millisecond(row.get(1)?)
                .map_err(|_| Error::OutOfRange("started_at"))?;
            index.insert(id, sessions.len());
            sessions.push(Session {
                id: id_from(id),
                started_at,
                day: day_from_number(row.get(2)?)?,
                sets: Vec::new(),
            });
        }

        let mut statement = self.conn().prepare_cached(&format!(
            "SELECT s.session_id, {SET_COLUMNS}
             FROM set_entry s JOIN workout_session w ON w.id = s.session_id
             WHERE s.deleted_at IS NULL AND w.deleted_at IS NULL AND s.day BETWEEN ?1 AND ?2
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
        })
    }
}
