//! Training records, as logged.
//!
//! Sets keep the raw values only: every derived number (load, records, 1RM) is
//! recomputed from them, so a formula change never loses what the user
//! logged.

use std::time::Duration;

use jiff::Timestamp;
use jiff::civil::Date;

use crate::exercise::ExerciseId;
use crate::id::Id;
use crate::units::{Distance, HeartRate, Mass};

/// Identifier of a logged set.
pub type SetId = Id<SetRecord>;

/// Identifier of a [`Session`].
pub type SessionId = Id<Session>;

/// Reps in reserve: how many more reps the lifter could have done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rir(u8);

impl Rir {
    /// Taken to failure.
    pub const FAILURE: Self = Self(0);

    /// `None` above 10 (RIR beyond that is not a meaningful rating).
    #[must_use]
    pub const fn new(reps: u8) -> Option<Self> {
        if reps <= 10 { Some(Self(reps)) } else { None }
    }

    /// Reps left in the tank.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Rate of perceived exertion on the 1–10 scale, in half points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rpe {
    half_points: u8,
}

impl Rpe {
    /// `None` outside 1.0 ..= 10.0 or off the half-point grid.
    #[must_use]
    pub fn new(rpe: f64) -> Option<Self> {
        let doubled = rpe * 2.0;
        if !(2.0..=20.0).contains(&doubled) || doubled.fract() != 0.0 {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "whole number in 2 ..= 20, checked above"
        )]
        let half_points = doubled as u8;
        Some(Self { half_points })
    }

    /// The rating.
    #[must_use]
    pub fn get(self) -> f64 {
        f64::from(self.half_points) / 2.0
    }

    /// From half points (`2 ..= 20` for RPE 1 ..= 10).
    #[must_use]
    pub const fn from_half_points(half_points: u8) -> Option<Self> {
        if half_points >= 2 && half_points <= 20 {
            Some(Self { half_points })
        } else {
            None
        }
    }

    /// Half points, for storage.
    #[must_use]
    pub const fn half_points(self) -> u8 {
        self.half_points
    }
}

/// How hard a set was, in whichever scale the user logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effort {
    /// Reps in reserve.
    Rir(Rir),
    /// Perceived exertion.
    Rpe(Rpe),
}

impl Effort {
    /// Reps in reserve, converting RPE with the usual `RIR = 10 − RPE`.
    #[must_use]
    pub fn reps_in_reserve(self) -> f64 {
        match self {
            Self::Rir(rir) => f64::from(rir.get()),
            Self::Rpe(rpe) => 10.0 - rpe.get(),
        }
    }
}

/// What a set was for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SetKind {
    /// Warm-up: logged, not counted as training volume.
    Warmup,
    /// Regular effective set.
    Working,
    /// Taken to failure (implies RIR 0 when no effort is logged).
    Failure,
    /// Drop set following a heavier set.
    Drop,
}

/// Which side a set trained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Both sides at once (default).
    Both,
    /// Left side only (unilateral).
    Left,
    /// Right side only (unilateral).
    Right,
}

/// One logged set. Fields a modality does not use stay `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetRecord {
    /// Exercise performed.
    pub exercise: ExerciseId,
    /// Purpose of the set.
    pub kind: SetKind,
    /// Side trained.
    pub side: Side,
    /// External load (barbell, dumbbell, ballast).
    pub load: Option<Mass>,
    /// Repetitions.
    pub reps: Option<u16>,
    /// Effort rating, if the user gave one.
    pub effort: Option<Effort>,
    /// Time under effort (holds, intervals, cardio).
    pub duration: Option<Duration>,
    /// Distance (cardio, swimming).
    pub distance: Option<Distance>,
    /// Average heart rate.
    pub heart_rate: Option<HeartRate>,
    /// Superset group within the session; sets sharing it alternate.
    pub superset: Option<u16>,
}

impl SetRecord {
    /// A working set of `exercise` with nothing else logged yet.
    #[must_use]
    pub const fn new(exercise: ExerciseId) -> Self {
        Self {
            exercise,
            kind: SetKind::Working,
            side: Side::Both,
            load: None,
            reps: None,
            effort: None,
            duration: None,
            distance: None,
            heart_rate: None,
            superset: None,
        }
    }

    /// Set the purpose.
    #[must_use]
    pub const fn with_kind(mut self, kind: SetKind) -> Self {
        self.kind = kind;
        self
    }

    /// Set the side.
    #[must_use]
    pub const fn with_side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    /// Set load and reps.
    #[must_use]
    pub const fn with_lift(mut self, load: Mass, reps: u16) -> Self {
        self.load = Some(load);
        self.reps = Some(reps);
        self
    }

    /// Set reps without external load.
    #[must_use]
    pub const fn with_reps(mut self, reps: u16) -> Self {
        self.reps = Some(reps);
        self
    }

    /// Set the effort rating.
    #[must_use]
    pub const fn with_effort(mut self, effort: Effort) -> Self {
        self.effort = Some(effort);
        self
    }

    /// Set the duration.
    #[must_use]
    pub const fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Set the distance.
    #[must_use]
    pub const fn with_distance(mut self, distance: Distance) -> Self {
        self.distance = Some(distance);
        self
    }

    /// Set the average heart rate.
    #[must_use]
    pub const fn with_heart_rate(mut self, heart_rate: HeartRate) -> Self {
        self.heart_rate = Some(heart_rate);
        self
    }

    /// Put the set in a superset group.
    #[must_use]
    pub const fn in_superset(mut self, group: u16) -> Self {
        self.superset = Some(group);
        self
    }
}

/// A training session: sets in the order they were done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Stable id.
    pub id: SessionId,
    /// When the session started.
    pub started_at: Timestamp,
    /// Training day as the user saw it on their clock. Weekly totals group
    /// by this, so travelling never moves a session to another day.
    pub day: Date,
    /// Sets, in order.
    pub sets: Vec<SetRecord>,
}

crate::codes!(SetKind {
    Warmup = 1,
    Working = 2,
    Failure = 3,
    Drop = 4,
});

crate::codes!(Side {
    Both = 1,
    Left = 2,
    Right = 3,
});

/// How a session was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionOrigin {
    /// Logged by hand.
    Manual,
    /// Started from a routine.
    Routine,
    /// Imported from a file (CSV export of another app, spreadsheet).
    Import,
    /// Imported from a wearable or health platform.
    Wearable,
}

crate::codes!(SessionOrigin {
    Manual = 1,
    Routine = 2,
    Import = 3,
    Wearable = 4,
});
