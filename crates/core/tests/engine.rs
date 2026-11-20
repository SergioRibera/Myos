//! Weighted load v1, 1RM estimates and weekly per-muscle volume.
#![expect(clippy::unwrap_used, reason = "tests: a panic is the failure report")]

use std::time::Duration;

use jiff::Timestamp;
use jiff::civil::{Date, date};
use myos_core::exercise::{Exercise, ExerciseId, Modality};
use myos_core::id::Id;
use myos_core::load::{LoadModel, Skipped};
use myos_core::muscle::{Muscle, MuscleRole, MuscleTarget, Share};
use myos_core::session::{Effort, Rir, Rpe, Session, SetKind, SetRecord, Side};
use myos_core::strength::OneRepMax;
use myos_core::units::{Distance, Mass};
use myos_core::volume::{BestOneRepMax, DaySpan, NoReferences, SkipReason, VolumeReport};

const EPSILON: f64 = 1e-9;

fn approx(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < EPSILON,
        "expected {expected}, got {actual}"
    );
}

fn kg(value: f64) -> Mass {
    Mass::from_kg(value).unwrap()
}

const fn rir(reps: u8) -> Effort {
    Effort::Rir(Rir::new(reps).unwrap())
}

fn bench() -> Exercise {
    Exercise {
        id: Id::generate(),
        modality: Modality::Strength,
        targets: vec![
            MuscleTarget::new(Muscle::Chest, MuscleRole::Primary),
            MuscleTarget::new(Muscle::Triceps, MuscleRole::Secondary),
            MuscleTarget::new(Muscle::FrontDeltoids, MuscleRole::Secondary)
                .with_weight(Share::from_basis_points(3_000).unwrap()),
            MuscleTarget::new(Muscle::Abs, MuscleRole::Stabilizer),
        ],
    }
}

fn lunge() -> Exercise {
    Exercise {
        id: Id::generate(),
        modality: Modality::Strength,
        targets: vec![MuscleTarget::new(Muscle::Quadriceps, MuscleRole::Primary)],
    }
}

fn run() -> Exercise {
    Exercise {
        id: Id::generate(),
        modality: Modality::Cardio,
        targets: vec![MuscleTarget::new(Muscle::Quadriceps, MuscleRole::Primary)],
    }
}

fn session(day: Date, sets: Vec<SetRecord>) -> Session {
    Session {
        id: Id::generate(),
        started_at: Timestamp::UNIX_EPOCH,
        day,
        sets,
    }
}

#[test]
fn effort_factor_follows_v1_table() {
    let id: ExerciseId = Id::generate();
    let factor = |set: SetRecord| LoadModel::effort_factor(&set);
    assert_eq!(factor(SetRecord::new(id).with_effort(rir(0))), (1.0, false));
    assert_eq!(factor(SetRecord::new(id).with_effort(rir(3))), (1.0, false));
    assert_eq!(factor(SetRecord::new(id).with_effort(rir(4))), (0.7, false));
    assert_eq!(factor(SetRecord::new(id).with_effort(rir(6))), (0.4, false));
    // RPE 8 = RIR 2; RPE 6.5 = RIR 3.5.
    let rpe = |value| Effort::Rpe(Rpe::new(value).unwrap());
    assert_eq!(
        factor(SetRecord::new(id).with_effort(rpe(8.0))),
        (1.0, false)
    );
    assert_eq!(
        factor(SetRecord::new(id).with_effort(rpe(6.5))),
        (0.7, false)
    );
    // No rating: 0.8 and flagged, except failure sets.
    assert_eq!(factor(SetRecord::new(id)), (0.8, true));
    assert_eq!(
        factor(SetRecord::new(id).with_kind(SetKind::Failure)),
        (1.0, false)
    );
}

#[test]
fn intensity_factor_is_neutral_from_sixty_percent() {
    approx(LoadModel::intensity_factor(0.85), 1.0);
    approx(LoadModel::intensity_factor(0.60), 1.0);
    approx(LoadModel::intensity_factor(0.45), 0.75);
    approx(LoadModel::intensity_factor(0.30), 0.5);
    approx(LoadModel::intensity_factor(0.10), 0.5);
}

#[test]
fn rpe_rejects_off_grid_values() {
    assert!(Rpe::new(7.5).is_some());
    assert!(Rpe::new(7.3).is_none());
    assert!(Rpe::new(10.5).is_none());
    assert!(Rpe::new(f64::NAN).is_none());
}

#[test]
fn one_rep_max_adds_reps_in_reserve() {
    let id = Id::generate();
    let set = SetRecord::new(id)
        .with_lift(kg(100.0), 5)
        .with_effort(rir(1));
    let estimate = OneRepMax::estimate(&set).unwrap();
    approx(estimate.kg, 120.0);
    assert!(!estimate.effort_assumed);

    let single = SetRecord::new(id)
        .with_lift(kg(140.0), 1)
        .with_effort(rir(0));
    approx(OneRepMax::estimate(&single).unwrap().kg, 140.0);

    let warmup = SetRecord::new(id)
        .with_kind(SetKind::Warmup)
        .with_lift(kg(60.0), 5);
    assert!(OneRepMax::estimate(&warmup).is_none());

    let too_light = SetRecord::new(id)
        .with_lift(kg(20.0), 12)
        .with_effort(rir(8));
    assert!(OneRepMax::estimate(&too_light).is_none());
}

#[test]
fn set_load_splits_by_role_and_weight() {
    let bench = bench();
    let set = SetRecord::new(bench.id)
        .with_lift(kg(80.0), 8)
        .with_effort(rir(2));
    let load = LoadModel::WeightedV1.set_load(&set, &bench, None).unwrap();
    approx(load.base.get(), 1.0);
    assert!(load.assumed.intensity);
    assert!(!load.assumed.effort);

    let split: Vec<_> = load
        .per_muscle(&bench)
        .map(|(muscle, amount)| (muscle, amount.get()))
        .collect();
    // Stabilizer (abs) is dropped.
    assert_eq!(
        split,
        vec![
            (Muscle::Chest, 1.0),
            (Muscle::Triceps, 0.5),
            (Muscle::FrontDeltoids, 0.3),
        ]
    );
}

#[test]
fn light_set_counts_less_against_reference() {
    let bench = bench();
    let reference = OneRepMax::estimate(
        &SetRecord::new(bench.id)
            .with_lift(kg(100.0), 1)
            .with_effort(rir(0)),
    );
    let light = SetRecord::new(bench.id)
        .with_lift(kg(45.0), 20)
        .with_effort(rir(2));
    let load = LoadModel::WeightedV1
        .set_load(&light, &bench, reference)
        .unwrap();
    approx(load.base.get(), 0.75);
    assert!(!load.assumed.intensity);
}

#[test]
fn warmups_and_cardio_are_skipped() {
    let bench = bench();
    let run = run();
    let warmup = SetRecord::new(bench.id).with_kind(SetKind::Warmup);
    assert_eq!(
        LoadModel::WeightedV1.set_load(&warmup, &bench, None),
        Err(Skipped::Warmup)
    );
    let jog = SetRecord::new(run.id)
        .with_duration(Duration::from_secs(1_800))
        .with_distance(Distance::from_meters(5_000));
    assert_eq!(
        LoadModel::WeightedV1.set_load(&jog, &run, None),
        Err(Skipped::Cardio)
    );
}

#[test]
fn iso_week_runs_monday_to_sunday() {
    // 2026-11-20 is a Friday.
    let week = DaySpan::iso_week(date(2026, 11, 20));
    assert_eq!(week.first(), date(2026, 11, 16));
    assert_eq!(week.last(), date(2026, 11, 22));
    assert!(week.contains(date(2026, 11, 22)));
    assert!(!week.contains(date(2026, 11, 23)));
    assert!(DaySpan::new(date(2026, 11, 2), date(2026, 11, 1)).is_none());
}

#[test]
fn weekly_report_sums_sides_and_flags_assumptions() {
    let bench = bench();
    let lunge = lunge();
    let run = run();
    let catalog = vec![bench.clone(), lunge.clone(), run.clone()];
    let unknown: ExerciseId = Id::generate();

    let week = DaySpan::iso_week(date(2026, 11, 20));
    let sessions = vec![
        session(
            date(2026, 11, 16),
            vec![
                SetRecord::new(bench.id)
                    .with_kind(SetKind::Warmup)
                    .with_lift(kg(40.0), 10),
                SetRecord::new(bench.id)
                    .with_lift(kg(80.0), 8)
                    .with_effort(rir(2)),
                SetRecord::new(bench.id).with_lift(kg(80.0), 8),
            ],
        ),
        session(
            date(2026, 11, 18),
            vec![
                SetRecord::new(lunge.id)
                    .with_side(Side::Left)
                    .with_lift(kg(20.0), 10)
                    .with_effort(rir(2)),
                SetRecord::new(lunge.id)
                    .with_side(Side::Right)
                    .with_lift(kg(20.0), 10)
                    .with_effort(rir(2)),
                SetRecord::new(run.id).with_duration(Duration::from_secs(1_200)),
                SetRecord::new(unknown).with_reps(10),
            ],
        ),
        // Outside the week.
        session(
            date(2026, 11, 23),
            vec![
                SetRecord::new(bench.id)
                    .with_lift(kg(80.0), 8)
                    .with_effort(rir(2)),
            ],
        ),
    ];

    let report = VolumeReport::compute(
        week,
        LoadModel::WeightedV1,
        &sessions,
        catalog.as_slice(),
        &NoReferences,
    );

    // 1.0 (rated) + 0.8 (assumed effort) on the chest.
    approx(report.volume.get(Muscle::Chest).total().get(), 1.8);
    approx(report.volume.get(Muscle::Triceps).total().get(), 0.9);
    // One set per leg = one set equivalent for the muscle.
    let quads = report.volume.get(Muscle::Quadriceps);
    approx(quads.left.get(), 1.0);
    approx(quads.right.get(), 1.0);
    approx(quads.total().get(), 1.0);
    approx(report.volume.get(Muscle::Abs).total().get(), 0.0);

    assert_eq!(report.counted_sets, 4);
    assert_eq!(report.effort_assumed_sets, 1);
    assert_eq!(report.intensity_unknown_sets, 4);
    assert_eq!(report.skipped[&SkipReason::Load(Skipped::Warmup)], 1);
    assert_eq!(report.skipped[&SkipReason::Load(Skipped::Cardio)], 1);
    assert_eq!(report.skipped[&SkipReason::UnknownExercise], 1);
}

#[test]
fn best_one_rep_max_takes_the_heaviest_estimate() {
    let bench = bench();
    let history = vec![session(
        date(2026, 11, 9),
        vec![
            SetRecord::new(bench.id)
                .with_lift(kg(80.0), 8)
                .with_effort(rir(2)),
            SetRecord::new(bench.id)
                .with_lift(kg(100.0), 3)
                .with_effort(rir(1)),
        ],
    )];
    let references = BestOneRepMax::from_sessions(&history);
    let week = DaySpan::iso_week(date(2026, 11, 20));
    let this_week = vec![session(
        date(2026, 11, 17),
        vec![
            SetRecord::new(bench.id)
                .with_lift(kg(50.0), 15)
                .with_effort(rir(2)),
        ],
    )];
    let report = VolumeReport::compute(
        week,
        LoadModel::WeightedV1,
        &this_week,
        std::slice::from_ref(&bench),
        &references,
    );
    // Best e1RM: 100 × (1 + 4/30) ≈ 113.3; 50 kg ≈ 44 % → factor ≈ 0.74.
    let expected = LoadModel::intensity_factor(50.0 / (100.0 * (1.0 + 4.0 / 30.0)));
    approx(report.volume.get(Muscle::Chest).total().get(), expected);
    assert_eq!(report.intensity_unknown_sets, 0);
}
