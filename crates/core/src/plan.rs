//! Periodization, calibration and suggestions.

use std::time::Duration;

use jiff::Timestamp;
use jiff::civil::Date;

use crate::exercise::ExerciseId;
use crate::id::Id;
use crate::load::LoadModel;
use crate::muscle::{Muscle, Share};
use crate::session::{Rir, SetId};
use crate::units::{Distance, Mass};

/// A training phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhaseKind {
    /// Guided tests to find real working loads.
    Calibration,
    /// Volume build-up.
    Accumulation,
    /// Reduced volume to recover.
    Deload,
    /// Heavier, lower volume.
    Intensification,
    /// Taper before an event.
    Taper,
}

crate::codes!(PhaseKind {
    Calibration = 1,
    Accumulation = 2,
    Deload = 3,
    Intensification = 4,
    Taper = 5,
});

/// Multiplier in basis points, `0 ..= 30_000` (0 × to 3 ×).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Multiplier(u16);

impl Multiplier {
    /// No change.
    pub const NEUTRAL: Self = Self(10_000);

    /// `None` above 3 ×.
    #[must_use]
    pub const fn from_basis_points(basis_points: u16) -> Option<Self> {
        if basis_points <= 30_000 {
            Some(Self(basis_points))
        } else {
            None
        }
    }

    /// Basis points.
    #[must_use]
    pub const fn basis_points(self) -> u16 {
        self.0
    }
}

impl Default for Multiplier {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

/// Identifier of a [`Phase`].
pub type PhaseId = Id<Phase>;

/// A phase of the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Phase {
    /// First day.
    pub starts_on: Date,
    /// Last day (inclusive).
    pub ends_on: Date,
    /// Kind.
    pub kind: PhaseKind,
    /// Volume multiplier on reference ranges.
    pub volume: Multiplier,
    /// Intensity multiplier.
    pub intensity: Multiplier,
}

/// What an event is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// Combat sports bout.
    Fight,
    /// Race.
    Race,
    /// Strength or physique competition.
    Competition,
    /// Test or tryout.
    Test,
    /// Anything else.
    Other,
}

crate::codes!(EventKind {
    Fight = 1,
    Race = 2,
    Competition = 3,
    Test = 4,
    Other = 5,
});

/// How much an event matters for the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventPriority {
    /// Train through it.
    Low,
    /// Short taper.
    Medium,
    /// Peak for it.
    High,
}

crate::codes!(EventPriority {
    Low = 1,
    Medium = 2,
    High = 3,
});

/// Identifier of an [`Event`].
pub type EventId = Id<Event>;

/// A dated event the plan prepares for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Day of the event.
    pub on: Date,
    /// Kind.
    pub kind: EventKind,
    /// Priority.
    pub priority: EventPriority,
    /// Name.
    pub name: String,
}

/// Test protocol of a calibration result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CalibrationProtocol {
    /// Working load found by reps at a declared RIR.
    WorkingLoad,
    /// Maximal reps at a fixed load or bodyweight.
    RepsTest,
    /// Timed endurance test.
    Endurance,
    /// Mobility test.
    Mobility,
}

crate::codes!(CalibrationProtocol {
    WorkingLoad = 1,
    RepsTest = 2,
    Endurance = 3,
    Mobility = 4,
});

/// What a calibration test measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CalibrationMeasure {
    /// A load.
    Load(Mass),
    /// A rep count.
    Reps(u16),
    /// A time.
    Time(Duration),
    /// A distance.
    Distance(Distance),
}

/// Identifier of a [`CalibrationResult`].
pub type CalibrationId = Id<CalibrationResult>;

/// A calibration result, always marked as an estimate in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalibrationResult {
    /// Exercise tested.
    pub exercise: ExerciseId,
    /// Day of the test.
    pub day: Date,
    /// Protocol.
    pub protocol: CalibrationProtocol,
    /// Value measured.
    pub measure: CalibrationMeasure,
    /// Reps in reserve the user declared.
    pub rir: Option<Rir>,
    /// Set the test came from, if logged.
    pub set: Option<SetId>,
}

/// What a suggestion is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SuggestionKind {
    /// Too little weekly load.
    Deficit,
    /// Too much weekly load.
    Excess,
    /// Opposing groups out of balance.
    Imbalance,
    /// Time to deload.
    Deload,
    /// An exercise to try.
    Exercise,
}

crate::codes!(SuggestionKind {
    Deficit = 1,
    Excess = 2,
    Imbalance = 3,
    Deload = 4,
    Exercise = 5,
});

/// Identifier of a [`Suggestion`].
pub type SuggestionId = Id<Suggestion>;

/// A suggestion with what produced it (the base of the future dataset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// When it was produced.
    pub created_at: Timestamp,
    /// Rule code.
    pub rule: u16,
    /// Rule version.
    pub rule_version: u16,
    /// Load model the numbers came from.
    pub model: LoadModel,
    /// Muscle concerned, if any.
    pub muscle: Option<Muscle>,
    /// Kind.
    pub kind: SuggestionKind,
    /// Confidence.
    pub confidence: Share,
    /// Encoded inputs (TLV) used to produce it.
    pub inputs: Vec<u8>,
}

/// How the user answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Response {
    /// Followed it.
    Accepted,
    /// Dismissed it.
    Ignored,
    /// Said it does not apply.
    NotApplicable,
}

crate::codes!(Response {
    Accepted = 1,
    Ignored = 2,
    NotApplicable = 3,
});

/// Why a suggestion does not apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResponseReason {
    /// Injury or condition.
    Injury,
    /// Trained outside the app.
    TrainedElsewhere,
    /// Not the user's goal.
    NotMyGoal,
    /// Something else.
    Other,
}

crate::codes!(ResponseReason {
    Injury = 1,
    TrainedElsewhere = 2,
    NotMyGoal = 3,
    Other = 4,
});

/// The user's answer to a suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Answer {
    /// Response.
    pub response: Response,
    /// Reason for `NotApplicable`.
    pub reason: Option<ResponseReason>,
    /// When it was given.
    pub at: Timestamp,
}
