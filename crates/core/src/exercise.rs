//! Exercises: what is trained and which muscles it works.
//!
//! Only what the load engine needs: id, modality and muscle targets.

use std::collections::HashMap;
use std::hash::BuildHasher;

use crate::id::Id;
use crate::muscle::MuscleTarget;

/// Identifier of an [`Exercise`]. History always points at Myos ids, never
/// at a source catalog's.
pub type ExerciseId = Id<Exercise>;

/// How an exercise is measured, which decides how its load is computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modality {
    /// Sets of reps with external load (weights, machines).
    Strength,
    /// Sets of reps or holds with bodyweight, optionally ballasted.
    Calisthenics,
    /// Continuous effort measured in time / distance (run, bike, row).
    Cardio,
    /// Swimming, measured in distance per stroke.
    Swimming,
}

/// An exercise as the load engine sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Exercise {
    /// Stable Myos id.
    pub id: ExerciseId,
    /// How it is measured.
    pub modality: Modality,
    /// Muscles it works, with role and weight.
    pub targets: Vec<MuscleTarget>,
}

/// Where the engine looks exercises up (bundled catalog, user and
/// organization custom exercises, or all of them layered).
pub trait ExerciseLookup {
    /// The exercise with this id, if known.
    fn exercise(&self, id: ExerciseId) -> Option<&Exercise>;
}

impl<S: BuildHasher> ExerciseLookup for HashMap<ExerciseId, Exercise, S> {
    fn exercise(&self, id: ExerciseId) -> Option<&Exercise> {
        self.get(&id)
    }
}

impl ExerciseLookup for [Exercise] {
    fn exercise(&self, id: ExerciseId) -> Option<&Exercise> {
        self.iter().find(|exercise| exercise.id == id)
    }
}
