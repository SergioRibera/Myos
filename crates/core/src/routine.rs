//! Routines: templates of days, blocks and exercise targets.
//!
//! A routine's content is immutable once saved as a version; editing saves a
//! new version, so two devices (or a professional and a client) never merge
//! half-edits of the same plan.

use std::time::Duration;

use crate::exercise::ExerciseId;
use crate::id::Id;
use crate::session::{Rir, Rpe};
use crate::units::Mass;

/// Identifier of a routine (its head, stable across versions).
pub type RoutineId = Id<Routine>;

/// Identifier of one saved version of a routine.
pub type RoutineVersionId = Id<RoutineContent>;

/// Identifier of an item inside a routine, stable across versions so logged
/// sets can point at what they fulfilled.
pub type RoutineItemId = Id<RoutineItem>;

/// Marker for routine ids.
#[derive(Debug, Clone, Copy)]
pub struct Routine(std::convert::Infallible);

/// How a routine relates to where it came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FollowMode {
    /// The user's own routine.
    #[default]
    Own,
    /// A copy: upstream changes are not offered.
    Copy,
    /// Follows the upstream: new upstream versions are offered.
    Follow,
}

crate::codes!(FollowMode {
    Own = 1,
    Copy = 2,
    Follow = 3,
});

/// How sets of a block are performed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BlockKind {
    /// One exercise at a time.
    #[default]
    Single,
    /// Exercises alternate set by set.
    Superset,
    /// Exercises run back to back as rounds.
    Circuit,
}

crate::codes!(BlockKind {
    Single = 1,
    Superset = 2,
    Circuit = 3,
});

/// How an item progresses between sessions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Progression {
    /// No automatic progression.
    #[default]
    None,
    /// Add reps up to the top of the range, then load.
    DoubleProgression,
    /// Add a fixed load each session.
    LinearLoad,
    /// Adjust load to hit the effort target.
    EffortBased,
}

crate::codes!(Progression {
    None = 1,
    DoubleProgression = 2,
    LinearLoad = 3,
    EffortBased = 4,
});

/// Effort target of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffortTarget {
    /// Reps in reserve.
    Rir(Rir),
    /// Perceived exertion.
    Rpe(Rpe),
}

/// One exercise with its targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineItem {
    /// Stable id across versions.
    pub id: RoutineItemId,
    /// Exercise.
    pub exercise: ExerciseId,
    /// Working sets.
    pub sets: u16,
    /// Rep range (`min ..= max`); equal bounds for a fixed count.
    pub reps: Option<(u16, u16)>,
    /// Effort target.
    pub effort: Option<EffortTarget>,
    /// Rest after each set.
    pub rest: Option<Duration>,
    /// Starting load.
    pub load: Option<Mass>,
    /// Progression rule.
    pub progression: Progression,
    /// Load step for load-based progressions.
    pub step: Option<Mass>,
}

/// Exercises performed together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutineBlock {
    /// How they are performed.
    pub kind: BlockKind,
    /// Rest after the block (round).
    pub rest: Option<Duration>,
    /// Items, in order.
    pub items: Vec<RoutineItem>,
}

/// One training day of a routine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutineDay {
    /// Day name ("Empuje", "Piernas").
    pub name: String,
    /// Blocks, in order.
    pub blocks: Vec<RoutineBlock>,
}

/// The content of one routine version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutineContent {
    /// Days, in order.
    pub days: Vec<RoutineDay>,
}

impl RoutineContent {
    /// Every item, in order.
    pub fn items(&self) -> impl Iterator<Item = &RoutineItem> + '_ {
        self.days
            .iter()
            .flat_map(|day| &day.blocks)
            .flat_map(|block| &block.items)
    }
}
