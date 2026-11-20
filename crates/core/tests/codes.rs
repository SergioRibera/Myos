//! Storage codes are persisted in databases and sync payloads: changing one
//! corrupts existing data. These tests pin every code.

use myos_core::code::Code;
use myos_core::exercise::Modality;
use myos_core::load::LoadModel;
use myos_core::muscle::{Muscle, MuscleRole};
use myos_core::session::{SessionOrigin, SetKind, Side};
use myos_core::strength::OneRepMaxFormula;

fn pinned<T: Code + PartialEq + std::fmt::Debug>(table: &[(T, u16)]) {
    for &(variant, code) in table {
        assert_eq!(variant.code(), code, "{variant:?}");
        assert_eq!(T::from_code(code), Some(variant), "{code}");
    }
    assert_eq!(T::from_code(0), None, "0 is reserved for unspecified");
}

#[test]
fn modality_codes() {
    pinned(&[
        (Modality::Strength, 1),
        (Modality::Calisthenics, 2),
        (Modality::Cardio, 3),
        (Modality::Swimming, 4),
    ]);
}

#[test]
fn set_codes() {
    pinned(&[
        (SetKind::Warmup, 1),
        (SetKind::Working, 2),
        (SetKind::Failure, 3),
        (SetKind::Drop, 4),
    ]);
    pinned(&[(Side::Both, 1), (Side::Left, 2), (Side::Right, 3)]);
}

#[test]
fn formula_codes() {
    pinned(&[(LoadModel::WeightedV1, 1)]);
    pinned(&[(OneRepMaxFormula::EpleyRirV1, 1)]);
}

#[test]
fn muscle_codes() {
    pinned(&[
        (MuscleRole::Primary, 1),
        (MuscleRole::Secondary, 2),
        (MuscleRole::Stabilizer, 3),
    ]);
    let expected: Vec<u16> = (1..=19).map(|block| block * 100).collect();
    let actual: Vec<u16> = Muscle::ALL.iter().map(|muscle| muscle.code()).collect();
    assert_eq!(actual, expected);
    for muscle in Muscle::ALL {
        assert_eq!(Muscle::from_code(muscle.code()), Some(muscle));
    }
}

#[test]
fn session_origin_codes() {
    pinned(&[
        (SessionOrigin::Manual, 1),
        (SessionOrigin::Routine, 2),
        (SessionOrigin::Import, 3),
        (SessionOrigin::Wearable, 4),
    ]);
}

#[test]
fn profile_codes() {
    use myos_core::profile::{
        DietMode, DistanceUnit, EffortScale, EnergyUnit, Goal, Level, MassUnit, Sex, Theme,
    };
    pinned(&[(Sex::Female, 1), (Sex::Male, 2), (Sex::Other, 3)]);
    pinned(&[
        (Goal::Hypertrophy, 1),
        (Goal::Strength, 2),
        (Goal::Endurance, 3),
        (Goal::FatLoss, 4),
        (Goal::Health, 5),
        (Goal::Performance, 6),
    ]);
    pinned(&[
        (Level::Beginner, 1),
        (Level::Intermediate, 2),
        (Level::Advanced, 3),
    ]);
    pinned(&[(Theme::System, 1), (Theme::Light, 2), (Theme::Dark, 3)]);
    pinned(&[(MassUnit::Kilogram, 1), (MassUnit::Pound, 2)]);
    pinned(&[(DistanceUnit::Kilometer, 1), (DistanceUnit::Mile, 2)]);
    pinned(&[(EnergyUnit::Kilocalorie, 1), (EnergyUnit::Kilojoule, 2)]);
    pinned(&[(EffortScale::Rir, 1), (EffortScale::Rpe, 2)]);
    pinned(&[(DietMode::Quick, 1), (DietMode::Detailed, 2)]);
}

#[test]
fn body_codes() {
    use myos_core::body::{MeasurementKind as K, MeasurementMethod as M, MeasurementSource as S};
    pinned(&[
        (K::Weight, 1),
        (K::BodyFat, 2),
        (K::Waist, 3),
        (K::Hips, 4),
        (K::Chest, 5),
        (K::Neck, 6),
        (K::Arm, 7),
        (K::Forearm, 8),
        (K::Thigh, 9),
        (K::Calf, 10),
        (K::Skinfold, 11),
    ]);
    pinned(&[
        (M::Unspecified, 1),
        (M::Tape, 2),
        (M::Scale, 3),
        (M::Bioimpedance, 4),
        (M::Dexa, 5),
        (M::Caliper, 6),
    ]);
    pinned(&[
        (S::Manual, 1),
        (S::SmartScale, 2),
        (S::HealthKit, 3),
        (S::HealthConnect, 4),
        (S::Garmin, 5),
        (S::Import, 6),
    ]);
}
