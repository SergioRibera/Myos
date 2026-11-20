//! Estimated one-rep max.

use crate::session::{Effort, SetKind, SetRecord};
use crate::units::Mass;

/// Formula used for an estimate. Stored next to every derived value so
/// history can be recomputed when the formula changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OneRepMaxFormula {
    /// Epley with reps in reserve added to the reps done:
    /// `1RM = load × (1 + (reps + RIR) / 30)`, and `load` itself when
    /// `reps + RIR = 1`.
    EpleyRirV1,
}

/// An estimated one-rep max and how it was obtained.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OneRepMax {
    /// Estimated maximal load for a single rep.
    pub kg: f64,
    /// Formula used.
    pub formula: OneRepMaxFormula,
    /// Whether RIR was assumed (0) because the set had no effort rating.
    pub effort_assumed: bool,
}

impl OneRepMax {
    /// Reps-to-failure past which Epley stops being informative.
    pub const MAX_REPS_TO_FAILURE: f64 = 15.0;

    /// Estimate from one set; `None` for warm-ups, sets without external
    /// load or reps, and sets too far from failure to say anything.
    ///
    /// A missing effort rating assumes RIR 0, which gives the lower (safer)
    /// estimate.
    #[must_use]
    pub fn estimate(set: &SetRecord) -> Option<Self> {
        if set.kind == SetKind::Warmup {
            return None;
        }
        let load = set.load.filter(|load| *load > Mass::ZERO)?;
        let reps = set.reps.filter(|reps| *reps > 0)?;
        let rir = set.effort.map_or(0.0, Effort::reps_in_reserve);
        let to_failure = f64::from(reps) + rir;
        if to_failure > Self::MAX_REPS_TO_FAILURE {
            return None;
        }
        let kg = if to_failure <= 1.0 {
            load.kg()
        } else {
            load.kg() * (1.0 + to_failure / 30.0)
        };
        Some(Self {
            kg,
            formula: OneRepMaxFormula::EpleyRirV1,
            effort_assumed: set.effort.is_none() && set.kind != SetKind::Failure,
        })
    }
}

crate::code::codes!(OneRepMaxFormula { EpleyRirV1 = 1 });
