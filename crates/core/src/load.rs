//! Weighted load: how much one set counts toward each muscle.
//!
//! Shown to the user as *set equivalents*: one equivalent is one set close
//! to failure (RIR ≤ 3), with the muscle as prime mover, at a reference
//! intensity. The model is versioned; every stored derivative carries the
//! [`LoadModel`] that produced it so history can be recomputed from raw sets.

use std::iter::Sum;
use std::ops::{Add, AddAssign, Mul};

use crate::exercise::{Exercise, Modality};
use crate::muscle::{Muscle, Share};
use crate::session::{SetKind, SetRecord};
use crate::strength::OneRepMax;

/// Load expressed in set equivalents.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
pub struct SetEquivalents(f64);

impl SetEquivalents {
    /// No load.
    pub const ZERO: Self = Self(0.0);

    /// The amount.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Mean of two amounts.
    #[must_use]
    pub const fn mean(self, other: Self) -> Self {
        Self(f64::midpoint(self.0, other.0))
    }
}

impl Add for SetEquivalents {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for SetEquivalents {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sum for SetEquivalents {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, Add::add)
    }
}

impl Mul<Share> for SetEquivalents {
    type Output = Self;

    fn mul(self, share: Share) -> Self {
        Self(self.0 * share.get())
    }
}

/// Version of the load formula.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LoadModel {
    /// `load = role × effort(RIR/RPE) × intensity(%1RM)`.
    /// See [`LoadModel::effort_factor`] and
    /// [`LoadModel::intensity_factor`].
    #[default]
    WeightedV1,
}

/// What was assumed while computing a set's load, so callers can mark it
/// as an estimate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Assumptions {
    /// No effort rating: the effort factor used the default.
    pub effort: bool,
    /// No reference 1RM: intensity was taken as neutral.
    pub intensity: bool,
}

/// Load of one set before it is split across muscles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetLoad {
    /// Equivalents credited to a prime mover with share 1.0.
    pub base: SetEquivalents,
    /// What was assumed.
    pub assumed: Assumptions,
    /// Formula version.
    pub model: LoadModel,
}

impl SetLoad {
    /// The set's load per muscle the exercise works (zero-share muscles
    /// skipped).
    pub fn per_muscle<'a>(
        &self,
        exercise: &'a Exercise,
    ) -> impl Iterator<Item = (Muscle, SetEquivalents)> + 'a {
        let base = self.base;
        exercise.targets.iter().filter_map(move |target| {
            let share = target.share();
            (share > Share::NONE).then(|| (target.muscle, base * share))
        })
    }
}

/// Why a set contributes no load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Skipped {
    /// Warm-ups are logged but never counted.
    Warmup,
    /// Cardio has no load model yet.
    Cardio,
    /// Swimming has no load model yet.
    Swimming,
}

impl LoadModel {
    /// Load of `set`, performed as `exercise`. `reference` is the
    /// exercise's current estimated 1RM, used for the intensity factor.
    pub fn set_load(
        self,
        set: &SetRecord,
        exercise: &Exercise,
        reference: Option<OneRepMax>,
    ) -> Result<SetLoad, Skipped> {
        match self {
            Self::WeightedV1 => Self::weighted_v1(set, exercise, reference),
        }
    }

    fn weighted_v1(
        set: &SetRecord,
        exercise: &Exercise,
        reference: Option<OneRepMax>,
    ) -> Result<SetLoad, Skipped> {
        match exercise.modality {
            Modality::Strength | Modality::Calisthenics => {}
            Modality::Cardio => return Err(Skipped::Cardio),
            Modality::Swimming => return Err(Skipped::Swimming),
        }
        if set.kind == SetKind::Warmup {
            return Err(Skipped::Warmup);
        }

        let (effort, effort_assumed) = Self::effort_factor(set);
        let relative = set
            .load
            .zip(reference)
            .filter(|(_, reference)| reference.kg > 0.0)
            .map(|(load, reference)| load.kg() / reference.kg);
        let intensity = relative.map_or(1.0, Self::intensity_factor);

        Ok(SetLoad {
            base: SetEquivalents(effort * intensity),
            assumed: Assumptions {
                effort: effort_assumed,
                intensity: relative.is_none(),
            },
            model: Self::WeightedV1,
        })
    }

    /// v1 effort factor and whether it was assumed: RIR 0–3 → 1.0, RIR 4 →
    /// 0.7, RIR ≥ 5 → 0.4; no rating → 0.8 (assumed), except failure sets
    /// which are RIR 0 by definition.
    #[must_use]
    pub fn effort_factor(set: &SetRecord) -> (f64, bool) {
        let Some(effort) = set.effort else {
            return match set.kind {
                SetKind::Failure => (1.0, false),
                SetKind::Warmup | SetKind::Working | SetKind::Drop => (0.8, true),
            };
        };
        let rir = effort.reps_in_reserve();
        let factor = if rir <= 3.0 {
            1.0
        } else if rir <= 4.0 {
            0.7
        } else {
            0.4
        };
        (factor, false)
    }

    /// v1 intensity factor for a load at `relative` × 1RM: neutral (1.0)
    /// from 60 % up, falling linearly to 0.5 at 30 % and flat below.
    #[must_use]
    pub fn intensity_factor(relative: f64) -> f64 {
        const NEUTRAL_FROM: f64 = 0.60;
        const FLOOR_AT: f64 = 0.30;
        const FLOOR: f64 = 0.5;
        if relative >= NEUTRAL_FROM {
            1.0
        } else if relative <= FLOOR_AT {
            FLOOR
        } else {
            FLOOR + (1.0 - FLOOR) * (relative - FLOOR_AT) / (NEUTRAL_FROM - FLOOR_AT)
        }
    }
}

crate::code::codes!(LoadModel { WeightedV1 = 1 });
