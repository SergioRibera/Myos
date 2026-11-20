//! Per-muscle, per-side load totals over a span of training days.

use std::collections::{BTreeMap, HashMap};

use jiff::ToSpan;
use jiff::civil::Date;

use crate::exercise::{ExerciseId, ExerciseLookup};
use crate::load::{LoadModel, SetEquivalents, Skipped};
use crate::muscle::Muscle;
use crate::session::{Session, Side};
use crate::strength::OneRepMax;

/// Inclusive span of training days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DaySpan {
    first: Date,
    last: Date,
}

impl DaySpan {
    /// Days `first ..= last`; `None` when `last` is before `first`.
    #[must_use]
    pub fn new(first: Date, last: Date) -> Option<Self> {
        (first <= last).then_some(Self { first, last })
    }

    /// The ISO week (Monday to Sunday) containing `day`.
    ///
    /// # Panics
    ///
    /// Never for dates the app produces: only within six days of
    /// [`Date::MIN`] / [`Date::MAX`].
    #[must_use]
    pub fn iso_week(day: Date) -> Self {
        let back = i64::from(day.weekday().to_monday_zero_offset());
        let first = day
            .checked_sub(back.days())
            .expect("invariant: training days are far from Date::MIN");
        let last = first
            .checked_add(6.days())
            .expect("invariant: training days are far from Date::MAX");
        Self { first, last }
    }

    /// First day.
    #[must_use]
    pub const fn first(self) -> Date {
        self.first
    }

    /// Last day (inclusive).
    #[must_use]
    pub const fn last(self) -> Date {
        self.last
    }

    /// Whether `day` falls in the span.
    #[must_use]
    pub fn contains(self, day: Date) -> bool {
        (self.first..=self.last).contains(&day)
    }
}

/// Load on the two sides of one muscle. Bilateral sets credit both sides,
/// unilateral sets only theirs.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SideVolume {
    /// Left side.
    pub left: SetEquivalents,
    /// Right side.
    pub right: SetEquivalents,
}

impl SideVolume {
    /// Volume of the muscle as a whole: the mean of both sides, so one left
    /// set plus one right set count as one set.
    #[must_use]
    pub const fn total(self) -> SetEquivalents {
        self.left.mean(self.right)
    }

    fn credit(&mut self, side: Side, amount: SetEquivalents) {
        match side {
            Side::Both => {
                self.left += amount;
                self.right += amount;
            }
            Side::Left => self.left += amount,
            Side::Right => self.right += amount,
        }
    }
}

/// Load per muscle and side.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MuscleVolume {
    by_muscle: [SideVolume; Muscle::COUNT],
}

impl MuscleVolume {
    /// Load on `muscle`.
    #[must_use]
    pub const fn get(&self, muscle: Muscle) -> SideVolume {
        self.by_muscle[muscle.index()]
    }

    /// Every muscle with its load, in [`Muscle::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = (Muscle, SideVolume)> + '_ {
        Muscle::ALL.into_iter().zip(self.by_muscle)
    }

    /// Replace the load on `muscle` (rebuilding a cached report).
    pub const fn set(&mut self, muscle: Muscle, volume: SideVolume) {
        self.by_muscle[muscle.index()] = volume;
    }

    fn credit(&mut self, muscle: Muscle, side: Side, amount: SetEquivalents) {
        self.by_muscle[muscle.index()].credit(side, amount);
    }
}

/// Where the engine finds an exercise's reference 1RM.
pub trait ReferenceLookup {
    /// Reference estimate for `exercise`, if any.
    fn reference(&self, exercise: ExerciseId) -> Option<OneRepMax>;
}

/// No reference: every intensity is taken as neutral (and flagged).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoReferences;

impl ReferenceLookup for NoReferences {
    fn reference(&self, _exercise: ExerciseId) -> Option<OneRepMax> {
        None
    }
}

/// Best estimated 1RM per exercise over some history.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BestOneRepMax {
    best: HashMap<ExerciseId, OneRepMax>,
}

impl BestOneRepMax {
    /// Best estimate per exercise over `sessions`. Pass the history before
    /// the span being measured so a session is never its own reference.
    #[must_use]
    pub fn from_sessions<'a>(sessions: impl IntoIterator<Item = &'a Session>) -> Self {
        let mut best: HashMap<ExerciseId, OneRepMax> = HashMap::new();
        let estimates = sessions.into_iter().flat_map(|session| {
            session
                .sets
                .iter()
                .filter_map(|set| Some((set.exercise, OneRepMax::estimate(set)?)))
        });
        for (exercise, estimate) in estimates {
            best.entry(exercise)
                .and_modify(|current| {
                    if estimate.kg > current.kg {
                        *current = estimate;
                    }
                })
                .or_insert(estimate);
        }
        Self { best }
    }
}

impl ReferenceLookup for BestOneRepMax {
    fn reference(&self, exercise: ExerciseId) -> Option<OneRepMax> {
        self.best.get(&exercise).copied()
    }
}

/// Load totals over a span, with what was left out or assumed.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeReport {
    /// Span measured.
    pub span: DaySpan,
    /// Formula used.
    pub model: LoadModel,
    /// Load per muscle and side.
    pub volume: MuscleVolume,
    /// Sets that contributed load.
    pub counted_sets: u32,
    /// Counted sets without an effort rating.
    pub effort_assumed_sets: u32,
    /// Counted sets without a reference 1RM (intensity taken as neutral).
    pub intensity_unknown_sets: u32,
    /// Sets left out, by reason.
    pub skipped: BTreeMap<SkipReason, u32>,
}

/// Why a set is missing from a [`VolumeReport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SkipReason {
    /// The load model left it out.
    Load(Skipped),
    /// The exercise is not in the catalog (deleted custom exercise, catalog
    /// not synced yet).
    UnknownExercise,
}

impl VolumeReport {
    /// Total load of the sessions whose training day falls in `span`.
    #[must_use]
    pub fn compute<'a>(
        span: DaySpan,
        model: LoadModel,
        sessions: impl IntoIterator<Item = &'a Session>,
        exercises: &(impl ExerciseLookup + ?Sized),
        references: &impl ReferenceLookup,
    ) -> Self {
        let mut report = Self {
            span,
            model,
            volume: MuscleVolume::default(),
            counted_sets: 0,
            effort_assumed_sets: 0,
            intensity_unknown_sets: 0,
            skipped: BTreeMap::new(),
        };
        let sets = sessions
            .into_iter()
            .filter(|session| span.contains(session.day))
            .flat_map(|session| &session.sets);
        for set in sets {
            let Some(exercise) = exercises.exercise(set.exercise) else {
                report.skip(SkipReason::UnknownExercise);
                continue;
            };
            match model.set_load(set, exercise, references.reference(set.exercise)) {
                Ok(load) => {
                    report.counted_sets += 1;
                    report.effort_assumed_sets += u32::from(load.assumed.effort);
                    report.intensity_unknown_sets += u32::from(load.assumed.intensity);
                    for (muscle, amount) in load.per_muscle(exercise) {
                        report.volume.credit(muscle, set.side, amount);
                    }
                }
                Err(skipped) => report.skip(SkipReason::Load(skipped)),
            }
        }
        report
    }

    fn skip(&mut self, reason: SkipReason) {
        *self.skipped.entry(reason).or_default() += 1;
    }
}
