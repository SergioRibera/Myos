//! TLV layouts of structured columns. Tags are append-only per layout.
//!
//! - Nutrients: tag = `Nutrient` code, value = amount (joules / mg).
//! - Lists: a record whose tag `n` (from 1) holds element `n`.
//! - Routine content: `1 days[]`; day `1 name, 2 blocks[]`; block `1 kind,
//!   2 rest_ms, 3 items[]`; item `1 id, 2 exercise, 3 sets, 4 reps_min,
//!   5 reps_max, 6 rir, 7 rpe_half, 8 rest_ms, 9 load_g, 10 progression,
//!   11 step_g`.

use std::time::Duration;

use myos_core::code::Code;
use myos_core::diet::{Nutrient, Nutrients};
use myos_core::id::Id;
use myos_core::routine::{
    BlockKind, EffortTarget, RoutineBlock, RoutineContent, RoutineDay, RoutineItem,
};
use myos_core::session::{Rir, Rpe};
use myos_core::units::Mass;
use uuid::Uuid;

use crate::Error;
use crate::tlv::{Tlv, TlvError};

/// Size limit of a nutrient record.
pub(crate) const NUTRIENTS_LIMIT: usize = 4 * 1024;
/// Size limit of a routine content record.
pub(crate) const ROUTINE_LIMIT: usize = 256 * 1024;

/// Encode per-100 g amounts.
pub(crate) fn encode_nutrients(nutrients: &Nutrients) -> Vec<u8> {
    let mut record = Tlv::default();
    for (nutrient, amount) in nutrients.iter() {
        record.put_uint(nutrient.code(), amount);
    }
    record.encode()
}

/// Decode per-100 g amounts. A nutrient this build does not know fails the
/// decode: the row reads as undecodable instead of losing that nutrient when
/// it is rewritten.
pub(crate) fn decode_nutrients(bytes: &[u8]) -> Result<Nutrients, TlvError> {
    let record = Tlv::decode(bytes, NUTRIENTS_LIMIT)?;
    let mut nutrients = Nutrients::default();
    for tag in record.tags() {
        let nutrient = Nutrient::from_code(tag).ok_or(TlvError::Malformed { tag })?;
        if let Some(amount) = record.uint(tag)? {
            nutrients = nutrients.with(nutrient, amount);
        }
    }
    Ok(nutrients)
}

fn put_list(record: &mut Tlv, tag: u16, items: impl IntoIterator<Item = Tlv>) -> Result<(), Error> {
    let mut list = Tlv::default();
    for (index, item) in items.into_iter().enumerate() {
        let position = u16::try_from(index + 1).map_err(|_| Error::OutOfRange("list length"))?;
        list.put_record(position, &item);
    }
    record.put_record(tag, &list);
    Ok(())
}

/// Fail on a tag outside `known`: a routine written by a newer build is shown
/// whole or not at all, never silently stripped when saved again.
fn known_tags(record: &Tlv, known: &[u16]) -> Result<(), TlvError> {
    record
        .tags()
        .find(|tag| !known.contains(tag))
        .map_or(Ok(()), |tag| Err(TlvError::Malformed { tag }))
}
fn list(record: &Tlv, tag: u16) -> Result<Vec<Tlv>, TlvError> {
    let Some(list) = record.record(tag, ROUTINE_LIMIT)? else {
        return Ok(Vec::new());
    };
    // Positions are 1..=n with no gaps.
    if list
        .tags()
        .zip(1_u16..)
        .any(|(position, expected)| position != expected)
    {
        return Err(TlvError::Malformed { tag });
    }
    list.tags()
        .map(|position| {
            list.record(position, ROUTINE_LIMIT)?
                .ok_or(TlvError::Malformed { tag: position })
        })
        .collect()
}

fn millis(duration: Duration) -> Result<u64, Error> {
    u64::try_from(duration.as_millis()).map_err(|_| Error::OutOfRange("duration"))
}

fn uuid_at(record: &Tlv, tag: u16) -> Result<Uuid, TlvError> {
    record
        .bytes(tag)
        .and_then(|bytes| Uuid::from_slice(bytes).ok())
        .ok_or(TlvError::Malformed { tag })
}

fn narrow<T: TryFrom<u64>>(record: &Tlv, tag: u16) -> Result<Option<T>, TlvError> {
    record
        .uint(tag)?
        .map(|value| T::try_from(value).map_err(|_| TlvError::Malformed { tag }))
        .transpose()
}

fn code_at<T: Code>(record: &Tlv, tag: u16) -> Result<Option<T>, TlvError> {
    narrow::<u16>(record, tag)?
        .map(|code| T::from_code(code).ok_or(TlvError::Malformed { tag }))
        .transpose()
}

fn encode_item(item: &RoutineItem) -> Tlv {
    let mut record = Tlv::default();
    record
        .put_bytes(1, item.id.uuid().as_bytes())
        .put_bytes(2, item.exercise.uuid().as_bytes())
        .put_uint(3, u64::from(item.sets))
        .put_opt_uint(4, item.reps.map(|(min, _)| u64::from(min)))
        .put_opt_uint(5, item.reps.map(|(_, max)| u64::from(max)))
        .put_opt_uint(
            6,
            match item.effort {
                Some(EffortTarget::Rir(rir)) => Some(u64::from(rir.get())),
                _ => None,
            },
        )
        .put_opt_uint(
            7,
            match item.effort {
                Some(EffortTarget::Rpe(rpe)) => Some(u64::from(rpe.half_points())),
                _ => None,
            },
        )
        .put_opt_uint(9, item.load.map(|load| u64::from(load.grams())))
        .put_uint(10, u64::from(item.progression.code()))
        .put_opt_uint(11, item.step.map(|step| u64::from(step.grams())));
    record
}

fn decode_item(record: &Tlv) -> Result<RoutineItem, TlvError> {
    known_tags(record, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])?;
    let reps = match (narrow::<u16>(record, 4)?, narrow::<u16>(record, 5)?) {
        (Some(min), Some(max)) if min <= max => Some((min, max)),
        (None, None) => None,
        _ => return Err(TlvError::Malformed { tag: 4 }),
    };
    let effort = match (narrow::<u8>(record, 6)?, narrow::<u8>(record, 7)?) {
        (None, None) => None,
        (Some(rir), None) => Some(EffortTarget::Rir(
            Rir::new(rir).ok_or(TlvError::Malformed { tag: 6 })?,
        )),
        (None, Some(half)) => Some(EffortTarget::Rpe(
            Rpe::from_half_points(half).ok_or(TlvError::Malformed { tag: 7 })?,
        )),
        (Some(_), Some(_)) => return Err(TlvError::Malformed { tag: 6 }),
    };
    Ok(RoutineItem {
        id: Id::from(uuid_at(record, 1)?),
        exercise: Id::from(uuid_at(record, 2)?),
        sets: narrow(record, 3)?.ok_or(TlvError::Malformed { tag: 3 })?,
        reps,
        effort,
        rest: record.uint(8)?.map(Duration::from_millis),
        load: narrow::<u32>(record, 9)?.map(Mass::from_grams),
        progression: code_at(record, 10)?.unwrap_or_default(),
        step: narrow::<u32>(record, 11)?.map(Mass::from_grams),
    })
}

/// Encode a routine version's content.
pub(crate) fn encode_routine(content: &RoutineContent) -> Result<Vec<u8>, Error> {
    let mut days = Vec::with_capacity(content.days.len());
    for day in &content.days {
        let mut blocks = Vec::with_capacity(day.blocks.len());
        for block in &day.blocks {
            let mut items = Vec::with_capacity(block.items.len());
            for item in &block.items {
                let mut record = encode_item(item);
                record.put_opt_uint(8, item.rest.map(millis).transpose()?);
                items.push(record);
            }
            let mut record = Tlv::default();
            record
                .put_uint(1, u64::from(block.kind.code()))
                .put_opt_uint(2, block.rest.map(millis).transpose()?);
            put_list(&mut record, 3, items)?;
            blocks.push(record);
        }
        let mut record = Tlv::default();
        record.put_text(1, &day.name);
        put_list(&mut record, 2, blocks)?;
        days.push(record);
    }
    let mut record = Tlv::default();
    put_list(&mut record, 1, days)?;
    let bytes = record.encode();
    if bytes.len() > ROUTINE_LIMIT {
        return Err(Error::OutOfRange("routine size"));
    }
    Ok(bytes)
}

/// Decode a routine version's content. Unknown codes or shapes fail: a
/// routine is shown whole or not at all.
pub(crate) fn decode_routine(bytes: &[u8]) -> Result<RoutineContent, TlvError> {
    let record = Tlv::decode(bytes, ROUTINE_LIMIT)?;
    known_tags(&record, &[1])?;
    let mut content = RoutineContent::default();
    for day in list(&record, 1)? {
        known_tags(&day, &[1, 2])?;
        let mut blocks = Vec::new();
        for block in list(&day, 2)? {
            known_tags(&block, &[1, 2, 3])?;
            let items = list(&block, 3)?
                .iter()
                .map(decode_item)
                .collect::<Result<_, _>>()?;
            blocks.push(RoutineBlock {
                kind: code_at::<BlockKind>(&block, 1)?.unwrap_or_default(),
                rest: block.uint(2)?.map(Duration::from_millis),
                items,
            });
        }
        content.days.push(RoutineDay {
            name: day.text(1)?.unwrap_or_default().to_owned(),
            blocks,
        });
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use myos_core::routine::{Progression, RoutineItemId};

    use super::*;

    fn sample() -> RoutineContent {
        let item = |sets, effort| RoutineItem {
            id: RoutineItemId::generate(),
            exercise: Id::generate(),
            sets,
            reps: Some((8, 12)),
            effort,
            rest: Some(Duration::from_secs(120)),
            load: Mass::from_kg(60.0),
            progression: Progression::DoubleProgression,
            step: Mass::from_kg(2.5),
        };
        RoutineContent {
            days: vec![
                RoutineDay {
                    name: "Empuje".into(),
                    blocks: vec![
                        RoutineBlock {
                            kind: BlockKind::Single,
                            rest: None,
                            items: vec![item(4, Rir::new(2).map(EffortTarget::Rir))],
                        },
                        RoutineBlock {
                            kind: BlockKind::Superset,
                            rest: Some(Duration::from_secs(90)),
                            items: vec![
                                item(3, Rpe::new(8.5).map(EffortTarget::Rpe)),
                                item(3, None),
                            ],
                        },
                    ],
                },
                RoutineDay {
                    name: "Piernas".into(),
                    blocks: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn routine_content_round_trips() -> Result<(), Error> {
        let content = sample();
        let bytes = encode_routine(&content)?;
        assert_eq!(decode_routine(&bytes).map_err(Error::Payload)?, content);
        assert_eq!(
            decode_routine(&encode_routine(&RoutineContent::default())?).map_err(Error::Payload)?,
            RoutineContent::default()
        );
        Ok(())
    }

    #[test]
    fn nutrients_round_trip_and_reject_unknown_tags() -> Result<(), TlvError> {
        let nutrients = Nutrients::default()
            .with(Nutrient::Energy, 544_000)
            .with(Nutrient::Protein, 31_000);
        let mut record = Tlv::decode(&encode_nutrients(&nutrients), NUTRIENTS_LIMIT)?;
        assert_eq!(decode_nutrients(&record.encode())?, nutrients);
        record.put_uint(999, 5);
        assert_eq!(
            decode_nutrients(&record.encode()),
            Err(TlvError::Malformed { tag: 999 })
        );
        Ok(())
    }

    #[test]
    fn malformed_routines_are_rejected() {
        let mut item = Tlv::default();
        item.put_bytes(1, &[1, 2, 3]);
        let mut items = Tlv::default();
        items.put_record(1, &item);
        let mut block = Tlv::default();
        block.put_record(3, &items);
        let mut blocks = Tlv::default();
        blocks.put_record(1, &block);
        let mut day = Tlv::default();
        day.put_record(2, &blocks);
        let mut days = Tlv::default();
        days.put_record(1, &day);
        let mut root = Tlv::default();
        root.put_record(1, &days);
        assert!(decode_routine(&root.encode()).is_err());
    }

    #[test]
    fn routines_with_tags_from_a_newer_build_are_rejected() -> Result<(), Error> {
        let mut root = Tlv::decode(&encode_routine(&sample())?, ROUTINE_LIMIT)?;
        root.put_uint(9, 1);
        assert_eq!(
            decode_routine(&root.encode()),
            Err(TlvError::Malformed { tag: 9 })
        );
        Ok(())
    }
}
