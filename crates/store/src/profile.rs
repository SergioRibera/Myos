//! Profile, preferences, goals and body measurements.

use jiff::Timestamp;
use jiff::civil::Date;
use myos_core::body::{
    BodyMeasurement, MeasurementId, MeasurementKind, MeasurementMethod, MeasurementSource,
};
use myos_core::code::Code;
use myos_core::id::Id;
use myos_core::profile::{Goal, GoalPeriod, Level, Preferences, Profile, Sex};
use myos_core::session::Side;
use myos_core::units::Length;
use myos_core::volume::DaySpan;
use rusqlite::{OptionalExtension, Row, Transaction, params};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::convert::{day_from_number, day_number};
use crate::sync::{self, Entity, Op};
use crate::{Error, Store, ids};

/// Body measurements read for a span of days.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeasurementsRead {
    /// Live measurements, oldest first.
    pub measurements: Vec<(MeasurementId, BodyMeasurement)>,
    /// Rows with codes this build does not know.
    pub undecodable: u32,
}

/// Longest note, in characters.
const MAX_NOTE: usize = 500;
/// Longest locale tag, in bytes.
const MAX_LOCALE: usize = 35;

fn user_key(tx: &Transaction<'_>) -> Result<Zeroizing<[u8; 32]>, Error> {
    let key: [u8; 32] = tx.query_row("SELECT user_key FROM sync_state WHERE k = 1", [], |row| {
        row.get(0)
    })?;
    Ok(Zeroizing::new(key))
}

/// Op for an explicit local write to a natural-key row: a tombstoned row is
/// restored on purpose (deletes otherwise win on merge). `table` is one of
/// this module's literals, never user input.
fn upsert_op(tx: &Transaction<'_>, table: &'static str, id: Uuid) -> Result<Op, Error> {
    let deleted: Option<bool> = tx
        .query_row(
            &format!("SELECT deleted_at IS NOT NULL FROM {table} WHERE id = ?1"),
            [id.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(if deleted == Some(true) {
        Op::Restore
    } else {
        Op::Upsert
    })
}

/// A stored code; unknown or unspecified (0) reads as `None`.
fn decode_code<T: Code>(raw: Option<i64>) -> Option<T> {
    raw.and_then(|raw| u16::try_from(raw).ok())
        .and_then(T::from_code)
}

fn decode_measurement(row: &Row<'_>) -> Result<Option<(MeasurementId, BodyMeasurement)>, Error> {
    let side = match row.get::<_, i64>(2)? {
        0 => None,
        raw => match decode_code::<Side>(Some(raw)) {
            Some(side) => Some(side),
            None => return Ok(None),
        },
    };
    // 0 is the column default for "not stated".
    let method = match row.get::<_, i64>(3)? {
        0 => Some(MeasurementMethod::Unspecified),
        raw => decode_code::<MeasurementMethod>(Some(raw)),
    };
    let (Some(kind), Some(method), Some(source)) = (
        decode_code::<MeasurementKind>(row.get(1)?),
        method,
        decode_code::<MeasurementSource>(row.get(7)?),
    ) else {
        return Ok(None);
    };
    let (Ok(measured_at), Ok(day)) = (
        Timestamp::from_millisecond(row.get(5)?),
        day_from_number(row.get(6)?),
    ) else {
        return Ok(None);
    };
    let measurement = BodyMeasurement {
        kind,
        side,
        method,
        value: row.get(4)?,
        measured_at,
        day,
        source,
        note: row.get(8)?,
    };
    // Rows synced from elsewhere get the same checks as local input.
    if measurement.validate().is_err() {
        return Ok(None);
    }
    Ok(Some((Id::from(Uuid::from_bytes(row.get(0)?)), measurement)))
}

impl Store {
    /// The profile; empty until the user fills it.
    pub fn profile(&self) -> Result<Profile, Error> {
        let read: Option<(Option<i16>, Option<i64>, Option<u32>)> = self
            .conn()
            .query_row(
                "SELECT birth_year, sex, height_mm FROM profile
                 WHERE id = ?1 AND deleted_at IS NULL",
                [ids::PROFILE.as_bytes().as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(
            read.map_or_else(Profile::default, |(birth_year, sex, height)| Profile {
                birth_year,
                sex: decode_code::<Sex>(sex),
                height: height.map(Length::from_millimeters),
            }),
        )
    }

    /// Replace the profile. Rejects users younger than the minimum age on
    /// `today`.
    pub fn set_profile(&mut self, profile: &Profile, today: Date) -> Result<(), Error> {
        profile.validate(today)?;
        if let Some(height) = profile.height
            && !(500..=2_800).contains(&height.millimeters())
        {
            return Err(Error::OutOfRange("height"));
        }
        let id = ids::PROFILE;
        self.write(|tx, stamp| {
            let op = upsert_op(tx, "profile", id)?;
            tx.execute(
                "INSERT INTO profile (id, birth_year, sex, height_mm, hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (id) DO UPDATE SET
                   birth_year = excluded.birth_year, sex = excluded.sex,
                   height_mm = excluded.height_mm, hlc = excluded.hlc,
                   node = excluded.node, local_seq = excluded.local_seq,
                   deleted_at = NULL, field_hlc = NULL",
                params![
                    id.as_bytes().as_slice(),
                    profile.birth_year,
                    profile.sex.map(Sex::code),
                    profile.height.map(Length::millimeters),
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::Profile, id, op, stamp)
        })
    }

    /// App preferences; defaults until changed. Unknown or unspecified codes
    /// read as their default.
    pub fn preferences(&self) -> Result<Preferences, Error> {
        type Codes = [Option<i64>; 6];
        let read: Option<(Codes, Option<String>)> = self
            .conn()
            .query_row(
                "SELECT theme, unit_mass, unit_distance, unit_energy, effort_scale, diet_mode,
                        locale
                 FROM preferences WHERE id = ?1 AND deleted_at IS NULL",
                [ids::PREFERENCES.as_bytes().as_slice()],
                |row| {
                    let mut codes: Codes = [None; 6];
                    for (at, code) in codes.iter_mut().enumerate() {
                        *code = row.get(at)?;
                    }
                    Ok((codes, row.get(6)?))
                },
            )
            .optional()?;
        Ok(
            read.map_or_else(Preferences::default, |(codes, locale)| Preferences {
                theme: decode_code(codes[0]).unwrap_or_default(),
                mass_unit: decode_code(codes[1]).unwrap_or_default(),
                distance_unit: decode_code(codes[2]).unwrap_or_default(),
                energy_unit: decode_code(codes[3]).unwrap_or_default(),
                effort_scale: decode_code(codes[4]).unwrap_or_default(),
                diet_mode: decode_code(codes[5]).unwrap_or_default(),
                locale,
            }),
        )
    }

    /// Replace the preferences.
    pub fn set_preferences(&mut self, preferences: &Preferences) -> Result<(), Error> {
        if preferences
            .locale
            .as_ref()
            .is_some_and(|locale| locale.len() > MAX_LOCALE)
        {
            return Err(Error::OutOfRange("locale"));
        }
        let id = ids::PREFERENCES;
        self.write(|tx, stamp| {
            let op = upsert_op(tx, "preferences", id)?;
            tx.execute(
                "INSERT INTO preferences
                   (id, theme, unit_mass, unit_distance, unit_energy, effort_scale, diet_mode,
                    locale, hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (id) DO UPDATE SET
                   theme = excluded.theme, unit_mass = excluded.unit_mass,
                   unit_distance = excluded.unit_distance, unit_energy = excluded.unit_energy,
                   effort_scale = excluded.effort_scale, diet_mode = excluded.diet_mode,
                   locale = excluded.locale, hlc = excluded.hlc, node = excluded.node,
                   local_seq = excluded.local_seq, deleted_at = NULL, field_hlc = NULL",
                params![
                    id.as_bytes().as_slice(),
                    preferences.theme.code(),
                    preferences.mass_unit.code(),
                    preferences.distance_unit.code(),
                    preferences.energy_unit.code(),
                    preferences.effort_scale.code(),
                    preferences.diet_mode.code(),
                    preferences.locale,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::Preferences, id, op, stamp)
        })
    }

    /// Set the goal and level from `period.from` on. One row per start day:
    /// setting the same day again replaces it, on every device.
    pub fn set_goal(&mut self, period: GoalPeriod) -> Result<(), Error> {
        let from = day_number(period.from)?;
        self.write(|tx, stamp| {
            let key = user_key(tx)?;
            let id = ids::natural(&key, Entity::ProfileGoal, &[&from.to_be_bytes()]);
            let op = upsert_op(tx, "profile_goal", id)?;
            tx.execute(
                "INSERT INTO profile_goal (id, from_day, goal, level, hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (id) DO UPDATE SET
                   goal = excluded.goal, level = excluded.level,
                   hlc = excluded.hlc, node = excluded.node, local_seq = excluded.local_seq,
                   deleted_at = NULL",
                params![
                    id.as_bytes().as_slice(),
                    from,
                    period.goal.code(),
                    period.level.code(),
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::ProfileGoal, id, op, stamp)
        })
    }

    /// Goal and level in force on `day`.
    pub fn goal_on(&self, day: Date) -> Result<Option<GoalPeriod>, Error> {
        let day = day_number(day)?;
        let mut statement = self.conn().prepare_cached(
            "SELECT from_day, goal, level FROM profile_goal
             WHERE deleted_at IS NULL AND from_day <= ?1
             ORDER BY from_day DESC, hlc DESC, node DESC
             LIMIT 1",
        )?;
        let mut rows = statement.query([day])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        // A period written by a newer version reads as unknown rather than
        // falling back to an older, no longer valid goal.
        let (Some(goal), Some(level)) = (
            decode_code::<Goal>(row.get(1)?),
            decode_code::<Level>(row.get(2)?),
        ) else {
            return Ok(None);
        };
        Ok(Some(GoalPeriod {
            from: day_from_number(row.get(0)?)?,
            goal,
            level,
        }))
    }

    /// Record a body measurement taken on a clock `tz_offset_min` minutes
    /// from UTC.
    pub fn record_measurement(
        &mut self,
        measurement: &BodyMeasurement,
        tz_offset_min: i16,
    ) -> Result<MeasurementId, Error> {
        measurement.validate()?;
        if !(-840..=840).contains(&tz_offset_min) {
            return Err(Error::OutOfRange("tz_offset_min"));
        }
        if measurement
            .note
            .as_ref()
            .is_some_and(|note| note.chars().count() > MAX_NOTE)
        {
            return Err(Error::OutOfRange("note"));
        }
        let day = day_number(measurement.day)?;
        let id = Uuid::now_v7();
        self.write(|tx, stamp| {
            tx.execute(
                "INSERT INTO body_measurement
                   (id, measured_at, day, tz_offset_min, kind, side, method, value, source,
                    note, hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    id.as_bytes().as_slice(),
                    measurement.measured_at.as_millisecond(),
                    day,
                    tz_offset_min,
                    measurement.kind.code(),
                    measurement.side.map_or(0, Side::code),
                    measurement.method.code(),
                    measurement.value,
                    measurement.source.code(),
                    measurement.note,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::BodyMeasurement, id, Op::Upsert, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// Live measurements of `kind` taken on a day in `span`, oldest first.
    pub fn measurements(
        &self,
        kind: MeasurementKind,
        span: DaySpan,
    ) -> Result<MeasurementsRead, Error> {
        let mut statement = self.conn().prepare_cached(
            "SELECT id, kind, side, method, value, measured_at, day, source, note
             FROM body_measurement
             WHERE deleted_at IS NULL AND kind = ?1 AND day BETWEEN ?2 AND ?3
             ORDER BY measured_at, id",
        )?;
        let mut rows = statement.query(params![
            kind.code(),
            day_number(span.first())?,
            day_number(span.last())?
        ])?;
        let mut read = MeasurementsRead::default();
        while let Some(row) = rows.next()? {
            match decode_measurement(row)? {
                Some(measurement) => read.measurements.push(measurement),
                None => read.undecodable += 1,
            }
        }
        Ok(read)
    }

    /// Tombstone a measurement.
    pub fn delete_measurement(&mut self, measurement: MeasurementId) -> Result<(), Error> {
        let id = measurement.uuid();
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE body_measurement SET deleted_at = ?2, hlc = ?3, node = ?4, local_seq = ?5
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
                return Err(Error::NotFound("live measurement"));
            }
            sync::enqueue(tx, Entity::BodyMeasurement, id, Op::Delete, stamp)
        })
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use myos_core::profile::{Goal, GoalPeriod, Level, Profile};

    use super::*;
    use crate::DatabaseKey;

    fn store() -> Result<Store, Error> {
        Store::open_in_memory(&DatabaseKey::generate()?)
    }

    #[test]
    fn writing_a_tombstoned_profile_restores_it() -> Result<(), Error> {
        let mut store = store()?;
        let profile = Profile {
            birth_year: Some(1990),
            ..Profile::default()
        };
        store.set_profile(&profile, date(2026, 11, 20))?;
        store
            .conn
            .execute("UPDATE profile SET deleted_at = 1", [])?;
        assert_eq!(store.profile()?, Profile::default());
        store.set_profile(&profile, date(2026, 11, 20))?;
        assert_eq!(store.profile()?, profile);
        let pending = store.pending_outbox(10)?;
        assert_eq!(pending[0].op, Some(Op::Restore));
        Ok(())
    }

    #[test]
    fn newest_goal_with_unknown_codes_reads_as_unknown() -> Result<(), Error> {
        let mut store = store()?;
        store.set_goal(GoalPeriod {
            from: date(2026, 1, 1),
            goal: Goal::Health,
            level: Level::Beginner,
        })?;
        store.set_goal(GoalPeriod {
            from: date(2026, 6, 1),
            goal: Goal::Strength,
            level: Level::Beginner,
        })?;
        store.conn.execute(
            "UPDATE profile_goal SET goal = 99 WHERE from_day > 20500",
            [],
        )?;
        assert_eq!(store.goal_on(date(2026, 11, 20))?, None);
        Ok(())
    }

    #[test]
    fn measurement_rows_from_elsewhere_are_checked() -> Result<(), Error> {
        let mut store = store()?;
        let weight = BodyMeasurement {
            kind: MeasurementKind::Weight,
            side: None,
            method: MeasurementMethod::Scale,
            value: 80_000,
            measured_at: Timestamp::from_second(1_795_000_000)
                .map_err(|_| Error::OutOfRange("test"))?,
            day: date(2026, 11, 18),
            source: MeasurementSource::Manual,
            note: None,
        };
        store.record_measurement(&weight, 0)?;
        let week = DaySpan::iso_week(date(2026, 11, 18));
        store
            .conn
            .execute("UPDATE body_measurement SET method = 0", [])?;
        let read = store.measurements(MeasurementKind::Weight, week)?;
        assert_eq!(
            read.measurements[0].1.method,
            MeasurementMethod::Unspecified
        );
        store
            .conn
            .execute("UPDATE body_measurement SET value = 5", [])?;
        let read = store.measurements(MeasurementKind::Weight, week)?;
        assert_eq!((read.measurements.len(), read.undecodable), (0, 1));
        Ok(())
    }
}
