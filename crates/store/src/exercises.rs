//! User-created exercises.

use std::collections::HashMap;

use myos_core::code::Code;
use myos_core::exercise::{Exercise, ExerciseId, Modality};
use myos_core::id::Id;
use myos_core::muscle::{Muscle, MuscleRole, MuscleTarget, Share};
use rusqlite::{Transaction, params};
use uuid::Uuid;

use crate::sync::{self, Entity, Op};
use crate::{Error, Stamp, Store};

/// A user exercise with its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomExercise {
    /// What the load engine needs.
    pub exercise: Exercise,
    /// Name shown to the user.
    pub name: String,
}

const MAX_NAME: usize = 120;

fn check(name: &str, targets: &[MuscleTarget]) -> Result<(), Error> {
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(Error::OutOfRange("exercise name"));
    }
    let mut seen = std::collections::HashSet::new();
    if targets.iter().any(|target| !seen.insert(target.muscle)) {
        return Err(Error::OutOfRange("duplicate target muscle"));
    }
    Ok(())
}

fn write_targets(tx: &Transaction<'_>, id: Uuid, targets: &[MuscleTarget]) -> Result<(), Error> {
    tx.execute(
        "DELETE FROM custom_exercise_target WHERE exercise_id = ?1",
        [id.as_bytes().as_slice()],
    )?;
    for target in targets {
        tx.execute(
            "INSERT INTO custom_exercise_target (exercise_id, muscle, role, weight_bp)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                id.as_bytes().as_slice(),
                target.muscle.code(),
                target.role.code(),
                target.weight.map(Share::basis_points),
            ],
        )?;
    }
    Ok(())
}

fn touch(tx: &Transaction<'_>, id: Uuid, stamp: Stamp) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO exercise_ref (id, src) VALUES (?1, 2) ON CONFLICT (id) DO NOTHING",
        [id.as_bytes().as_slice()],
    )?;
    sync::enqueue(tx, Entity::CustomExercise, id, Op::Upsert, stamp)
}

impl Store {
    /// Create a user exercise.
    pub fn create_custom_exercise(
        &mut self,
        name: &str,
        modality: Modality,
        targets: &[MuscleTarget],
    ) -> Result<ExerciseId, Error> {
        check(name, targets)?;
        let id = Uuid::now_v7();
        self.write(|tx, stamp| {
            tx.execute(
                "INSERT INTO custom_exercise (id, modality, name, hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id.as_bytes().as_slice(),
                    modality.code(),
                    name,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            write_targets(tx, id, targets)?;
            touch(tx, id, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// Replace a live user exercise. Weekly volume is recomputed for every
    /// week that used it on the next derived refresh.
    pub fn update_custom_exercise(
        &mut self,
        exercise: ExerciseId,
        name: &str,
        modality: Modality,
        targets: &[MuscleTarget],
    ) -> Result<(), Error> {
        check(name, targets)?;
        let id = exercise.uuid();
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE custom_exercise SET name = ?2, modality = ?3, hlc = ?4, node = ?5,
                   local_seq = ?6
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![
                    id.as_bytes().as_slice(),
                    name,
                    modality.code(),
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            if changed == 0 {
                return Err(Error::NotFound("live custom exercise"));
            }
            write_targets(tx, id, targets)?;
            touch(tx, id, stamp)
        })
    }

    /// Tombstone a user exercise; logged sets keep pointing at it.
    pub fn delete_custom_exercise(&mut self, exercise: ExerciseId) -> Result<(), Error> {
        let id = exercise.uuid();
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE custom_exercise SET deleted_at = ?2, hlc = ?3, node = ?4, local_seq = ?5
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![
                    id.as_bytes().as_slice(),
                    stamp.now_millis,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            if changed == 0 {
                return Err(Error::NotFound("live custom exercise"));
            }
            sync::enqueue(tx, Entity::CustomExercise, id, Op::Delete, stamp)
        })
    }

    /// User exercises, deleted ones included (`include_deleted`) so history
    /// that used them still resolves. Rows with unknown codes are skipped.
    pub fn custom_exercises(&self, include_deleted: bool) -> Result<Vec<CustomExercise>, Error> {
        let mut targets: HashMap<[u8; 16], Option<Vec<MuscleTarget>>> = HashMap::new();
        let mut statement = self.conn().prepare_cached(
            "SELECT exercise_id, muscle, role, weight_bp FROM custom_exercise_target
             ORDER BY exercise_id, muscle",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: [u8; 16] = row.get(0)?;
            let code = |at| -> Result<Option<u16>, Error> {
                Ok(u16::try_from(row.get::<_, i64>(at)?).ok())
            };
            // `Some(None)`: role default; `None`: a weight this build rejects.
            let weight = row.get::<_, Option<i64>>(3)?.map_or(Some(None), |raw| {
                u16::try_from(raw)
                    .ok()
                    .and_then(Share::from_basis_points)
                    .map(Some)
            });
            let target = match (
                code(1)?.and_then(Muscle::from_code),
                code(2)?.and_then(MuscleRole::from_code),
                weight,
            ) {
                (Some(muscle), Some(role), Some(weight)) => Some(MuscleTarget {
                    muscle,
                    role,
                    weight,
                }),
                _ => None,
            };
            let slot = targets.entry(id).or_insert_with(|| Some(Vec::new()));
            match (slot.as_mut(), target) {
                (Some(list), Some(target)) => list.push(target),
                _ => *slot = None,
            }
        }

        let mut statement = self.conn().prepare_cached(
            "SELECT id, modality, name FROM custom_exercise
             WHERE ?1 OR deleted_at IS NULL ORDER BY name COLLATE NOCASE, id",
        )?;
        let mut rows = statement.query([include_deleted])?;
        let mut exercises = Vec::new();
        while let Some(row) = rows.next()? {
            let id: [u8; 16] = row.get(0)?;
            let modality = u16::try_from(row.get::<_, i64>(1)?)
                .ok()
                .and_then(Modality::from_code);
            let exercise_targets = targets.remove(&id).unwrap_or_else(|| Some(Vec::new()));
            if let (Some(modality), Some(targets)) = (modality, exercise_targets) {
                exercises.push(CustomExercise {
                    exercise: Exercise {
                        id: Id::from(Uuid::from_bytes(id)),
                        modality,
                        targets,
                    },
                    name: row.get(2)?,
                });
            }
        }
        Ok(exercises)
    }
}
