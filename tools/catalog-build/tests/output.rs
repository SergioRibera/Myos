//! Smoke test of built reference files: the app's store must attach and
//! read them. Runs when `CATALOG_DIST` points at the build output (CI);
//! skipped otherwise.
#![expect(clippy::unwrap_used, reason = "tests: a panic is the failure report")]

use std::path::PathBuf;

use myos_core::diet::Nutrient;
use myos_core::exercise::ExerciseLookup;
use myos_store::catalog::Reference;
use myos_store::{DatabaseKey, Store};

#[test]
fn built_files_open_in_the_app_store() {
    let Some(dist) = std::env::var_os("CATALOG_DIST").map(PathBuf::from) else {
        eprintln!("skipping: CATALOG_DIST not set");
        return;
    };
    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    store
        .attach_reference(Reference::Catalog, &dist.join("catalog.db"))
        .unwrap();
    let catalog = store.catalog().unwrap();
    assert!(catalog.len() > 500, "{} exercises", catalog.len());

    let bench = store
        .search_exercises("barbell bench press", "en", 5)
        .unwrap();
    assert!(!bench.is_empty());
    let exercise = catalog.exercise(bench[0].id).unwrap();
    assert!(!exercise.targets.is_empty());

    let banana = store.search_foods("banana raw", "en", 5).unwrap();
    let food = store.catalog_food(banana[0].id, "en").unwrap().unwrap();
    let energy = food.density.get(Nutrient::Energy).unwrap();
    // Raw banana: about 89 kcal / 100 g.
    assert!((300_000..450_000).contains(&energy), "{energy} J");

    for entry in std::fs::read_dir(&dist).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("off-") && path.extension().is_some_and(|ext| ext == "db") {
            store.attach_reference(Reference::FoodFacts, &path).unwrap();
            store.search_products("a", 1).unwrap();
        }
    }
}
