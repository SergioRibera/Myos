//! Plan phases, events, calibration results and suggestions.

use std::time::Duration;

use jiff::Timestamp;
use myos_core::code::Code;
use myos_core::id::Id;
use myos_core::load::LoadModel;
use myos_core::muscle::{Muscle, Share};
use myos_core::plan::{
    Answer, CalibrationId, CalibrationMeasure, CalibrationProtocol, CalibrationResult, Event,
    EventId, EventKind, EventPriority, Multiplier, Phase, PhaseId, PhaseKind, Response,
    ResponseReason, Suggestion, SuggestionId, SuggestionKind,
};
use myos_core::session::Rir;
use myos_core::units::{Distance, Mass};
use myos_core::volume::DaySpan;
use rusqlite::{OptionalExtension, Row, params};
use uuid::Uuid;

use crate::convert::{day_from_number, day_number};
use crate::sync::{self, Entity, Op};
use crate::{Error, Store};

/// Rows read with how many could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read<T> {
    /// Decoded rows.
    pub rows: Vec<T>,
    /// Rows with values this build does not understand.
    pub undecodable: u32,
}

impl<T> Default for Read<T> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            undecodable: 0,
        }
    }
}

const MAX_NAME: usize = 120;
const MAX_INPUTS: usize = 64 * 1024;

/// Calibration value units (`calibration_result.unit`).
const UNIT_GRAMS: u8 = 1;
const UNIT_REPS: u8 = 2;
const UNIT_MILLIS: u8 = 3;
const UNIT_METERS: u8 = 4;

fn code<T: Code>(raw: i64) -> Option<T> {
    u16::try_from(raw).ok().and_then(T::from_code)
}

/// `Ok(None)` for NULL, `Err(())` for a code this build does not know.
fn optional_code<T: Code>(raw: Option<i64>) -> Result<Option<T>, ()> {
    raw.map_or(Ok(None), |raw| code::<T>(raw).map(Some).ok_or(()))
}

fn measure_columns(measure: CalibrationMeasure) -> Result<(i64, u8), Error> {
    Ok(match measure {
        CalibrationMeasure::Load(mass) => (i64::from(mass.grams()), UNIT_GRAMS),
        CalibrationMeasure::Reps(reps) => (i64::from(reps), UNIT_REPS),
        CalibrationMeasure::Time(time) => (
            i64::try_from(time.as_millis()).map_err(|_| Error::OutOfRange("calibration time"))?,
            UNIT_MILLIS,
        ),
        CalibrationMeasure::Distance(distance) => (i64::from(distance.meters()), UNIT_METERS),
    })
}

fn measure_from(value: i64, unit: i64) -> Option<CalibrationMeasure> {
    match u8::try_from(unit).ok()? {
        UNIT_GRAMS => u32::try_from(value)
            .ok()
            .map(|grams| CalibrationMeasure::Load(Mass::from_grams(grams))),
        UNIT_REPS => u16::try_from(value).ok().map(CalibrationMeasure::Reps),
        UNIT_MILLIS => u64::try_from(value)
            .ok()
            .map(|ms| CalibrationMeasure::Time(Duration::from_millis(ms))),
        UNIT_METERS => u32::try_from(value)
            .ok()
            .map(|meters| CalibrationMeasure::Distance(Distance::from_meters(meters))),
        _ => None,
    }
}

fn decode_phase(row: &Row<'_>) -> Result<Option<(PhaseId, Phase)>, Error> {
    let (Ok(starts_on), Ok(ends_on), Some(kind), Some(volume), Some(intensity)) = (
        day_from_number(row.get(1)?),
        day_from_number(row.get(2)?),
        code::<PhaseKind>(row.get(3)?),
        u16::try_from(row.get::<_, i64>(4)?)
            .ok()
            .and_then(Multiplier::from_basis_points),
        u16::try_from(row.get::<_, i64>(5)?)
            .ok()
            .and_then(Multiplier::from_basis_points),
    ) else {
        return Ok(None);
    };
    Ok(Some((
        Id::from(Uuid::from_bytes(row.get(0)?)),
        Phase {
            starts_on,
            ends_on,
            kind,
            volume,
            intensity,
        },
    )))
}

impl Store {
    /// Add a plan phase.
    pub fn add_phase(&mut self, phase: Phase) -> Result<PhaseId, Error> {
        let starts_on = day_number(phase.starts_on)?;
        let ends_on = day_number(phase.ends_on)?;
        if ends_on < starts_on {
            return Err(Error::OutOfRange("phase dates"));
        }
        let id = Uuid::now_v7();
        self.write(|tx, stamp| {
            tx.execute(
                "INSERT INTO plan_phase
                   (id, starts_on, ends_on, kind, volume_mult_bp, intensity_mult_bp,
                    hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id.as_bytes().as_slice(),
                    starts_on,
                    ends_on,
                    phase.kind.code(),
                    phase.volume.basis_points(),
                    phase.intensity.basis_points(),
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::PlanPhase, id, Op::Upsert, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// Live phases overlapping `span`, by start day.
    pub fn phases_in(&self, span: DaySpan) -> Result<Read<(PhaseId, Phase)>, Error> {
        let mut statement = self.conn().prepare_cached(
            "SELECT id, starts_on, ends_on, kind, volume_mult_bp, intensity_mult_bp
             FROM plan_phase
             WHERE deleted_at IS NULL AND starts_on <= ?2 AND ends_on >= ?1
             ORDER BY starts_on, id",
        )?;
        let mut rows = statement.query([day_number(span.first())?, day_number(span.last())?])?;
        let mut read = Read::default();
        while let Some(row) = rows.next()? {
            match decode_phase(row)? {
                Some(phase) => read.rows.push(phase),
                None => read.undecodable += 1,
            }
        }
        Ok(read)
    }

    /// Tombstone a phase.
    pub fn delete_phase(&mut self, phase: PhaseId) -> Result<(), Error> {
        self.tombstone_plan_row("plan_phase", Entity::PlanPhase, phase.uuid())
    }

    /// Add an event.
    pub fn add_event(&mut self, event: &Event) -> Result<EventId, Error> {
        if event.name.is_empty() || event.name.chars().count() > MAX_NAME {
            return Err(Error::OutOfRange("event name"));
        }
        let day = day_number(event.on)?;
        let id = Uuid::now_v7();
        self.write(|tx, stamp| {
            tx.execute(
                "INSERT INTO plan_event (id, on_day, kind, priority, name, hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id.as_bytes().as_slice(),
                    day,
                    event.kind.code(),
                    event.priority.code(),
                    event.name,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::PlanEvent, id, Op::Upsert, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// Live events on a day in `span`, by day.
    pub fn events_in(&self, span: DaySpan) -> Result<Read<(EventId, Event)>, Error> {
        let mut statement = self.conn().prepare_cached(
            "SELECT id, on_day, kind, priority, name FROM plan_event
             WHERE deleted_at IS NULL AND on_day BETWEEN ?1 AND ?2
             ORDER BY on_day, id",
        )?;
        let mut rows = statement.query([day_number(span.first())?, day_number(span.last())?])?;
        let mut read = Read::default();
        while let Some(row) = rows.next()? {
            let decoded = (
                day_from_number(row.get(1)?),
                code::<EventKind>(row.get(2)?),
                code::<EventPriority>(row.get(3)?),
            );
            let (Ok(on), Some(kind), Some(priority)) = decoded else {
                read.undecodable += 1;
                continue;
            };
            read.rows.push((
                Id::from(Uuid::from_bytes(row.get(0)?)),
                Event {
                    on,
                    kind,
                    priority,
                    name: row.get(4)?,
                },
            ));
        }
        Ok(read)
    }

    /// Tombstone an event.
    pub fn delete_event(&mut self, event: EventId) -> Result<(), Error> {
        self.tombstone_plan_row("plan_event", Entity::PlanEvent, event.uuid())
    }

    /// Record a calibration result.
    pub fn record_calibration(
        &mut self,
        result: &CalibrationResult,
    ) -> Result<CalibrationId, Error> {
        let day = day_number(result.day)?;
        let (value, unit) = measure_columns(result.measure)?;
        let id = Uuid::now_v7();
        self.write(|tx, stamp| {
            tx.execute(
                "INSERT INTO calibration_result
                   (id, set_id, exercise_id, day, protocol, kind, value, unit, rir_declared,
                    hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id.as_bytes().as_slice(),
                    result.set.map(|set| *set.uuid().as_bytes()),
                    result.exercise.uuid().as_bytes().as_slice(),
                    day,
                    result.protocol.code(),
                    value,
                    unit,
                    result.rir.map(Rir::get),
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::CalibrationResult, id, Op::Upsert, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// Live calibration results of an exercise, oldest first.
    pub fn calibrations_of(
        &self,
        exercise: myos_core::exercise::ExerciseId,
    ) -> Result<Read<(CalibrationId, CalibrationResult)>, Error> {
        let mut statement = self.conn().prepare_cached(
            "SELECT id, set_id, day, protocol, value, unit, rir_declared
             FROM calibration_result
             WHERE deleted_at IS NULL AND exercise_id = ?1
             ORDER BY day, id",
        )?;
        let mut rows = statement.query([exercise.uuid().as_bytes().as_slice()])?;
        let mut read = Read::default();
        while let Some(row) = rows.next()? {
            let rir = row.get::<_, Option<i64>>(6)?.map_or(Some(None), |raw| {
                u8::try_from(raw).ok().and_then(Rir::new).map(Some)
            });
            let decoded = (
                day_from_number(row.get(2)?),
                code::<CalibrationProtocol>(row.get(3)?),
                measure_from(row.get(4)?, row.get(5)?),
                rir,
            );
            let (Ok(day), Some(protocol), Some(measure), Some(rir)) = decoded else {
                read.undecodable += 1;
                continue;
            };
            read.rows.push((
                Id::from(Uuid::from_bytes(row.get(0)?)),
                CalibrationResult {
                    exercise,
                    day,
                    protocol,
                    measure,
                    rir,
                    set: row
                        .get::<_, Option<[u8; 16]>>(1)?
                        .map(|bytes| Id::from(Uuid::from_bytes(bytes))),
                },
            ));
        }
        Ok(read)
    }

    /// Store a suggestion the engine produced.
    pub fn record_suggestion(&mut self, suggestion: &Suggestion) -> Result<SuggestionId, Error> {
        if suggestion.inputs.len() > MAX_INPUTS {
            return Err(Error::OutOfRange("suggestion inputs"));
        }
        let id = Uuid::now_v7();
        self.write(|tx, stamp| {
            tx.execute(
                "INSERT INTO suggestion
                   (id, created_at, rule, rule_version, model, muscle, kind, confidence_bp,
                    inputs, hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id.as_bytes().as_slice(),
                    suggestion.created_at.as_millisecond(),
                    suggestion.rule,
                    suggestion.rule_version,
                    suggestion.model.code(),
                    suggestion.muscle.map(Muscle::code),
                    suggestion.kind.code(),
                    suggestion.confidence.basis_points(),
                    suggestion.inputs,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::Suggestion, id, Op::Upsert, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// Answer a suggestion. Answers are appended: the latest one is current
    /// and earlier ones stay as history.
    pub fn answer_suggestion(
        &mut self,
        suggestion: SuggestionId,
        answer: Answer,
    ) -> Result<(), Error> {
        let suggestion_id = suggestion.uuid();
        let id = Uuid::now_v7();
        self.write(|tx, stamp| {
            let exists: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM suggestion WHERE id = ?1 AND deleted_at IS NULL",
                    [suggestion_id.as_bytes().as_slice()],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Err(Error::NotFound("live suggestion"));
            }
            tx.execute(
                "INSERT INTO suggestion_response
                   (id, suggestion_id, response, reason, at, hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id.as_bytes().as_slice(),
                    suggestion_id.as_bytes().as_slice(),
                    answer.response.code(),
                    answer.reason.map(ResponseReason::code),
                    answer.at.as_millisecond(),
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            sync::enqueue(tx, Entity::SuggestionResponse, id, Op::Upsert, stamp)
        })
    }

    /// Live suggestions created at or after `since`, newest first, each with
    /// its latest answer.
    pub fn suggestions_since(
        &self,
        since: Timestamp,
    ) -> Result<Read<(SuggestionId, Suggestion, Option<Answer>)>, Error> {
        let mut statement = self.conn().prepare_cached(
            "SELECT s.id, s.created_at, s.rule, s.rule_version, s.model, s.muscle, s.kind,
                    s.confidence_bp, s.inputs, r.response, r.reason, r.at
             FROM suggestion s
             LEFT JOIN suggestion_response r ON r.id = (
               SELECT id FROM suggestion_response
               WHERE suggestion_id = s.id AND deleted_at IS NULL
               ORDER BY at DESC, hlc DESC, id DESC LIMIT 1)
             WHERE s.deleted_at IS NULL AND s.created_at >= ?1
             ORDER BY s.created_at DESC, s.id DESC",
        )?;
        let mut rows = statement.query([since.as_millisecond()])?;
        let mut read = Read::default();
        while let Some(row) = rows.next()? {
            let muscle = optional_code::<Muscle>(row.get(5)?);
            let answer = match row.get::<_, Option<i64>>(9)? {
                None => Some(None),
                Some(raw) => {
                    let reason = optional_code::<ResponseReason>(row.get(10)?);
                    let at = Timestamp::from_millisecond(row.get(11)?).ok();
                    match (code::<Response>(raw), reason, at) {
                        (Some(response), Ok(reason), Some(at)) => Some(Some(Answer {
                            response,
                            reason,
                            at,
                        })),
                        _ => None,
                    }
                }
            };
            let decoded = (
                Timestamp::from_millisecond(row.get(1)?),
                u16::try_from(row.get::<_, i64>(2)?),
                u16::try_from(row.get::<_, i64>(3)?),
                code::<LoadModel>(row.get(4)?),
                muscle,
                code::<SuggestionKind>(row.get(6)?),
                u16::try_from(row.get::<_, i64>(7)?)
                    .ok()
                    .and_then(Share::from_basis_points),
                answer,
            );
            let (
                Ok(created_at),
                Ok(rule),
                Ok(rule_version),
                Some(model),
                Ok(muscle),
                Some(kind),
                Some(confidence),
                Some(answer),
            ) = decoded
            else {
                read.undecodable += 1;
                continue;
            };
            read.rows.push((
                Id::from(Uuid::from_bytes(row.get(0)?)),
                Suggestion {
                    created_at,
                    rule,
                    rule_version,
                    model,
                    muscle,
                    kind,
                    confidence,
                    inputs: row.get(8)?,
                },
                answer,
            ));
        }
        Ok(read)
    }

    fn tombstone_plan_row(
        &mut self,
        table: &'static str,
        entity: Entity,
        id: Uuid,
    ) -> Result<(), Error> {
        // `table` is one of this module's literals, never user input.
        let sql = format!(
            "UPDATE {table} SET deleted_at = ?2, hlc = ?3, node = ?4, local_seq = ?5
             WHERE id = ?1 AND deleted_at IS NULL"
        );
        self.write(|tx, stamp| {
            let changed = tx.execute(
                &sql,
                params![
                    id.as_bytes().as_slice(),
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
}
