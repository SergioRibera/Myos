//! Sync outbox: which rows changed since the last push.

use myos_core::code::Code;
use rusqlite::Transaction;
use uuid::Uuid;

use crate::{Error, Hlc, Stamp, Store};

/// A synced table. Codes travel in sync payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Entity {
    /// `workout_session`.
    WorkoutSession,
    /// `set_entry`.
    SetEntry,
    /// `profile`.
    Profile,
    /// `profile_goal`.
    ProfileGoal,
    /// `preferences`.
    Preferences,
    /// `custom_exercise` with its targets.
    CustomExercise,
    /// `import_batch`.
    ImportBatch,
    /// `external_activity`.
    ExternalActivity,
    /// `body_measurement`.
    BodyMeasurement,
}

myos_core::codes!(Entity {
    WorkoutSession = 1,
    SetEntry = 2,
    Profile = 3,
    ProfileGoal = 4,
    Preferences = 5,
    CustomExercise = 6,
    ImportBatch = 7,
    ExternalActivity = 8,
    BodyMeasurement = 9,
});

/// What happened to a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    /// Created or changed.
    Upsert,
    /// Tombstoned.
    Delete,
    /// Tombstone lifted on purpose (deletes otherwise win).
    Restore,
}

myos_core::codes!(Op {
    Upsert = 1,
    Delete = 2,
    Restore = 3,
});

/// A row waiting to be pushed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    /// Order of the first unpushed change.
    pub seq: i64,
    /// Idempotency key for the server.
    pub op_id: Uuid,
    /// Table, or `None` for a code this build does not know.
    pub entity: Option<Entity>,
    /// Row.
    pub row_id: Uuid,
    /// Latest operation.
    pub op: Option<Op>,
    /// Clock of the latest write.
    pub hlc: Hlc,
}

pub(crate) fn sql_hlc(hlc: Hlc) -> Result<i64, Error> {
    i64::try_from(hlc.get()).map_err(|_| Error::OutOfRange("hlc"))
}

/// Queue `row_id` for push. A pending entry for the same row is updated in
/// place, keeping its position, with a fresh `op_id` (the old one may already
/// have a server receipt from an attempt that timed out). An explicit restore
/// is not downgraded to a plain upsert: deletes win unless restored.
pub(crate) fn enqueue(
    tx: &Transaction<'_>,
    entity: Entity,
    row_id: Uuid,
    op: Op,
    stamp: Stamp,
) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO sync_outbox (op_id, entity, row_id, op, hlc, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (entity, row_id) WHERE state = 0
         DO UPDATE SET
           op_id = excluded.op_id,
           op = CASE WHEN op = 3 AND excluded.op = 1 THEN 3 ELSE excluded.op END,
           hlc = excluded.hlc",
        (
            Uuid::now_v7().as_bytes().as_slice(),
            entity.code(),
            row_id.as_bytes().as_slice(),
            op.code(),
            sql_hlc(stamp.hlc)?,
            stamp.now_millis,
        ),
    )?;
    Ok(())
}

impl Store {
    /// Pending outbox entries, oldest first.
    pub fn pending_outbox(&self, limit: u32) -> Result<Vec<OutboxEntry>, Error> {
        let mut statement = self.conn().prepare_cached(
            "SELECT seq, op_id, entity, row_id, op, hlc FROM sync_outbox
             WHERE state = 0 ORDER BY seq LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, [u8; 16]>(1)?,
                row.get::<_, u16>(2)?,
                row.get::<_, [u8; 16]>(3)?,
                row.get::<_, u16>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;
        rows.map(|row| {
            let (seq, op_id, entity, row_id, op, hlc) = row?;
            Ok(OutboxEntry {
                seq,
                op_id: Uuid::from_bytes(op_id),
                entity: Entity::from_code(entity),
                row_id: Uuid::from_bytes(row_id),
                op: Op::from_code(op),
                hlc: Hlc::from_raw(u64::try_from(hlc).map_err(|_| Error::OutOfRange("hlc"))?),
            })
        })
        .collect()
    }
}
