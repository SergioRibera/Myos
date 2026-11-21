//! Catalog and food-facts files attached read-only.
#![expect(clippy::unwrap_used, reason = "tests: a panic is the failure report")]

use std::path::{Path, PathBuf};

use myos_core::code::Code;
use myos_core::diet::{Nutrient, Nutrients};
use myos_core::exercise::{ExerciseLookup, Modality};
use myos_core::id::Id;
use myos_core::muscle::{Muscle, MuscleRole};
use myos_store::catalog::{CATALOG_SCHEMA, FOOD_FACTS_SCHEMA, Reference};
use myos_store::{DatabaseKey, Error, Store};
use rusqlite::{Connection, params};
use uuid::Uuid;

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        // A space and a `#` check the URI escaping.
        let dir = std::env::temp_dir().join(format!("myos catalog #{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _removed = std::fs::remove_dir_all(&self.0);
    }
}

const BENCH: Uuid = Uuid::from_u128(0x5a7e_0000_0000_5000_8000_0000_0000_0001);
const SQUAT: Uuid = Uuid::from_u128(0x5a7e_0000_0000_5000_8000_0000_0000_0002);
const OATS: Uuid = Uuid::from_u128(0x5a7e_0000_0000_5000_8000_0000_0000_0003);

fn build_catalog(path: &Path, version: u32) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(CATALOG_SCHEMA).unwrap();
    conn.execute(
        "INSERT INTO catalog_meta (key, value) VALUES ('schema_version', '1'), ('version', ?1)",
        [version.to_string()],
    )
    .unwrap();
    for (id, slug, status, en, es) in [
        (BENCH, "bench-press", 1, "Bench press", "Press de banca"),
        (SQUAT, "back-squat", 2, "Back squat", "Sentadilla"),
    ] {
        conn.execute(
            "INSERT INTO exercise (id, slug, modality, status) VALUES (?1, ?2, ?3, ?4)",
            params![
                id.as_bytes().as_slice(),
                slug,
                Modality::Strength.code(),
                status
            ],
        )
        .unwrap();
        for (lang, name) in [("en", en), ("es", es)] {
            conn.execute(
                "INSERT INTO exercise_name (exercise_id, lang, name) VALUES (?1, ?2, ?3)",
                params![id.as_bytes().as_slice(), lang, name],
            )
            .unwrap();
        }
    }
    conn.execute(
        "INSERT INTO exercise_target (exercise_id, muscle, role) VALUES (?1, ?2, ?3)",
        params![
            BENCH.as_bytes().as_slice(),
            Muscle::Chest.code(),
            MuscleRole::Primary.code()
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO food (id) VALUES (?1)",
        [OATS.as_bytes().as_slice()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO food_name (food_id, lang, name, aliases) VALUES (?1, 'es', 'Avena', 'copos')",
        [OATS.as_bytes().as_slice()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO food_nutrient (food_id, nutrient, amount_100g) VALUES (?1, ?2, 13000)",
        params![OATS.as_bytes().as_slice(), Nutrient::Protein.code()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO food_portion (food_id, position, lang, label, grams_mg)
         VALUES (?1, 1, 'es', '1 taza', 80000)",
        [OATS.as_bytes().as_slice()],
    )
    .unwrap();
    conn.execute_batch(
        "INSERT INTO exercise_search (exercise_search) VALUES ('rebuild');
         INSERT INTO food_search (food_search) VALUES ('rebuild');",
    )
    .unwrap();
}

fn build_food_facts(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(FOOD_FACTS_SCHEMA).unwrap();
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('schema_version', '1'), ('version', '7')",
        [],
    )
    .unwrap();
    // Protein (code 2) 25 000 mg per 100 g as a nutrient TLV.
    let nutrients = vec![2_u8, 3, 0xa8, 0xc3, 0x01];
    conn.execute(
        "INSERT INTO product (barcode, name, brand, nutrients) VALUES ('7790001', 'Yogur griego', 'Tambo', ?1)",
        [nutrients],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO product_search (product_search) VALUES ('rebuild')",
        [],
    )
    .unwrap();
}

fn store() -> Store {
    Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap()
}

#[test]
fn catalog_exercises_and_foods_are_found() {
    let dir = Dir::new();
    let path = dir.path("catalog.db");
    build_catalog(&path, 3);
    let mut store = store();
    assert!(store.catalog().unwrap().is_empty(), "nothing attached yet");
    assert_eq!(
        store
            .attach_reference(Reference::Catalog, &path)
            .unwrap()
            .version,
        3
    );

    let catalog = store.catalog().unwrap();
    assert_eq!((catalog.version, catalog.len()), (3, 2));
    let bench = catalog.exercise(Id::from(BENCH)).unwrap();
    assert_eq!(bench.targets[0].muscle, Muscle::Chest);
    // Retired exercises still resolve for history but are not searchable.
    assert!(catalog.exercise(Id::from(SQUAT)).is_some());
    assert!(
        store
            .search_exercises("sentad", "es", 10)
            .unwrap()
            .is_empty()
    );

    let hits = store.search_exercises("pres banc", "es", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "Press de banca");
    // Diacritics folded, FTS syntax inert.
    assert_eq!(
        store
            .search_exercises("BENCH\" OR", "en", 10)
            .unwrap()
            .len(),
        0
    );
    assert_eq!(store.search_exercises("bénch", "en", 10).unwrap().len(), 1);

    let foods = store.search_foods("copos", "es", 10).unwrap();
    assert_eq!(foods[0].id, Id::from(OATS));
    let oats = store.catalog_food(Id::from(OATS), "es").unwrap().unwrap();
    assert_eq!(
        oats.density,
        Nutrients::default().with(Nutrient::Protein, 13_000)
    );
    assert_eq!(oats.portions[0].grams_mg, 80_000);
    assert!(store.catalog_food(Id::from(OATS), "en").unwrap().is_none());
}

#[test]
fn catalog_files_are_never_written_and_can_be_replaced() {
    let dir = Dir::new();
    let current = dir.path("catalog.db");
    let next = dir.path("catalog.next.db");
    build_catalog(&current, 1);
    build_catalog(&next, 2);
    let before = std::fs::read(&current).unwrap();

    let mut store = store();
    store
        .attach_reference(Reference::Catalog, &current)
        .unwrap();
    store.search_exercises("bench", "en", 5).unwrap();
    assert_eq!(std::fs::read(&current).unwrap(), before);

    // Cached statements on the attached file do not block the swap.
    let info = store.replace_catalog(&next, &current).unwrap();
    assert_eq!(info.version, 2);
    assert_eq!(store.catalog().unwrap().version, 2);
    assert!(!next.exists());

    store.detach_reference(Reference::Catalog).unwrap();
    assert!(store.catalog().unwrap().is_empty());
}

#[test]
fn files_of_another_schema_are_refused() {
    let dir = Dir::new();
    let path = dir.path("other.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(CATALOG_SCHEMA).unwrap();
    conn.execute(
        "INSERT INTO catalog_meta (key, value) VALUES ('schema_version', '99'), ('version', '1')",
        [],
    )
    .unwrap();
    drop(conn);
    let mut store = store();
    assert!(matches!(
        store.attach_reference(Reference::Catalog, &path),
        Err(Error::ReferenceUnreadable)
    ));
    assert!(store.catalog().unwrap().is_empty(), "left detached");
    assert!(matches!(
        store.attach_reference(Reference::Catalog, &dir.path("missing.db")),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn products_are_found_by_barcode_and_name() {
    let dir = Dir::new();
    let path = dir.path("off-ar.db");
    build_food_facts(&path);
    let mut store = store();
    assert!(store.product("7790001").unwrap().is_none());
    store.attach_reference(Reference::FoodFacts, &path).unwrap();
    let yogurt = store.product("7790001").unwrap().unwrap();
    assert_eq!(yogurt.brand.as_deref(), Some("Tambo"));
    assert_eq!(yogurt.density.get(Nutrient::Protein), Some(25_000));
    assert_eq!(store.search_products("griego", 5).unwrap()[0].id, "7790001");
    assert!(store.product("000").unwrap().is_none());
}

#[test]
fn english_names_answer_when_a_language_has_none() {
    let dir = Dir::new();
    let path = dir.path("catalog.db");
    build_catalog(&path, 1);
    let mut store = store();
    store.attach_reference(Reference::Catalog, &path).unwrap();
    // No German names: the English ones answer.
    let hits = store.search_exercises("bench", "de", 5).unwrap();
    assert_eq!(hits[0].name, "Bench press");
}
