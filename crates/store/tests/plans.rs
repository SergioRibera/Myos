//! Diet, routines, plan phases, events, calibration and suggestions.
#![expect(clippy::unwrap_used, reason = "tests: a panic is the failure report")]

use std::time::Duration;

use jiff::Timestamp;
use jiff::civil::date;
use myos_core::diet::{
    CustomFood, FoodRef, JOULES_PER_KCAL, Meal, MealContent, MealItem, MealSlot, Nutrient,
    Nutrients, Portion, PortionSize, ProteinLevel,
};
use myos_core::id::Id;
use myos_core::load::LoadModel;
use myos_core::muscle::{Muscle, Share};
use myos_core::plan::{
    Answer, CalibrationMeasure, CalibrationProtocol, CalibrationResult, Event, EventKind,
    EventPriority, Multiplier, Phase, PhaseKind, Response, ResponseReason, Suggestion,
    SuggestionKind,
};
use myos_core::routine::{
    BlockKind, EffortTarget, FollowMode, Progression, RoutineBlock, RoutineContent, RoutineDay,
    RoutineItem, RoutineItemId,
};
use myos_core::session::Rir;
use myos_core::units::Mass;
use myos_core::volume::DaySpan;
use myos_store::{DatabaseKey, Error, Store};

fn store() -> Store {
    Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap()
}

fn item(name: &str, grams_mg: u32, protein_mg: u64) -> MealItem {
    MealItem {
        food: FoodRef::Catalog(Id::generate()),
        grams_mg,
        name: name.into(),
        brand: None,
        density: Nutrients::default()
            .with(Nutrient::Energy, 150 * JOULES_PER_KCAL)
            .with(Nutrient::Protein, protein_mg),
    }
}

fn meal(day: i8, content: MealContent) -> Meal {
    Meal {
        eaten_at: Timestamp::from_second(1_795_000_000 + i64::from(day) * 86_400).unwrap(),
        day: date(2026, 11, day),
        slot: Some(MealSlot::Lunch),
        content,
        note: None,
    }
}

#[test]
fn meals_round_trip_and_replace_items() {
    let mut store = store();
    let barcode = MealItem {
        food: FoodRef::OpenFoodFacts("7790070012345".into()),
        brand: Some("Marca".into()),
        ..item("Yogur", 190_000, 3_500)
    };
    let detailed = meal(
        17,
        MealContent::Detailed(vec![item("Arroz", 150_000, 2_700), barcode]),
    );
    let quick = meal(
        18,
        MealContent::Quick {
            size: PortionSize::Large,
            protein: ProteinLevel::High,
        },
    );
    let detailed_id = store.log_meal(&detailed, -180).unwrap();
    store.log_meal(&quick, -180).unwrap();

    let week = DaySpan::iso_week(date(2026, 11, 18));
    let read = store.meals_in(week).unwrap();
    assert_eq!(read.undecodable, 0);
    assert_eq!(read.meals.len(), 2);
    assert_eq!(read.meals[0].1, detailed);
    assert_eq!(read.meals[1].1, quick);

    let replaced = meal(
        17,
        MealContent::Detailed(vec![item("Pollo", 120_000, 31_000)]),
    );
    store.replace_meal(detailed_id, &replaced, -180).unwrap();
    let read = store.meals_in(week).unwrap();
    assert_eq!(read.meals[0].1, replaced);
    assert_eq!(
        read.meals[0].1.totals().unwrap().get(Nutrient::Protein),
        Some(37_200)
    );

    store.delete_meal(detailed_id).unwrap();
    assert_eq!(store.meals_in(week).unwrap().meals.len(), 1);
    assert!(matches!(
        store.replace_meal(detailed_id, &replaced, 0),
        Err(Error::NotFound(_))
    ));

    let bad = meal(17, MealContent::Detailed(vec![item("", 1, 0)]));
    assert!(matches!(store.log_meal(&bad, 0), Err(Error::OutOfRange(_))));
}

#[test]
fn custom_foods_round_trip() {
    let mut store = store();
    let food = CustomFood {
        name: "Empanada de carne".into(),
        brand: None,
        barcode: None,
        density: Nutrients::default().with(Nutrient::Energy, 280 * JOULES_PER_KCAL),
        portions: vec![Portion {
            label: "1 unidad".into(),
            grams_mg: 90_000,
        }],
    };
    let id = store.create_custom_food(&food).unwrap();
    assert_eq!(store.custom_food(id).unwrap(), food);
    store.delete_custom_food(id).unwrap();
    assert!(matches!(store.custom_food(id), Err(Error::NotFound(_))));
}

fn routine_content(sets: u16) -> RoutineContent {
    RoutineContent {
        days: vec![RoutineDay {
            name: "Empuje".into(),
            blocks: vec![RoutineBlock {
                kind: BlockKind::Superset,
                rest: Some(Duration::from_secs(90)),
                items: vec![RoutineItem {
                    id: RoutineItemId::generate(),
                    exercise: Id::generate(),
                    sets,
                    reps: Some((8, 12)),
                    effort: Rir::new(2).map(EffortTarget::Rir),
                    rest: Some(Duration::from_secs(120)),
                    load: Mass::from_kg(50.0),
                    progression: Progression::DoubleProgression,
                    step: Mass::from_kg(2.5),
                }],
            }],
        }],
    }
}

#[test]
fn routines_version_copy_and_follow() {
    let mut store = store();
    let first = routine_content(3);
    let id = store.create_routine("Push Pull Legs", &first).unwrap();
    let stored = store.routine(id).unwrap();
    assert_eq!((stored.number, stored.follow), (1, FollowMode::Own));
    assert_eq!(stored.content, first);

    let second = routine_content(4);
    let v2 = store.save_routine_version(id, &second).unwrap();
    let stored = store.routine(id).unwrap();
    assert_eq!((stored.number, stored.version), (2, v2));
    assert_eq!(stored.content, second);
    assert_eq!(store.routine_version(v2).unwrap(), second);

    let followed = store
        .copy_routine(id, FollowMode::Follow, "PPL (entrenador)")
        .unwrap();
    let copy = store.routine(followed).unwrap();
    assert_eq!(copy.upstream, Some(id));
    assert_eq!(copy.follow, FollowMode::Follow);
    assert_eq!(copy.content, second);

    store.rename_routine(id, "PPL").unwrap();
    let names: Vec<String> = store
        .routines()
        .unwrap()
        .into_iter()
        .map(|routine| routine.name)
        .collect();
    assert_eq!(names, vec!["PPL", "PPL (entrenador)"]);

    store.delete_routine(id).unwrap();
    assert!(matches!(store.routine(id), Err(Error::NotFound(_))));
    // Versions outlive the head so logged sets keep resolving.
    assert_eq!(store.routine_version(v2).unwrap(), second);

    let mut duplicate = routine_content(3);
    let item = duplicate.days[0].blocks[0].items[0].clone();
    duplicate.days[0].blocks[0].items.push(item);
    assert!(matches!(
        store.create_routine("x", &duplicate),
        Err(Error::OutOfRange(_))
    ));
    assert!(matches!(
        store.create_routine("", &first),
        Err(Error::OutOfRange(_))
    ));
}

#[test]
fn phases_events_and_calibration() {
    let mut store = store();
    let phase = Phase {
        starts_on: date(2026, 11, 2),
        ends_on: date(2026, 11, 15),
        kind: PhaseKind::Calibration,
        volume: Multiplier::from_basis_points(8_000).unwrap(),
        intensity: Multiplier::NEUTRAL,
    };
    let phase_id = store.add_phase(phase).unwrap();
    let overlapping = DaySpan::new(date(2026, 11, 10), date(2026, 11, 20)).unwrap();
    assert_eq!(
        store.phases_in(overlapping).unwrap().rows,
        vec![(phase_id, phase)]
    );
    let later = DaySpan::iso_week(date(2026, 11, 18));
    assert!(store.phases_in(later).unwrap().rows.is_empty());
    store.delete_phase(phase_id).unwrap();
    assert!(store.phases_in(overlapping).unwrap().rows.is_empty());

    let event = Event {
        on: date(2026, 11, 21),
        kind: EventKind::Fight,
        priority: EventPriority::High,
        name: "Pelea amateur".into(),
    };
    let event_id = store.add_event(&event).unwrap();
    assert_eq!(
        store.events_in(later).unwrap().rows,
        vec![(event_id, event)]
    );

    let exercise = Id::generate();
    let result = CalibrationResult {
        exercise,
        day: date(2026, 11, 3),
        protocol: CalibrationProtocol::WorkingLoad,
        measure: CalibrationMeasure::Load(Mass::from_kg(62.5).unwrap()),
        rir: Rir::new(3),
        set: None,
    };
    let calibration = store.record_calibration(&result).unwrap();
    assert_eq!(
        store.calibrations_of(exercise).unwrap().rows,
        vec![(calibration, result)]
    );
}

#[test]
fn suggestions_keep_every_answer_and_show_the_latest() {
    let mut store = store();
    let suggestion = Suggestion {
        created_at: Timestamp::from_second(1_795_000_000).unwrap(),
        rule: 1,
        rule_version: 1,
        model: LoadModel::WeightedV1,
        muscle: Some(Muscle::Hamstrings),
        kind: SuggestionKind::Deficit,
        confidence: Share::from_basis_points(7_500).unwrap(),
        inputs: vec![1, 2, 3],
    };
    let id = store.record_suggestion(&suggestion).unwrap();
    let since = Timestamp::from_second(1_794_000_000).unwrap();
    let read = store.suggestions_since(since).unwrap();
    assert_eq!(read.rows, vec![(id, suggestion.clone(), None)]);

    let ignored = Answer {
        response: Response::Ignored,
        reason: None,
        at: Timestamp::from_second(1_795_000_100).unwrap(),
    };
    let not_applicable = Answer {
        response: Response::NotApplicable,
        reason: Some(ResponseReason::TrainedElsewhere),
        at: Timestamp::from_second(1_795_000_200).unwrap(),
    };
    store.answer_suggestion(id, ignored).unwrap();
    store.answer_suggestion(id, not_applicable).unwrap();
    let read = store.suggestions_since(since).unwrap();
    assert_eq!(read.rows[0].2, Some(not_applicable));
    assert!(matches!(
        store.answer_suggestion(Id::generate(), ignored),
        Err(Error::NotFound(_))
    ));
}
