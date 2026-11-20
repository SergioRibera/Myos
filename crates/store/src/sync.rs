//! Sync outbox: which rows changed since the last push.

use myos_core::code::Code;
use rusqlite::{OptionalExtension, Transaction};
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
    /// `custom_food` with its portions.
    CustomFood,
    /// `recipe` with its items.
    Recipe,
    /// `meal`.
    Meal,
    /// `meal_item`.
    MealItem,
    /// `routine` (head).
    Routine,
    /// `routine_version`.
    RoutineVersion,
    /// `nutrition_plan` (head).
    NutritionPlan,
    /// `nutrition_plan_version`.
    NutritionPlanVersion,
    /// `plan_phase`.
    PlanPhase,
    /// `plan_event`.
    PlanEvent,
    /// `calibration_result`.
    CalibrationResult,
    /// `volume_range_override`.
    VolumeRangeOverride,
    /// `suggestion`.
    Suggestion,
    /// `suggestion_response`.
    SuggestionResponse,
    /// `equipment_profile` with its items.
    EquipmentProfile,
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
    CustomFood = 10,
    Recipe = 11,
    Meal = 12,
    MealItem = 13,
    Routine = 14,
    RoutineVersion = 15,
    NutritionPlan = 16,
    NutritionPlanVersion = 17,
    PlanPhase = 18,
    PlanEvent = 19,
    CalibrationResult = 20,
    VolumeRangeOverride = 21,
    Suggestion = 22,
    SuggestionResponse = 23,
    EquipmentProfile = 24,
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

/// Queue `row_id` for push. A pending entry for the same row is replaced by
/// one at the end of the queue, with a fresh `op_id` (the old one may already
/// have a server receipt from an attempt that timed out): pushing in queue
/// order then always sends a row after what it references (a routine head
/// after its new version). An explicit restore is not downgraded to a plain
/// upsert: deletes win unless restored.
pub(crate) fn enqueue(
    tx: &Transaction<'_>,
    entity: Entity,
    row_id: Uuid,
    op: Op,
    stamp: Stamp,
) -> Result<(), Error> {
    let previous: Option<u16> = tx
        .query_row(
            "DELETE FROM sync_outbox WHERE entity = ?1 AND row_id = ?2 AND state = 0
             RETURNING op",
            (entity.code(), row_id.as_bytes().as_slice()),
            |row| row.get(0),
        )
        .optional()?;
    let op = match (previous.and_then(Op::from_code), op) {
        (Some(Op::Restore), Op::Upsert) => Op::Restore,
        (_, op) => op,
    };
    tx.execute(
        "INSERT INTO sync_outbox (op_id, entity, row_id, op, hlc, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
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
