//! `catalog.db`: exercises from free-exercise-db, foods from USDA FDC
//! (Foundation Foods first, SR Legacy for coverage).

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::Path;

use myos_core::code::Code;
use myos_core::diet::{JOULES_PER_KCAL, Nutrient};
use myos_core::exercise::Modality;
use myos_core::muscle::{Muscle, MuscleRole};
use myos_store::catalog::{CATALOG_SCHEMA, SCHEMA_VERSION};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use uuid::Uuid;

use crate::BuildError;
use crate::fetch::{self, FDC_FOUNDATION, FDC_SR_LEGACY, FREE_EXERCISE_DB};

/// Language of the source names. Translations come from curation later.
const LANG: &str = "en";

/// Stable catalog id of `key` (`exercise:<slug>`, `food:fdc:<id>`).
pub(crate) fn catalog_id(key: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("https://myos.app/catalog/{key}").as_bytes(),
    )
}

/// `version` from a reference file's meta table.
pub(crate) fn read_version(path: &Path, table: &str) -> Result<u32, BuildError> {
    let conn = Connection::open(path)?;
    let value: Option<String> = conn
        .query_row(
            &format!("SELECT value FROM {table} WHERE key = 'version'"),
            [],
            |row| row.get(0),
        )
        .optional()?;
    value
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| BuildError::Invalid(format!("{} has no version", path.display())))
}

/// Create an empty reference file with `schema`, replacing `path`.
pub(crate) fn create(path: &Path, schema: &str) -> Result<Connection, BuildError> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let conn = Connection::open(path)?;
    // Attached immutable on devices: no WAL, no journal left behind.
    conn.execute_batch("PRAGMA journal_mode = DELETE; PRAGMA page_size = 4096;")?;
    conn.execute_batch(schema)?;
    Ok(conn)
}

/// Rebuild the search indexes, compact, and check the file.
pub(crate) fn finish(conn: &Connection, fts: &[&str]) -> Result<(), BuildError> {
    for table in fts {
        conn.execute_batch(&format!(
            "INSERT INTO {table} ({table}) VALUES ('rebuild');
             INSERT INTO {table} ({table}) VALUES ('optimize');"
        ))?;
    }
    conn.execute_batch("ANALYZE; VACUUM;")?;
    let check: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    let violations: i64 =
        conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if check != "ok" || violations > 0 {
        return Err(BuildError::Invalid(format!(
            "integrity check failed: {check}, {violations} foreign key violations"
        )));
    }
    Ok(())
}

pub(crate) fn meta(
    tx: &Transaction<'_>,
    table: &str,
    pairs: &[(&str, String)],
) -> Result<(), BuildError> {
    for (key, value) in pairs {
        tx.execute(
            &format!("INSERT INTO {table} (key, value) VALUES (?1, ?2)"),
            params![key, value],
        )?;
    }
    Ok(())
}

pub(crate) fn now_iso() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    format!("{seconds}")
}

/// Build `catalog.db` at `path`.
pub(crate) fn build(path: &Path, version: u32, cache: &Path) -> Result<(), BuildError> {
    let exercises: Vec<FreeExercise> = serde_json::from_reader(std::fs::File::open(
        fetch::cached(cache, &FREE_EXERCISE_DB)?,
    )?)?;
    let foundation: FoundationFile = read_zip_json(&fetch::cached(cache, &FDC_FOUNDATION)?)?;
    let legacy: LegacyFile = read_zip_json(&fetch::cached(cache, &FDC_SR_LEGACY)?)?;

    let mut conn = create(path, CATALOG_SCHEMA)?;
    let tx = conn.transaction()?;
    meta(
        &tx,
        "catalog_meta",
        &[
            ("schema_version", SCHEMA_VERSION.to_string()),
            ("version", version.to_string()),
            ("built_at", now_iso()),
        ],
    )?;
    tx.execute(
        "INSERT INTO source (id, name, license, attribution, url, version) VALUES
         (1, 'free-exercise-db', 'Unlicense', 'free-exercise-db contributors',
          'https://github.com/yuhonas/free-exercise-db', 'f00c92c'),
         (2, 'USDA FoodData Central', 'CC0-1.0',
          'U.S. Department of Agriculture, Agricultural Research Service. FoodData Central.',
          'https://fdc.nal.usda.gov', 'Foundation 2026-10-29, SR Legacy 2018-04')",
        [],
    )?;
    let exercise_count = insert_exercises(&tx, &exercises)?;
    let mut seen = HashSet::new();
    let mut food_count = 0;
    for food in foundation.foods.iter().chain(&legacy.foods) {
        // Foundation first: an SR Legacy food with the same name is skipped.
        if seen.insert(food.description.to_lowercase()) && insert_food(&tx, food)? {
            food_count += 1;
        }
    }
    tx.commit()?;
    finish(&conn, &["exercise_search", "food_search"])?;
    eprintln!("catalog.db v{version}: {exercise_count} exercises, {food_count} foods");
    Ok(())
}

fn read_zip_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, BuildError> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let index = (0..archive.len())
        .find(|index| {
            archive.by_index(*index).is_ok_and(|file| {
                Path::new(file.name())
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            })
        })
        .ok_or_else(|| BuildError::Invalid(format!("no JSON in {}", path.display())))?;
    let mut text = String::new();
    archive.by_index(index)?.read_to_string(&mut text)?;
    Ok(serde_json::from_str(&text)?)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FreeExercise {
    id: String,
    name: String,
    category: String,
    equipment: Option<String>,
    primary_muscles: Vec<String>,
    secondary_muscles: Vec<String>,
}

/// free-exercise-db muscle names → Myos muscles. "shoulders" is the front
/// deltoid unless the exercise works the rear one.
fn muscle(name: &str, exercise: &str) -> Option<Muscle> {
    let exercise = exercise.to_lowercase();
    Some(match name {
        "abdominals" => Muscle::Abs,
        "abductors" => Muscle::Abductors,
        "adductors" => Muscle::Adductors,
        "biceps" => Muscle::Biceps,
        "calves" => Muscle::Calves,
        "chest" => Muscle::Chest,
        "forearms" => Muscle::Forearms,
        "glutes" => Muscle::Glutes,
        "hamstrings" => Muscle::Hamstrings,
        "lats" | "middle back" => Muscle::UpperBack,
        "lower back" => Muscle::LowerBack,
        "neck" => Muscle::Neck,
        "quadriceps" => Muscle::Quadriceps,
        "traps" => Muscle::Trapezius,
        "triceps" => Muscle::Triceps,
        "shoulders"
            if [
                "rear",
                "reverse fly",
                "reverse flye",
                "face pull",
                "reverse pec",
            ]
            .iter()
            .any(|hint| exercise.contains(hint)) =>
        {
            Muscle::RearDeltoids
        }
        "shoulders" => Muscle::FrontDeltoids,
        _ => return None,
    })
}

fn modality(exercise: &FreeExercise) -> Option<Modality> {
    let equipment = exercise.equipment.as_deref().unwrap_or("body only");
    match (exercise.category.as_str(), equipment) {
        // No load to account for.
        ("stretching", _) | (_, "foam roll") => None,
        ("cardio", _) => Some(Modality::Cardio),
        ("strength" | "plyometrics", "body only") => Some(Modality::Calisthenics),
        _ => Some(Modality::Strength),
    }
}

fn insert_exercises(tx: &Transaction<'_>, exercises: &[FreeExercise]) -> Result<usize, BuildError> {
    let mut count = 0;
    let mut slugs = HashSet::new();
    for exercise in exercises {
        let Some(modality) = modality(exercise) else {
            continue;
        };
        let slug = exercise.id.to_lowercase().replace(['_', ' '], "-");
        if !slugs.insert(slug.clone()) {
            return Err(BuildError::Invalid(format!(
                "duplicate exercise slug {slug}"
            )));
        }
        // Strongest role per muscle (two source muscles can map to one).
        let mut targets: BTreeMap<Muscle, MuscleRole> = BTreeMap::new();
        for (names, role) in [
            (&exercise.primary_muscles, MuscleRole::Primary),
            (&exercise.secondary_muscles, MuscleRole::Secondary),
        ] {
            for name in names {
                if let Some(muscle) = muscle(name, &exercise.name) {
                    targets.entry(muscle).or_insert(role);
                }
            }
        }
        let id = catalog_id(&format!("exercise:{slug}"));
        tx.execute(
            "INSERT INTO exercise (id, slug, modality) VALUES (?1, ?2, ?3)",
            params![id.as_bytes().as_slice(), slug, modality.code()],
        )?;
        for (muscle, role) in targets {
            tx.execute(
                "INSERT INTO exercise_target (exercise_id, muscle, role) VALUES (?1, ?2, ?3)",
                params![id.as_bytes().as_slice(), muscle.code(), role.code()],
            )?;
        }
        tx.execute(
            "INSERT INTO exercise_name (exercise_id, lang, name) VALUES (?1, ?2, ?3)",
            params![id.as_bytes().as_slice(), LANG, exercise.name.trim()],
        )?;
        count += 1;
    }
    Ok(count)
}

#[derive(Debug, Deserialize)]
struct FoundationFile {
    #[serde(rename = "FoundationFoods")]
    foods: Vec<FdcFood>,
}

#[derive(Debug, Deserialize)]
struct LegacyFile {
    #[serde(rename = "SRLegacyFoods")]
    foods: Vec<FdcFood>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FdcFood {
    fdc_id: u64,
    description: String,
    #[serde(default)]
    food_nutrients: Vec<FdcNutrient>,
    #[serde(default)]
    food_portions: Vec<FdcPortion>,
}

#[derive(Debug, Deserialize)]
struct FdcNutrient {
    nutrient: Option<FdcNutrientId>,
    amount: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct FdcNutrientId {
    number: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FdcPortion {
    amount: Option<f64>,
    gram_weight: Option<f64>,
    modifier: Option<String>,
    measure_unit: Option<FdcUnit>,
    portion_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FdcUnit {
    name: Option<String>,
}

/// Per-100 g amounts in Myos units (joules, mg) from FDC nutrient numbers.
fn nutrients(food: &FdcFood) -> BTreeMap<Nutrient, u64> {
    let mut by_number: BTreeMap<&str, f64> = BTreeMap::new();
    for entry in &food.food_nutrients {
        if let (Some(number), Some(amount)) = (
            entry.nutrient.as_ref().and_then(|id| id.number.as_deref()),
            entry.amount,
        ) && amount.is_finite()
            && amount >= 0.0
        {
            by_number.entry(number).or_insert(amount);
        }
    }
    let first = |numbers: &[&str]| {
        numbers
            .iter()
            .find_map(|number| by_number.get(number).copied())
    };
    let mut out = BTreeMap::new();
    let mut put = |nutrient, value: Option<f64>| {
        if let Some(value) = value {
            // Rounded; amounts are small and non-negative.
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "checked finite and non-negative above"
            )]
            out.insert(nutrient, value.round() as u64);
        }
    };
    // Energy: kJ (268) when present, else kcal (208, then Atwater 957 / 958).
    let energy = first(&["268"]).map(|kj| kj * 1000.0).or_else(|| {
        first(&["208", "957", "958"])
            .map(|kcal| kcal * f64::from(u32::try_from(JOULES_PER_KCAL).unwrap_or(4184)))
    });
    put(Nutrient::Energy, energy);
    let grams = |numbers: &[&str]| first(numbers).map(|grams| grams * 1000.0);
    put(Nutrient::Protein, grams(&["203"]));
    put(Nutrient::Carbohydrate, grams(&["205", "205.2"]));
    put(Nutrient::Fat, grams(&["204", "298"]));
    put(Nutrient::Fiber, grams(&["291"]));
    put(Nutrient::Sugar, grams(&["269", "269.3"]));
    put(Nutrient::SaturatedFat, grams(&["606"]));
    put(Nutrient::Sodium, first(&["307"]));
    out
}

fn portion_label(portion: &FdcPortion) -> Option<String> {
    let amount = portion.amount.filter(|amount| *amount > 0.0)?;
    let unit = portion
        .measure_unit
        .as_ref()
        .and_then(|unit| unit.name.as_deref())
        .filter(|name| !matches!(*name, "undetermined" | "RACC"))
        .map(str::to_owned)
        .or_else(|| portion.modifier.clone().filter(|text| !text.is_empty()))
        .or_else(|| portion.portion_description.clone())?;
    let amount = if amount.fract() == 0.0 {
        format!("{amount:.0}")
    } else {
        format!("{amount}")
    };
    Some(format!("{amount} {}", unit.trim()))
}

/// Insert a food; `false` when it lacks energy or macronutrients.
fn insert_food(tx: &Transaction<'_>, food: &FdcFood) -> Result<bool, BuildError> {
    let amounts = nutrients(food);
    if !amounts.contains_key(&Nutrient::Energy) || !amounts.contains_key(&Nutrient::Protein) {
        return Ok(false);
    }
    let id = catalog_id(&format!("food:fdc:{}", food.fdc_id));
    tx.execute(
        "INSERT INTO food (id) VALUES (?1)",
        [id.as_bytes().as_slice()],
    )?;
    tx.execute(
        "INSERT INTO food_name (food_id, lang, name) VALUES (?1, ?2, ?3)",
        params![id.as_bytes().as_slice(), LANG, food.description.trim()],
    )?;
    for (nutrient, amount) in amounts {
        tx.execute(
            "INSERT INTO food_nutrient (food_id, nutrient, amount_100g) VALUES (?1, ?2, ?3)",
            params![
                id.as_bytes().as_slice(),
                nutrient.code(),
                i64::try_from(amount).unwrap_or(i64::MAX)
            ],
        )?;
    }
    let mut labels = HashSet::new();
    for (position, portion) in food.food_portions.iter().enumerate() {
        let (Some(label), Some(grams)) = (portion_label(portion), portion.gram_weight) else {
            continue;
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to a positive u32 range first"
        )]
        let grams_mg = (grams * 1000.0).round().clamp(1.0, f64::from(u32::MAX)) as u32;
        if grams.is_finite() && grams > 0.0 && labels.insert(label.clone()) {
            tx.execute(
                "INSERT INTO food_portion (food_id, position, lang, label, grams_mg)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id.as_bytes().as_slice(),
                    i64::try_from(position).unwrap_or(i64::MAX),
                    LANG,
                    label,
                    grams_mg
                ],
            )?;
        }
    }
    Ok(true)
}
