//! Routines: a head row plus immutable content versions.

use std::collections::HashSet;

use myos_core::code::Code;
use myos_core::id::Id;
use myos_core::routine::{FollowMode, RoutineContent, RoutineId, RoutineVersionId};
use rusqlite::{OptionalExtension, Transaction, params};
use uuid::Uuid;

use crate::content::{decode_routine, encode_routine};
use crate::sync::{self, Entity, Op};
use crate::{Error, Stamp, Store};

/// A routine with its current content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRoutine {
    /// Head id.
    pub id: RoutineId,
    /// Name.
    pub name: String,
    /// Relation to where it came from.
    pub follow: FollowMode,
    /// Routine it was copied from.
    pub upstream: Option<RoutineId>,
    /// Current version.
    pub version: RoutineVersionId,
    /// Version number (1 = first save).
    pub number: u32,
    /// Content of the current version.
    pub content: RoutineContent,
}

/// A routine in a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineSummary {
    /// Head id.
    pub id: RoutineId,
    /// Name.
    pub name: String,
    /// Relation to where it came from; `None` for a code this build does not
    /// know.
    pub follow: Option<FollowMode>,
}

const MAX_NAME: usize = 120;

fn check_name(name: &str) -> Result<(), Error> {
    if !name.is_empty() && name.chars().count() <= MAX_NAME {
        Ok(())
    } else {
        Err(Error::OutOfRange("routine name"))
    }
}

fn check_content(content: &RoutineContent) -> Result<(), Error> {
    let mut seen = HashSet::new();
    for item in content.items() {
        if !seen.insert(item.id) {
            return Err(Error::OutOfRange("duplicate routine item id"));
        }
        if item.sets == 0 {
            return Err(Error::OutOfRange("routine item sets"));
        }
        if let Some((min, max)) = item.reps
            && min > max
        {
            return Err(Error::OutOfRange("routine item reps"));
        }
    }
    for day in &content.days {
        if day.name.chars().count() > MAX_NAME {
            return Err(Error::OutOfRange("routine day name"));
        }
    }
    Ok(())
}

/// Insert version `number` of `routine` and return its id.
fn insert_version(
    tx: &Transaction<'_>,
    routine: Uuid,
    number: u32,
    content: &[u8],
    stamp: Stamp,
) -> Result<Uuid, Error> {
    let id = Uuid::now_v7();
    tx.execute(
        "INSERT INTO routine_version (id, routine_id, n, created_at, content, hlc, node, local_seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id.as_bytes().as_slice(),
            routine.as_bytes().as_slice(),
            number,
            stamp.now_millis,
            content,
            sync::sql_hlc(stamp.hlc)?,
            stamp.node,
            stamp.local_seq,
        ],
    )?;
    sync::enqueue(tx, Entity::RoutineVersion, id, Op::Upsert, stamp)?;
    Ok(id)
}

/// Insert a routine head pointing at a fresh first version.
fn insert_routine(
    tx: &Transaction<'_>,
    name: &str,
    follow: FollowMode,
    upstream: Option<(Uuid, Option<Uuid>)>,
    content: &[u8],
    stamp: Stamp,
) -> Result<Uuid, Error> {
    let id = Uuid::now_v7();
    let version = insert_version(tx, id, 1, content, stamp)?;
    tx.execute(
        "INSERT INTO routine
           (id, name, current_version_id, upstream_id, follow_mode, followed_version_id,
            hlc, node, local_seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id.as_bytes().as_slice(),
            name,
            version.as_bytes().as_slice(),
            upstream.map(|(routine, _)| *routine.as_bytes()),
            follow.code(),
            upstream
                .and_then(|(_, version)| version)
                .map(|version| *version.as_bytes()),
            sync::sql_hlc(stamp.hlc)?,
            stamp.node,
            stamp.local_seq,
        ],
    )?;
    sync::enqueue(tx, Entity::Routine, id, Op::Upsert, stamp)?;
    Ok(id)
}

impl Store {
    /// Create one of the user's own routines.
    pub fn create_routine(
        &mut self,
        name: &str,
        content: &RoutineContent,
    ) -> Result<RoutineId, Error> {
        check_name(name)?;
        check_content(content)?;
        let bytes = encode_routine(content)?;
        let id =
            self.write(|tx, stamp| insert_routine(tx, name, FollowMode::Own, None, &bytes, stamp))?;
        Ok(Id::from(id))
    }

    /// Save new content for a live routine as its next version.
    pub fn save_routine_version(
        &mut self,
        routine: RoutineId,
        content: &RoutineContent,
    ) -> Result<RoutineVersionId, Error> {
        check_content(content)?;
        let bytes = encode_routine(content)?;
        let routine = routine.uuid();
        let version = self.write(|tx, stamp| {
            let latest: Option<u32> = tx
                .query_row(
                    "SELECT (SELECT max(v.n) FROM routine_version v WHERE v.routine_id = r.id)
                     FROM routine r WHERE r.id = ?1 AND r.deleted_at IS NULL",
                    [routine.as_bytes().as_slice()],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or(Error::NotFound("live routine"))?;
            let number = latest
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(Error::OutOfRange("routine versions"))?;
            let version = insert_version(tx, routine, number, &bytes, stamp)?;
            tx.execute(
                "UPDATE routine SET current_version_id = ?2,
                   hlc = ?3, node = ?4, local_seq = ?5
                 WHERE id = ?1",
                params![
                    routine.as_bytes().as_slice(),
                    version.as_bytes().as_slice(),
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::Routine, routine, Op::Upsert, stamp)?;
            Ok(version)
        })?;
        Ok(Id::from(version))
    }

    /// Copy a live routine. `Follow` keeps the followed version so new
    /// upstream versions can be offered; `Copy` only records where it came
    /// from; `Own` is a plain duplicate.
    pub fn copy_routine(
        &mut self,
        source: RoutineId,
        follow: FollowMode,
        name: &str,
    ) -> Result<RoutineId, Error> {
        check_name(name)?;
        let source = source.uuid();
        let id = self.write(|tx, stamp| {
            let (version, content): ([u8; 16], Vec<u8>) = tx
                .query_row(
                    "SELECT v.id, v.content FROM routine r
                     JOIN routine_version v ON v.id = r.current_version_id
                     WHERE r.id = ?1 AND r.deleted_at IS NULL",
                    [source.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .ok_or(Error::NotFound("live routine"))?;
            let upstream = match follow {
                FollowMode::Own => None,
                FollowMode::Copy => Some((source, None)),
                FollowMode::Follow => Some((source, Some(Uuid::from_bytes(version)))),
            };
            insert_routine(tx, name, follow, upstream, &content, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// Rename a live routine.
    pub fn rename_routine(&mut self, routine: RoutineId, name: &str) -> Result<(), Error> {
        check_name(name)?;
        let id = routine.uuid();
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE routine SET name = ?2, hlc = ?3, node = ?4,
                   local_seq = ?5
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![
                    id.as_bytes().as_slice(),
                    name,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            if changed == 0 {
                return Err(Error::NotFound("live routine"));
            }
            sync::enqueue(tx, Entity::Routine, id, Op::Upsert, stamp)
        })
    }

    /// Tombstone a routine. Its versions stay, so logged sets keep resolving.
    pub fn delete_routine(&mut self, routine: RoutineId) -> Result<(), Error> {
        let id = routine.uuid();
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE routine SET deleted_at = ?2, hlc = ?3, node = ?4, local_seq = ?5
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
                return Err(Error::NotFound("live routine"));
            }
            sync::enqueue(tx, Entity::Routine, id, Op::Delete, stamp)
        })
    }

    /// A live routine with its current content.
    pub fn routine(&self, routine: RoutineId) -> Result<StoredRoutine, Error> {
        type Head = (String, i64, Option<[u8; 16]>, [u8; 16], u32, Vec<u8>);
        let id = routine.uuid();
        let (name, follow, upstream, version, number, content): Head = self
            .conn()
            // The head may arrive before its current version: fall back to the
            // newest version present. Version numbers can repeat across
            // devices, so order by (n, hlc, node).
            .query_row(
                "SELECT r.name, r.follow_mode, r.upstream_id, v.id, v.n, v.content
                 FROM routine r JOIN routine_version v ON v.id = coalesce(
                   (SELECT id FROM routine_version WHERE id = r.current_version_id),
                   (SELECT id FROM routine_version WHERE routine_id = r.id
                    ORDER BY n DESC, hlc DESC, node DESC LIMIT 1))
                 WHERE r.id = ?1 AND r.deleted_at IS NULL",
                [id.as_bytes().as_slice()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::NotFound("live routine"))?;
        let follow = u16::try_from(follow)
            .ok()
            .and_then(FollowMode::from_code)
            .ok_or(Error::OutOfRange("follow mode"))?;
        Ok(StoredRoutine {
            id: routine,
            name,
            follow,
            upstream: upstream.map(|bytes| Id::from(Uuid::from_bytes(bytes))),
            version: Id::from(Uuid::from_bytes(version)),
            number,
            content: decode_routine(&content)?,
        })
    }

    /// Content of any version (also of deleted routines), for sets that
    /// point at an older version.
    pub fn routine_version(&self, version: RoutineVersionId) -> Result<RoutineContent, Error> {
        let content: Vec<u8> = self
            .conn()
            .query_row(
                "SELECT content FROM routine_version WHERE id = ?1",
                [version.uuid().as_bytes().as_slice()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound("routine version"))?;
        Ok(decode_routine(&content)?)
    }

    /// Live routines, by name.
    pub fn routines(&self) -> Result<Vec<RoutineSummary>, Error> {
        self.conn()
            .prepare_cached(
                "SELECT id, name, follow_mode FROM routine WHERE deleted_at IS NULL
                 ORDER BY name COLLATE NOCASE, id",
            )?
            .query_map([], |row| {
                Ok(RoutineSummary {
                    id: Id::from(Uuid::from_bytes(row.get(0)?)),
                    name: row.get(1)?,
                    follow: u16::try_from(row.get::<_, i64>(2)?)
                        .ok()
                        .and_then(FollowMode::from_code),
                })
            })?
            .collect::<Result<_, _>>()
            .map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DatabaseKey;

    #[test]
    fn a_head_ahead_of_its_version_reads_the_newest_present() -> Result<(), Error> {
        let mut store = Store::open_in_memory(&DatabaseKey::generate()?)?;
        let content = RoutineContent::default();
        let id = store.create_routine("PPL", &content)?;
        let second = store.save_routine_version(id, &content)?;
        // The head synced with a version that has not arrived yet.
        store.conn.execute(
            "UPDATE routine SET current_version_id = ?1",
            [Uuid::now_v7().as_bytes().as_slice()],
        )?;
        let stored = store.routine(id)?;
        assert_eq!((stored.version, stored.number), (second, 2));
        Ok(())
    }
}
