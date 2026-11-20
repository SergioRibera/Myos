//! `derived.db`: caches recomputed from `user.db`, never synced.
//!
//! The file is attached with the same key (explicit `KEY`, its own salt). A
//! cache that cannot be opened or has another schema is dropped and rebuilt:
//! nothing in it is a source of truth. Refresh is incremental on
//! `user.db`'s `local_seq` watermark and fully rebuilds when the load model
//! or the catalog version changes.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use jiff::civil::Date;
use myos_core::code::Code;
use myos_core::exercise::{Exercise, ExerciseId, ExerciseLookup};
use myos_core::id::Id;
use myos_core::load::{LoadModel, SetEquivalents, Skipped};
use myos_core::muscle::Muscle;
use myos_core::strength::{OneRepMax, OneRepMaxFormula};
use myos_core::volume::{
    DaySpan, MuscleVolume, ReferenceLookup, SideVolume, SkipReason, VolumeReport,
};
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use uuid::Uuid;

use crate::convert::{day_from_number, day_number};
use crate::{DatabaseKey, Error, Store};

/// Bump when the cache layout changes: the cache is then rebuilt.
const SCHEMA: i64 = 1;

const SCHEMA_SQL: &str = "
CREATE TABLE derived.derived_meta (
  key   TEXT PRIMARY KEY,
  value INTEGER NOT NULL
) STRICT;
CREATE TABLE derived.e1rm (
  exercise_id BLOB NOT NULL,
  day         INTEGER NOT NULL,
  formula     INTEGER NOT NULL,
  value_g     INTEGER NOT NULL,
  session_id  BLOB NOT NULL,      -- session that produced the best estimate
  flags       INTEGER NOT NULL,
  PRIMARY KEY (exercise_id, day, formula)
) STRICT, WITHOUT ROWID;
CREATE INDEX derived.e1rm_day ON e1rm (day);
CREATE TABLE derived.week_volume (
  week_day    INTEGER NOT NULL,
  model       INTEGER NOT NULL,
  muscle      INTEGER NOT NULL,
  left_milli  INTEGER NOT NULL,
  right_milli INTEGER NOT NULL,
  PRIMARY KEY (week_day, model, muscle)
) STRICT, WITHOUT ROWID;
CREATE TABLE derived.week_summary (
  week_day          INTEGER NOT NULL,
  model             INTEGER NOT NULL,
  counted           INTEGER NOT NULL,
  effort_assumed    INTEGER NOT NULL,
  intensity_unknown INTEGER NOT NULL,
  skipped_warmup    INTEGER NOT NULL,
  skipped_cardio    INTEGER NOT NULL,
  skipped_swimming  INTEGER NOT NULL,
  unknown_exercise  INTEGER NOT NULL,
  PRIMARY KEY (week_day, model)
) STRICT, WITHOUT ROWID;
";

const TABLES: [&str; 4] = ["week_summary", "week_volume", "e1rm", "derived_meta"];

/// 1RM formula behind cached estimates; another one rebuilds the cache.
const FORMULA: OneRepMaxFormula = OneRepMaxFormula::EpleyRirV1;

/// `e1rm.flags` bit: no effort rating, RIR 0 assumed.
const FLAG_EFFORT_ASSUMED: i64 = 1;

/// What a refresh did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefreshStats {
    /// Everything was recomputed (first run, model or catalog change).
    pub full: bool,
    /// Days whose 1RM estimates were recomputed.
    pub days: u32,
    /// Weeks whose volume was recomputed.
    pub weeks: u32,
}

/// Attach (creating or rebuilding) the derived cache.
pub(crate) fn attach(conn: &Connection, path: &Path, key: &DatabaseKey) -> Result<(), Error> {
    let location = path.to_str().ok_or(Error::OutOfRange("derived path"))?;
    if attach_once(conn, location, key)? {
        return Ok(());
    }
    // Unreadable (lost key, corruption): the cache is disposable.
    if location != ":memory:" {
        for suffix in ["", "-wal", "-shm"] {
            match std::fs::remove_file(format!("{location}{suffix}")) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::OutOfRange("derived cache file")),
            }
        }
    }
    if attach_once(conn, location, key)? {
        Ok(())
    } else {
        Err(Error::WrongKey)
    }
}

/// Attach and prepare the schema; `false` when the file cannot be read with
/// `key`.
fn attach_once(conn: &Connection, location: &str, key: &DatabaseKey) -> Result<bool, Error> {
    // The key literal is hex only; the path is bound as a parameter.
    let sql = zeroize::Zeroizing::new(format!(
        "ATTACH DATABASE ?1 AS derived KEY \"{}\"",
        key.raw_literal().as_str()
    ));
    match conn.execute(&sql, [location]) {
        Ok(_) => {}
        // Garbage in the file can fail the attach itself.
        Err(err) if err.sqlite_error_code() == Some(ErrorCode::NotADatabase) => return Ok(false),
        Err(err) => return Err(err.into()),
    }
    let tables = conn.query_row("SELECT count(*) FROM derived.sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    });
    let tables = match tables {
        Ok(tables) => tables,
        Err(err) if err.sqlite_error_code() == Some(ErrorCode::NotADatabase) => {
            conn.execute_batch("DETACH DATABASE derived")?;
            return Ok(false);
        }
        Err(err) => return Err(err.into()),
    };
    if location != ":memory:" {
        conn.query_row("PRAGMA derived.journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })?;
    }
    let schema: Option<i64> = if tables == 0 {
        None
    } else {
        conn.query_row(
            "SELECT value FROM derived.derived_meta WHERE key = 'schema'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap_or(None)
    };
    if schema != Some(SCHEMA) {
        for table in TABLES {
            conn.execute_batch(&format!("DROP TABLE IF EXISTS derived.{table}"))?;
        }
        conn.execute_batch(SCHEMA_SQL)?;
        conn.execute(
            "INSERT INTO derived.derived_meta (key, value) VALUES ('schema', ?1)",
            [SCHEMA],
        )?;
    }
    Ok(true)
}

/// Catalog first, then the user's exercises (deleted ones too: history still
/// points at them).
struct Layered<'a, C: ?Sized> {
    catalog: &'a C,
    custom: HashMap<ExerciseId, Exercise>,
}

impl<C: ExerciseLookup + ?Sized> ExerciseLookup for Layered<'_, C> {
    fn exercise(&self, id: ExerciseId) -> Option<&Exercise> {
        self.custom.get(&id).or_else(|| self.catalog.exercise(id))
    }
}

/// Best estimate per exercise seen so far.
#[derive(Default)]
struct RunningBest(HashMap<ExerciseId, OneRepMax>);

impl RunningBest {
    fn offer(&mut self, exercise: ExerciseId, mut estimate: OneRepMax) {
        // Same precision as cached estimates: incremental and full refreshes
        // agree at intensity thresholds.
        estimate.kg = kilograms(grams(estimate.kg));
        self.0
            .entry(exercise)
            .and_modify(|best| {
                if estimate.kg > best.kg {
                    *best = estimate;
                }
            })
            .or_insert(estimate);
    }
}

impl ReferenceLookup for RunningBest {
    fn reference(&self, exercise: ExerciseId) -> Option<OneRepMax> {
        self.0.get(&exercise).copied()
    }
}

/// One cached estimate: `(exercise, day, estimate, session)`.
type DayEstimate = (ExerciseId, i64, OneRepMax, Uuid);

fn grams(kg: f64) -> i64 {
    // Estimates stay far below i64::MAX grams.
    #[expect(clippy::cast_possible_truncation, reason = "bounded domain values")]
    let grams = (kg * 1_000.0).round() as i64;
    grams
}

fn kilograms(grams: i64) -> f64 {
    #[expect(clippy::cast_precision_loss, reason = "bounded domain values")]
    let kg = grams as f64 / 1_000.0;
    kg
}

fn meta(conn: &Connection, key: &str) -> Result<Option<i64>, Error> {
    Ok(conn
        .query_row(
            "SELECT value FROM derived.derived_meta WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}

impl Store {
    /// Bring the derived cache up to date with `user.db`. `catalog` resolves
    /// catalog exercises (the user's own come from the store) and
    /// `catalog_version` identifies it: a different version rebuilds
    /// everything.
    pub fn refresh_derived(
        &mut self,
        catalog: &(impl ExerciseLookup + ?Sized),
        catalog_version: u32,
    ) -> Result<RefreshStats, Error> {
        let model = LoadModel::default();
        let conn = self.conn();
        let current: i64 =
            conn.query_row("SELECT local_seq FROM sync_state WHERE k = 1", [], |row| {
                row.get(0)
            })?;
        let watermark = meta(conn, "watermark")?;
        let full = watermark.is_none()
            || meta(conn, "model")? != Some(i64::from(model.code()))
            || meta(conn, "catalog")? != Some(i64::from(catalog_version))
            || meta(conn, "formula")? != Some(i64::from(FORMULA.code()));
        let since = if full { -1 } else { watermark.unwrap_or(-1) };
        if !full && since >= current {
            return Ok(RefreshStats::default());
        }

        let days = self.touched_days(since)?;
        let layered = Layered {
            catalog,
            custom: self
                .custom_exercises(true)?
                .into_iter()
                .map(|custom| (custom.exercise.id, custom.exercise))
                .collect(),
        };
        let estimates = self.day_estimates(&days)?;
        let weeks = match days.first() {
            Some(&earliest) => {
                self.recompute_weeks(earliest, &days, &estimates, &layered, model)?
            }
            None => Vec::new(),
        };
        let stats = RefreshStats {
            full,
            days: u32::try_from(days.len()).unwrap_or(u32::MAX),
            weeks: u32::try_from(weeks.len()).unwrap_or(u32::MAX),
        };
        let meta = [
            ("watermark", current),
            ("model", i64::from(model.code())),
            ("catalog", i64::from(catalog_version)),
            ("formula", i64::from(FORMULA.code())),
        ];
        self.write_cache(full, &days, &estimates, &weeks, model, &meta)?;
        Ok(stats)
    }

    /// Days touched after `since`: changed sets and sessions, and every day
    /// using a user exercise that changed.
    fn touched_days(&self, since: i64) -> Result<BTreeSet<i64>, Error> {
        let mut days = BTreeSet::new();
        for sql in [
            "SELECT day FROM set_entry WHERE local_seq > ?1",
            "SELECT day FROM workout_session WHERE local_seq > ?1",
            "SELECT s.day FROM set_entry s JOIN custom_exercise c ON c.id = s.exercise_id
             WHERE c.local_seq > ?1 AND s.deleted_at IS NULL",
            // Days a synced row moved away from.
            "SELECT day FROM derived_touched_day WHERE ?1 = ?1",
        ] {
            let mut statement = self.conn().prepare_cached(sql)?;
            let rows = statement.query_map([since], |row| row.get::<_, i64>(0))?;
            for day in rows {
                days.insert(day?);
            }
        }
        Ok(days)
    }

    /// Best 1RM estimate per exercise on each of `days`.
    fn day_estimates(&self, days: &BTreeSet<i64>) -> Result<Vec<DayEstimate>, Error> {
        let mut estimates = Vec::new();
        for &day in days {
            let date = day_from_number(day)?;
            let read =
                self.sessions_in(DaySpan::new(date, date).ok_or(Error::OutOfRange("day"))?)?;
            let mut best: BTreeMap<ExerciseId, (OneRepMax, Uuid)> = BTreeMap::new();
            for session in &read.sessions {
                for set in &session.sets {
                    if let Some(estimate) = OneRepMax::estimate(set) {
                        let entry = best
                            .entry(set.exercise)
                            .or_insert_with(|| (estimate, session.id.uuid()));
                        if estimate.kg > entry.0.kg {
                            *entry = (estimate, session.id.uuid());
                        }
                    }
                }
            }
            estimates.extend(
                best.into_iter()
                    .map(|(exercise, (estimate, session))| (exercise, day, estimate, session)),
            );
        }
        Ok(estimates)
    }

    /// Best estimates logged before `first_week_day`, from the cache except
    /// days being recomputed, plus the fresh estimates of those days.
    fn references_before(
        &self,
        first_week_day: i64,
        days: &BTreeSet<i64>,
        estimates: &[DayEstimate],
    ) -> Result<RunningBest, Error> {
        let mut running = RunningBest::default();
        let mut statement = self.conn().prepare_cached(
            "SELECT exercise_id, day, formula, value_g, flags FROM derived.e1rm WHERE day < ?1",
        )?;
        let mut rows = statement.query([first_week_day])?;
        while let Some(row) = rows.next()? {
            if days.contains(&row.get(1)?) {
                continue;
            }
            if let Some(formula) = u16::try_from(row.get::<_, i64>(2)?)
                .ok()
                .and_then(OneRepMaxFormula::from_code)
            {
                running.offer(
                    Id::from(Uuid::from_bytes(row.get(0)?)),
                    OneRepMax {
                        kg: kilograms(row.get(3)?),
                        formula,
                        effort_assumed: row.get::<_, i64>(4)? & FLAG_EFFORT_ASSUMED != 0,
                    },
                );
            }
        }
        for (exercise, day, estimate, _) in estimates {
            if *day < first_week_day {
                running.offer(*exercise, *estimate);
            }
        }
        Ok(running)
    }

    /// Volume of every week from the one holding `earliest` to the last
    /// logged day: a changed estimate shifts the intensity reference of every
    /// later week.
    fn recompute_weeks(
        &self,
        earliest: i64,
        days: &BTreeSet<i64>,
        estimates: &[DayEstimate],
        exercises: &(impl ExerciseLookup + ?Sized),
        model: LoadModel,
    ) -> Result<Vec<(DaySpan, VolumeReport)>, Error> {
        let last: Option<i64> = self.conn().query_row(
            "SELECT max(day) FROM workout_session WHERE deleted_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        let last = last.unwrap_or(earliest).max(earliest);
        let mut week = DaySpan::iso_week(day_from_number(earliest)?);
        let mut running = self.references_before(day_number(week.first())?, days, estimates)?;
        let mut weeks = Vec::new();
        loop {
            let read = self.sessions_in(week)?;
            let report = VolumeReport::compute(week, model, &read.sessions, exercises, &running);
            // This week's estimates count from the next week on.
            for session in &read.sessions {
                for set in &session.sets {
                    if let Some(estimate) = OneRepMax::estimate(set) {
                        running.offer(set.exercise, estimate);
                    }
                }
            }
            weeks.push((week, report));
            let next = week
                .last()
                .tomorrow()
                .map_err(|_| Error::OutOfRange("week"))?;
            if day_number(next)? > last {
                return Ok(weeks);
            }
            week = DaySpan::iso_week(next);
        }
    }

    /// Replace the recomputed rows in one transaction on `derived.db`.
    fn write_cache(
        &mut self,
        full: bool,
        days: &BTreeSet<i64>,
        estimates: &[DayEstimate],
        weeks: &[(DaySpan, VolumeReport)],
        model: LoadModel,
        meta: &[(&str, i64)],
    ) -> Result<(), Error> {
        let tx = self.conn_mut().transaction()?;
        if full {
            tx.execute_batch(
                "DELETE FROM derived.e1rm; DELETE FROM derived.week_volume;
                 DELETE FROM derived.week_summary;",
            )?;
        }
        tx.execute("DELETE FROM main.derived_touched_day", [])?;
        let mut delete_day = tx.prepare_cached("DELETE FROM derived.e1rm WHERE day = ?1")?;
        for &day in days {
            delete_day.execute([day])?;
        }
        drop(delete_day);
        for (exercise, day, estimate, session) in estimates {
            tx.execute(
                "INSERT INTO derived.e1rm (exercise_id, day, formula, value_g, session_id, flags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    exercise.uuid().as_bytes().as_slice(),
                    day,
                    estimate.formula.code(),
                    grams(estimate.kg),
                    session.as_bytes().as_slice(),
                    if estimate.effort_assumed {
                        FLAG_EFFORT_ASSUMED
                    } else {
                        0
                    },
                ],
            )?;
        }
        if let Some((first, _)) = weeks.first() {
            let from = day_number(first.first())?;
            tx.execute(
                "DELETE FROM derived.week_volume WHERE week_day >= ?1",
                [from],
            )?;
            tx.execute(
                "DELETE FROM derived.week_summary WHERE week_day >= ?1",
                [from],
            )?;
        }
        for (week, report) in weeks {
            let week_day = day_number(week.first())?;
            for (muscle, volume) in report.volume.iter() {
                let (left, right) = (volume.left.to_milli(), volume.right.to_milli());
                if left == 0 && right == 0 {
                    continue;
                }
                tx.execute(
                    "INSERT INTO derived.week_volume
                       (week_day, model, muscle, left_milli, right_milli)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![week_day, model.code(), muscle.code(), left, right],
                )?;
            }
            let skipped = |reason| report.skipped.get(&reason).copied().unwrap_or(0);
            tx.execute(
                "INSERT INTO derived.week_summary
                   (week_day, model, counted, effort_assumed, intensity_unknown,
                    skipped_warmup, skipped_cardio, skipped_swimming, unknown_exercise)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    week_day,
                    model.code(),
                    report.counted_sets,
                    report.effort_assumed_sets,
                    report.intensity_unknown_sets,
                    skipped(SkipReason::Load(Skipped::Warmup)),
                    skipped(SkipReason::Load(Skipped::Cardio)),
                    skipped(SkipReason::Load(Skipped::Swimming)),
                    skipped(SkipReason::UnknownExercise),
                ],
            )?;
        }
        for (key, value) in meta {
            tx.execute(
                "INSERT INTO derived.derived_meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Cached volume of the ISO week containing `day`; `None` when nothing
    /// was computed for it (no training logged up to then, or not refreshed).
    pub fn cached_week(&self, day: Date) -> Result<Option<VolumeReport>, Error> {
        let model = LoadModel::default();
        let span = DaySpan::iso_week(day);
        let week_day = day_number(span.first())?;
        let summary = self
            .conn()
            .query_row(
                "SELECT counted, effort_assumed, intensity_unknown, skipped_warmup,
                        skipped_cardio, skipped_swimming, unknown_exercise
                 FROM derived.week_summary WHERE week_day = ?1 AND model = ?2",
                params![week_day, model.code()],
                |row| {
                    let mut counts = [0_u32; 7];
                    for (at, count) in counts.iter_mut().enumerate() {
                        *count = row.get(at)?;
                    }
                    Ok(counts)
                },
            )
            .optional()?;
        let Some(counts) = summary else {
            return Ok(None);
        };
        let mut volume = MuscleVolume::default();
        let mut statement = self.conn().prepare_cached(
            "SELECT muscle, left_milli, right_milli FROM derived.week_volume
             WHERE week_day = ?1 AND model = ?2",
        )?;
        let mut rows = statement.query(params![week_day, model.code()])?;
        while let Some(row) = rows.next()? {
            if let Some(muscle) = u16::try_from(row.get::<_, i64>(0)?)
                .ok()
                .and_then(Muscle::from_code)
            {
                volume.set(
                    muscle,
                    SideVolume {
                        left: SetEquivalents::from_milli(row.get(1)?),
                        right: SetEquivalents::from_milli(row.get(2)?),
                    },
                );
            }
        }
        let skipped = [
            (SkipReason::Load(Skipped::Warmup), counts[3]),
            (SkipReason::Load(Skipped::Cardio), counts[4]),
            (SkipReason::Load(Skipped::Swimming), counts[5]),
            (SkipReason::UnknownExercise, counts[6]),
        ]
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .collect();
        Ok(Some(VolumeReport {
            span,
            model,
            volume,
            counted_sets: counts[0],
            effort_assumed_sets: counts[1],
            intensity_unknown_sets: counts[2],
            skipped,
        }))
    }

    /// Best cached 1RM estimate of `exercise` over all history.
    pub fn best_one_rep_max(&self, exercise: ExerciseId) -> Result<Option<OneRepMax>, Error> {
        let row: Option<(i64, i64, i64)> = self
            .conn()
            .query_row(
                "SELECT formula, value_g, flags FROM derived.e1rm WHERE exercise_id = ?1
                 ORDER BY value_g DESC LIMIT 1",
                [exercise.uuid().as_bytes().as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(formula, value, flags)| {
            Some(OneRepMax {
                kg: kilograms(value),
                formula: u16::try_from(formula)
                    .ok()
                    .and_then(OneRepMaxFormula::from_code)?,
                effort_assumed: flags & FLAG_EFFORT_ASSUMED != 0,
            })
        }))
    }
}
