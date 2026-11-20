//! Meals and custom foods.

use std::collections::HashMap;

use jiff::Timestamp;
use myos_core::code::Code;
use myos_core::diet::{
    CustomFood, Food, FoodRef, Meal, MealContent, MealId, MealItem, MealSlot, PortionSize,
    ProteinLevel,
};
use myos_core::id::Id;
use myos_core::volume::DaySpan;
use rusqlite::{Row, Transaction, params};
use uuid::Uuid;

use crate::content::{decode_nutrients, encode_nutrients};
use crate::convert::{MAX_INSTANT_MS, day_from_number, day_number};
use crate::sync::{self, Entity, Op};
use crate::{Error, Stamp, Store};

/// Meals read for a span of days.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MealsRead {
    /// Live meals, oldest first.
    pub meals: Vec<(MealId, Meal)>,
    /// Meals with values this build cannot read (left out whole).
    pub undecodable: u32,
}

/// Identifier of a custom food.
pub type FoodId = Id<Food>;

const MODE_QUICK: u8 = 1;
const MODE_DETAILED: u8 = 2;

const MAX_NAME: usize = 200;
const MAX_BRAND: usize = 120;
const MAX_BARCODE: usize = 32;
const MAX_NOTE: usize = 500;
const MAX_LABEL: usize = 80;
const MAX_GRAMS_MG: u32 = 100_000_000;

fn text_ok(value: &str, max: usize) -> bool {
    !value.is_empty() && value.chars().count() <= max
}

fn check_text(value: &str, max: usize, what: &'static str) -> Result<(), Error> {
    if text_ok(value, max) {
        Ok(())
    } else {
        Err(Error::OutOfRange(what))
    }
}

fn check_opt_text(value: Option<&str>, max: usize, what: &'static str) -> Result<(), Error> {
    value.map_or(Ok(()), |value| check_text(value, max, what))
}

fn check_grams(grams_mg: u32) -> Result<(), Error> {
    if (1..=MAX_GRAMS_MG).contains(&grams_mg) {
        Ok(())
    } else {
        Err(Error::OutOfRange("grams"))
    }
}

/// `(food_src, food_id, barcode)` columns of a food reference.
const fn food_columns(food: &FoodRef) -> (u8, Option<[u8; 16]>, Option<&str>) {
    match food {
        FoodRef::Catalog(id) => (1, Some(*id.uuid().as_bytes()), None),
        FoodRef::OpenFoodFacts(barcode) => (2, None, Some(barcode.as_str())),
        FoodRef::Custom(id) => (3, Some(*id.uuid().as_bytes()), None),
        FoodRef::Recipe(id) => (4, Some(*id.uuid().as_bytes()), None),
    }
}

fn food_from(src: i64, id: Option<[u8; 16]>, barcode: Option<String>) -> Option<FoodRef> {
    let id = id.map(Uuid::from_bytes);
    match (src, id, barcode) {
        (1, Some(id), None) => Some(FoodRef::Catalog(Id::from(id))),
        (2, None, Some(barcode)) => Some(FoodRef::OpenFoodFacts(barcode)),
        (3, Some(id), None) => Some(FoodRef::Custom(Id::from(id))),
        (4, Some(id), None) => Some(FoodRef::Recipe(Id::from(id))),
        _ => None,
    }
}

fn validate_meal(meal: &Meal, tz_offset_min: i16) -> Result<(), Error> {
    if !(-840..=840).contains(&tz_offset_min) {
        return Err(Error::OutOfRange("tz_offset_min"));
    }
    check_opt_text(meal.note.as_deref(), MAX_NOTE, "note")?;
    if !(0..=MAX_INSTANT_MS).contains(&meal.eaten_at.as_millisecond()) {
        return Err(Error::OutOfRange("eaten_at"));
    }
    if let MealContent::Detailed(items) = &meal.content {
        for item in items {
            check_grams(item.grams_mg)?;
            check_text(&item.name, MAX_NAME, "food name")?;
            check_opt_text(item.brand.as_deref(), MAX_BRAND, "brand")?;
            if let FoodRef::OpenFoodFacts(barcode) = &item.food {
                check_text(barcode, MAX_BARCODE, "barcode")?;
            }
        }
    }
    Ok(())
}

fn insert_items(
    tx: &Transaction<'_>,
    meal: Uuid,
    generation: Uuid,
    items: &[MealItem],
    stamp: Stamp,
) -> Result<(), Error> {
    for (position, item) in items.iter().enumerate() {
        let id = Uuid::now_v7();
        let (src, food_id, barcode) = food_columns(&item.food);
        tx.execute(
            "INSERT INTO meal_item
               (id, meal_id, generation, position, food_src, food_id, barcode, grams_mg,
                name_snapshot, brand_snapshot, nutrients, hlc, node, local_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                id.as_bytes().as_slice(),
                meal.as_bytes().as_slice(),
                generation.as_bytes().as_slice(),
                i64::try_from(position).map_err(|_| Error::OutOfRange("items"))?,
                src,
                food_id,
                barcode,
                item.grams_mg,
                item.name,
                item.brand,
                encode_nutrients(&item.density),
                sync::sql_hlc(stamp.hlc)?,
                stamp.node,
                stamp.local_seq,
            ],
        )?;
        sync::enqueue(tx, Entity::MealItem, id, Op::Upsert, stamp)?;
    }
    Ok(())
}

/// Meal header columns: `(mode, quick_size, quick_protein)`.
fn content_columns(content: &MealContent) -> (u8, Option<u16>, Option<u16>) {
    match content {
        MealContent::Quick { size, protein } => {
            (MODE_QUICK, Some(size.code()), Some(protein.code()))
        }
        MealContent::Detailed(_) => (MODE_DETAILED, None, None),
    }
}

fn decode_item(row: &Row<'_>) -> Result<Option<MealItem>, Error> {
    let Some(food) = food_from(row.get(1)?, row.get(2)?, row.get(3)?) else {
        return Ok(None);
    };
    let Ok(grams_mg) = u32::try_from(row.get::<_, i64>(4)?) else {
        return Ok(None);
    };
    let Ok(density) = decode_nutrients(&row.get::<_, Vec<u8>>(7)?) else {
        return Ok(None);
    };
    Ok(Some(MealItem {
        food,
        grams_mg,
        name: row.get(5)?,
        brand: row.get(6)?,
        density,
    }))
}

impl Store {
    /// Log a meal eaten on a clock `tz_offset_min` minutes from UTC.
    pub fn log_meal(&mut self, meal: &Meal, tz_offset_min: i16) -> Result<MealId, Error> {
        validate_meal(meal, tz_offset_min)?;
        let day = day_number(meal.day)?;
        let id = Uuid::now_v7();
        let (mode, quick_size, quick_protein) = content_columns(&meal.content);
        let generation = Uuid::now_v7();
        self.write(|tx, stamp| {
            tx.execute(
                "INSERT INTO meal
                   (id, eaten_at, day, tz_offset_min, slot, mode, quick_size, quick_protein,
                    note, items_gen, hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    id.as_bytes().as_slice(),
                    meal.eaten_at.as_millisecond(),
                    day,
                    tz_offset_min,
                    meal.slot.map(MealSlot::code),
                    mode,
                    quick_size,
                    quick_protein,
                    meal.note,
                    generation.as_bytes().as_slice(),
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            if let MealContent::Detailed(items) = &meal.content {
                insert_items(tx, id, generation, items, stamp)?;
            }
            // After the items: receivers see the new generation with its items.
            sync::enqueue(tx, Entity::Meal, id, Op::Upsert, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// Replace a live meal: header updated, previous items tombstoned, new
    /// items inserted.
    pub fn replace_meal(
        &mut self,
        meal_id: MealId,
        meal: &Meal,
        tz_offset_min: i16,
    ) -> Result<(), Error> {
        validate_meal(meal, tz_offset_min)?;
        let day = day_number(meal.day)?;
        let id = meal_id.uuid();
        let (mode, quick_size, quick_protein) = content_columns(&meal.content);
        let generation = Uuid::now_v7();
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE meal SET eaten_at = ?2, day = ?3, tz_offset_min = ?4, slot = ?5,
                   mode = ?6, quick_size = ?7, quick_protein = ?8, note = ?9, items_gen = ?13,
                   field_hlc = NULL, hlc = ?10, node = ?11, local_seq = ?12
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![
                    id.as_bytes().as_slice(),
                    meal.eaten_at.as_millisecond(),
                    day,
                    tz_offset_min,
                    meal.slot.map(MealSlot::code),
                    mode,
                    quick_size,
                    quick_protein,
                    meal.note,
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                    generation.as_bytes().as_slice(),
                ],
            )?;
            if changed == 0 {
                return Err(Error::NotFound("live meal"));
            }
            let old: Vec<[u8; 16]> = tx
                .prepare_cached(
                    "SELECT id FROM meal_item WHERE meal_id = ?1 AND deleted_at IS NULL",
                )?
                .query_map([id.as_bytes().as_slice()], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            for item in old {
                tx.execute(
                    "UPDATE meal_item SET deleted_at = ?2, hlc = ?3, node = ?4, local_seq = ?5
                     WHERE id = ?1",
                    params![
                        item.as_slice(),
                        stamp.now_millis,
                        sync::sql_hlc(stamp.hlc)?,
                        stamp.node,
                        stamp.local_seq,
                    ],
                )?;
                sync::enqueue(
                    tx,
                    Entity::MealItem,
                    Uuid::from_bytes(item),
                    Op::Delete,
                    stamp,
                )?;
            }
            if let MealContent::Detailed(items) = &meal.content {
                insert_items(tx, id, generation, items, stamp)?;
            }
            sync::enqueue(tx, Entity::Meal, id, Op::Upsert, stamp)
        })
    }

    /// Tombstone a meal; its items stop counting with it.
    pub fn delete_meal(&mut self, meal: MealId) -> Result<(), Error> {
        let id = meal.uuid();
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE meal SET deleted_at = ?2, hlc = ?3, node = ?4, local_seq = ?5
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
                return Err(Error::NotFound("live meal"));
            }
            sync::enqueue(tx, Entity::Meal, id, Op::Delete, stamp)
        })
    }

    /// Live meals eaten on a day in `span`, oldest first.
    pub fn meals_in(&self, span: DaySpan) -> Result<MealsRead, Error> {
        let first = day_number(span.first())?;
        let last = day_number(span.last())?;
        // One read transaction: the item list and the headers come from the
        // same snapshot.
        let tx = self.conn().unchecked_transaction()?;

        let mut items: HashMap<[u8; 16], Option<Vec<MealItem>>> = HashMap::new();
        let mut statement = tx.prepare_cached(
            "SELECT i.meal_id, i.food_src, i.food_id, i.barcode, i.grams_mg, i.name_snapshot,
                    i.brand_snapshot, i.nutrients
             FROM meal_item i JOIN meal m ON m.id = i.meal_id
             WHERE i.deleted_at IS NULL AND m.deleted_at IS NULL AND m.day BETWEEN ?1 AND ?2
               AND i.generation = m.items_gen
             ORDER BY i.meal_id, i.position, i.id",
        )?;
        let mut rows = statement.query([first, last])?;
        while let Some(row) = rows.next()? {
            let meal: [u8; 16] = row.get(0)?;
            let decoded = decode_item(row)?;
            let slot = items.entry(meal).or_insert_with(|| Some(Vec::new()));
            match (slot.as_mut(), decoded) {
                (Some(meal_items), Some(item)) => meal_items.push(item),
                // One unreadable item makes the whole meal unreadable.
                _ => *slot = None,
            }
        }

        let mut statement = tx.prepare_cached(
            "SELECT id, eaten_at, day, slot, mode, quick_size, quick_protein, note
             FROM meal WHERE deleted_at IS NULL AND day BETWEEN ?1 AND ?2
             ORDER BY eaten_at, id",
        )?;
        let mut rows = statement.query([first, last])?;
        let mut read = MealsRead::default();
        while let Some(row) = rows.next()? {
            let id: [u8; 16] = row.get(0)?;
            // `Some(None)`: no slot; `None`: a slot this build does not know.
            let slot = row.get::<_, Option<i64>>(3)?.map_or(Some(None), |raw| {
                u16::try_from(raw)
                    .ok()
                    .and_then(MealSlot::from_code)
                    .map(Some)
            });
            let content = match row.get::<_, i64>(4)? {
                1 => {
                    let size = row
                        .get::<_, Option<i64>>(5)?
                        .and_then(|raw| u16::try_from(raw).ok())
                        .and_then(PortionSize::from_code);
                    let protein = row
                        .get::<_, Option<i64>>(6)?
                        .and_then(|raw| u16::try_from(raw).ok())
                        .and_then(ProteinLevel::from_code);
                    size.zip(protein)
                        .map(|(size, protein)| MealContent::Quick { size, protein })
                }
                2 => items
                    .remove(&id)
                    .unwrap_or_else(|| Some(Vec::new()))
                    .map(MealContent::Detailed),
                _ => None,
            };
            let (Some(slot), Some(content), Ok(eaten_at), Ok(day)) = (
                slot,
                content,
                Timestamp::from_millisecond(row.get(1)?),
                day_from_number(row.get(2)?),
            ) else {
                read.undecodable += 1;
                continue;
            };
            read.meals.push((
                Id::from(Uuid::from_bytes(id)),
                Meal {
                    eaten_at,
                    day,
                    slot,
                    content,
                    note: row.get(7)?,
                },
            ));
        }
        Ok(read)
    }

    /// Create a custom food.
    pub fn create_custom_food(&mut self, food: &CustomFood) -> Result<FoodId, Error> {
        check_text(&food.name, MAX_NAME, "food name")?;
        check_opt_text(food.brand.as_deref(), MAX_BRAND, "brand")?;
        check_opt_text(food.barcode.as_deref(), MAX_BARCODE, "barcode")?;
        for portion in &food.portions {
            check_text(&portion.label, MAX_LABEL, "portion label")?;
            check_grams(portion.grams_mg)?;
        }
        let id = Uuid::now_v7();
        self.write(|tx, stamp| {
            tx.execute(
                "INSERT INTO custom_food
                   (id, name, brand, barcode, nutrients, hlc, node, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id.as_bytes().as_slice(),
                    food.name,
                    food.brand,
                    food.barcode,
                    encode_nutrients(&food.density),
                    sync::sql_hlc(stamp.hlc)?,
                    stamp.node,
                    stamp.local_seq,
                ],
            )?;
            for (position, portion) in food.portions.iter().enumerate() {
                tx.execute(
                    "INSERT INTO custom_food_portion (food_id, position, label, grams_mg)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        id.as_bytes().as_slice(),
                        i64::try_from(position).map_err(|_| Error::OutOfRange("portions"))?,
                        portion.label,
                        portion.grams_mg,
                    ],
                )?;
            }
            sync::enqueue(tx, Entity::CustomFood, id, Op::Upsert, stamp)
        })?;
        Ok(Id::from(id))
    }

    /// A live custom food with its portions.
    pub fn custom_food(&self, food: FoodId) -> Result<CustomFood, Error> {
        let id = food.uuid();
        let (name, brand, barcode, nutrients): (String, Option<String>, Option<String>, Vec<u8>) =
            self.conn()
                .query_row(
                    "SELECT name, brand, barcode, nutrients FROM custom_food
                     WHERE id = ?1 AND deleted_at IS NULL",
                    [id.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(|err| match err {
                    rusqlite::Error::QueryReturnedNoRows => Error::NotFound("live custom food"),
                    other => Error::Sqlite(other),
                })?;
        let portions = self
            .conn()
            .prepare_cached(
                "SELECT label, grams_mg FROM custom_food_portion
                 WHERE food_id = ?1 ORDER BY position",
            )?
            .query_map([id.as_bytes().as_slice()], |row| {
                Ok(myos_core::diet::Portion {
                    label: row.get(0)?,
                    grams_mg: row.get(1)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(CustomFood {
            name,
            brand,
            barcode,
            density: decode_nutrients(&nutrients)?,
            portions,
        })
    }

    /// Tombstone a custom food. Meals that used it keep their snapshot.
    pub fn delete_custom_food(&mut self, food: FoodId) -> Result<(), Error> {
        let id = food.uuid();
        self.write(|tx, stamp| {
            let changed = tx.execute(
                "UPDATE custom_food SET deleted_at = ?2, hlc = ?3, node = ?4, local_seq = ?5
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
                return Err(Error::NotFound("live custom food"));
            }
            sync::enqueue(tx, Entity::CustomFood, id, Op::Delete, stamp)
        })
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use myos_core::diet::{Nutrient, Nutrients};

    use super::*;
    use crate::DatabaseKey;

    fn item(name: &str) -> MealItem {
        MealItem {
            food: FoodRef::Catalog(Id::generate()),
            grams_mg: 100_000,
            name: name.into(),
            brand: None,
            density: Nutrients::default().with(Nutrient::Protein, 10_000),
        }
    }

    #[test]
    fn only_items_of_the_current_generation_count() -> Result<(), Error> {
        let mut store = Store::open_in_memory(&DatabaseKey::generate()?)?;
        let meal = Meal {
            eaten_at: Timestamp::UNIX_EPOCH,
            day: date(2026, 11, 18),
            slot: None,
            content: MealContent::Detailed(vec![item("Arroz")]),
            note: None,
        };
        store.log_meal(&meal, 0)?;
        // An item another device added under an older generation, arriving
        // after this device's list: it must not be summed in.
        store.conn.execute(
            "INSERT INTO meal_item
               (id, meal_id, generation, position, food_src, food_id, grams_mg, name_snapshot,
                nutrients, hlc, node, local_seq)
             SELECT ?1, id, ?2, 0, 1, ?3, 1000, 'Otro', x'', 1, 1, 1 FROM meal",
            params![
                Uuid::now_v7().as_bytes().as_slice(),
                Uuid::now_v7().as_bytes().as_slice(),
                Uuid::now_v7().as_bytes().as_slice(),
            ],
        )?;
        let read = store.meals_in(DaySpan::iso_week(date(2026, 11, 18)))?;
        assert_eq!(read.meals[0].1, meal);
        Ok(())
    }
}
