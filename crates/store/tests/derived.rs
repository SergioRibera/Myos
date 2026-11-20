//! Derived cache, user exercises and sync ordering.
#![expect(clippy::unwrap_used, reason = "tests: a panic is the failure report")]

use std::collections::HashMap;

use jiff::Timestamp;
use jiff::civil::{Date, date};
use myos_core::exercise::{Exercise, ExerciseId, Modality};
use myos_core::id::Id;
use myos_core::load::LoadModel;
use myos_core::muscle::{Muscle, MuscleRole, MuscleTarget};
use myos_core::routine::RoutineContent;
use myos_core::session::{Effort, Rir, SessionOrigin, SetRecord};
use myos_core::units::Mass;
use myos_core::volume::{DaySpan, VolumeReport};
use myos_store::{DatabaseKey, Entity, SessionDraft, Store};

fn store() -> Store {
    Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap()
}

fn catalog_bench() -> Exercise {
    Exercise {
        id: Id::generate(),
        modality: Modality::Strength,
        targets: vec![
            MuscleTarget::new(Muscle::Chest, MuscleRole::Primary),
            MuscleTarget::new(Muscle::Triceps, MuscleRole::Secondary),
        ],
    }
}

fn catalog(exercises: &[Exercise]) -> HashMap<ExerciseId, Exercise> {
    exercises
        .iter()
        .map(|exercise| (exercise.id, exercise.clone()))
        .collect()
}

fn train(store: &mut Store, day: Date, exercise: ExerciseId, kg: f64, reps: u16) {
    let session = store
        .start_session(SessionDraft {
            started_at: Timestamp::from_second(1_790_000_000).unwrap(),
            day,
            tz_offset_min: 0,
            origin: SessionOrigin::Manual,
        })
        .unwrap();
    store
        .log_set(
            session,
            &SetRecord::new(exercise)
                .with_lift(Mass::from_kg(kg).unwrap(), reps)
                .with_effort(Effort::Rir(Rir::new(2).unwrap())),
        )
        .unwrap();
}

fn computed(store: &Store, day: Date, exercises: &HashMap<ExerciseId, Exercise>) -> VolumeReport {
    // Reference: the engine over the raw history, best 1RM before the week.
    let week = DaySpan::iso_week(day);
    let before = DaySpan::new(date(2000, 1, 1), week.first().yesterday().unwrap()).unwrap();
    let history = store.sessions_in(before).unwrap().sessions;
    let references = myos_core::volume::BestOneRepMax::from_sessions(&history);
    let sessions = store.sessions_in(week).unwrap().sessions;
    VolumeReport::compute(
        week,
        LoadModel::WeightedV1,
        &sessions,
        exercises,
        &references,
    )
}

#[test]
fn cached_weeks_match_the_engine_and_refresh_incrementally() {
    let mut store = store();
    let bench = catalog_bench();
    let exercises = catalog(std::slice::from_ref(&bench));
    train(&mut store, date(2026, 11, 3), bench.id, 100.0, 5);
    train(&mut store, date(2026, 11, 10), bench.id, 50.0, 15);
    train(&mut store, date(2026, 11, 18), bench.id, 80.0, 8);

    let stats = store.refresh_derived(&exercises, 1).unwrap();
    assert!(stats.full);
    assert_eq!(stats.weeks, 3);
    for day in [date(2026, 11, 3), date(2026, 11, 10), date(2026, 11, 18)] {
        let cached = store.cached_week(day).unwrap().unwrap();
        let expected = computed(&store, day, &exercises);
        for muscle in Muscle::ALL {
            let (a, b) = (
                cached.volume.get(muscle).total().get(),
                expected.volume.get(muscle).total().get(),
            );
            assert!((a - b).abs() < 1e-3, "{day} {muscle:?}: {a} vs {b}");
        }
        assert_eq!(cached.counted_sets, expected.counted_sets);
        assert_eq!(
            cached.intensity_unknown_sets,
            expected.intensity_unknown_sets
        );
    }
    // Nothing changed: nothing to do.
    assert_eq!(
        store.refresh_derived(&exercises, 1).unwrap(),
        myos_store::RefreshStats::default()
    );

    // A new set in the last week recomputes from that week only.
    train(&mut store, date(2026, 11, 19), bench.id, 82.5, 8);
    let stats = store.refresh_derived(&exercises, 1).unwrap();
    assert!(!stats.full);
    assert_eq!((stats.days, stats.weeks), (1, 1));
    let week = store.cached_week(date(2026, 11, 18)).unwrap().unwrap();
    assert_eq!(week.counted_sets, 2);

    // A heavier old set raises later intensity references: later weeks too.
    train(&mut store, date(2026, 11, 4), bench.id, 140.0, 1);
    let stats = store.refresh_derived(&exercises, 1).unwrap();
    assert_eq!(stats.weeks, 3);
    let best = store.best_one_rep_max(bench.id).unwrap().unwrap();
    // 140 kg × 1 at RIR 2: Epley over 3 reps to failure.
    let expected = 140.0 * 1.1;
    assert!((best.kg - expected).abs() < 1e-9);

    // A new catalog version rebuilds everything.
    assert!(store.refresh_derived(&exercises, 2).unwrap().full);
    assert_eq!(store.cached_week(date(2025, 1, 1)).unwrap(), None);
}

#[test]
fn editing_a_user_exercise_recomputes_its_weeks() {
    let mut store = store();
    let exercise = store
        .create_custom_exercise(
            "Remo en polea",
            Modality::Strength,
            &[MuscleTarget::new(Muscle::UpperBack, MuscleRole::Primary)],
        )
        .unwrap();
    train(&mut store, date(2026, 11, 17), exercise, 60.0, 10);
    let empty = HashMap::new();
    store.refresh_derived(&empty, 1).unwrap();
    let week = store.cached_week(date(2026, 11, 17)).unwrap().unwrap();
    assert!(week.volume.get(Muscle::UpperBack).total().get() > 0.9);

    store
        .update_custom_exercise(
            exercise,
            "Remo en polea",
            Modality::Strength,
            &[MuscleTarget::new(Muscle::Biceps, MuscleRole::Primary)],
        )
        .unwrap();
    let stats = store.refresh_derived(&empty, 1).unwrap();
    assert_eq!(stats.weeks, 1);
    let week = store.cached_week(date(2026, 11, 17)).unwrap().unwrap();
    assert!(week.volume.get(Muscle::UpperBack).total().get() < 1e-9);
    assert!(week.volume.get(Muscle::Biceps).total().get() > 0.9);

    let listed = store.custom_exercises(false).unwrap();
    assert_eq!(listed[0].exercise.targets[0].muscle, Muscle::Biceps);
    store.delete_custom_exercise(exercise).unwrap();
    assert!(store.custom_exercises(false).unwrap().is_empty());
    assert_eq!(store.custom_exercises(true).unwrap().len(), 1);
}

#[test]
fn derived_cache_survives_reopen_and_rebuilds_when_unreadable() {
    let dir = std::env::temp_dir().join(format!("myos-derived-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("user.db");
    let key = DatabaseKey::generate().unwrap();
    let bench = catalog_bench();
    let exercises = catalog(std::slice::from_ref(&bench));
    {
        let mut store = Store::open(&path, &key).unwrap();
        train(&mut store, date(2026, 11, 18), bench.id, 80.0, 8);
        store.refresh_derived(&exercises, 1).unwrap();
    }
    {
        let mut store = Store::open(&path, &key).unwrap();
        assert!(store.cached_week(date(2026, 11, 18)).unwrap().is_some());
        assert!(!store.refresh_derived(&exercises, 1).unwrap().full);
    }
    // Garbage in the cache file: dropped and rebuilt, user data untouched.
    std::fs::write(dir.join("user.derived.db"), b"not a database").unwrap();
    let mut store = Store::open(&path, &key).unwrap();
    assert_eq!(store.cached_week(date(2026, 11, 18)).unwrap(), None);
    assert!(store.refresh_derived(&exercises, 1).unwrap().full);
    assert!(store.cached_week(date(2026, 11, 18)).unwrap().is_some());
    drop(store);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn routine_heads_queue_after_their_new_version() {
    let mut store = store();
    let routine = store
        .create_routine("Full body", &RoutineContent::default())
        .unwrap();
    store
        .save_routine_version(routine, &RoutineContent::default())
        .unwrap();
    let order: Vec<Entity> = store
        .pending_outbox(10)
        .unwrap()
        .into_iter()
        .filter_map(|entry| entry.entity)
        .collect();
    assert_eq!(
        order,
        vec![
            Entity::RoutineVersion,
            Entity::RoutineVersion,
            Entity::Routine
        ]
    );
}
