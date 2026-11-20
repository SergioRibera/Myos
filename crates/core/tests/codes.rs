//! Storage codes are persisted in databases and sync payloads: changing one
//! corrupts existing data. These tests pin every code.

use myos_core::code::Code;
use myos_core::exercise::Modality;
use myos_core::load::LoadModel;
use myos_core::muscle::{Muscle, MuscleRole};
use myos_core::session::{SetKind, Side};
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
