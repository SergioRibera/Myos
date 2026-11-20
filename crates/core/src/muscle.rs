//! Muscle taxonomy.
//!
//! One entry per muscle region the anatomical map draws (`myos-uikit`
//! `BodyRegion` minus the silhouette-only regions). Subzones (muscle heads)
//! extend this once the map has their outlines.

/// A muscle region. Declaration order is a stable index into per-muscle
/// arrays ([`Muscle::index`]); storage uses [`Code`](crate::code::Code).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Muscle {
    /// Neck flexors and extensors.
    Neck,
    /// Trapezius.
    Trapezius,
    /// Front (anterior) deltoids.
    FrontDeltoids,
    /// Rear (posterior) deltoids.
    RearDeltoids,
    /// Pectorals.
    Chest,
    /// Biceps.
    Biceps,
    /// Triceps.
    Triceps,
    /// Forearms.
    Forearms,
    /// Rectus abdominis.
    Abs,
    /// Obliques.
    Obliques,
    /// Lats and rhomboids.
    UpperBack,
    /// Spinal erectors.
    LowerBack,
    /// Glutes.
    Glutes,
    /// Hip abductors.
    Abductors,
    /// Hip adductors.
    Adductors,
    /// Quadriceps.
    Quadriceps,
    /// Hamstrings.
    Hamstrings,
    /// Gastrocnemius.
    Calves,
    /// Soleus.
    Soleus,
}

impl Muscle {
    /// Every muscle, in index order.
    pub const ALL: [Self; 19] = [
        Self::Neck,
        Self::Trapezius,
        Self::FrontDeltoids,
        Self::RearDeltoids,
        Self::Chest,
        Self::Biceps,
        Self::Triceps,
        Self::Forearms,
        Self::Abs,
        Self::Obliques,
        Self::UpperBack,
        Self::LowerBack,
        Self::Glutes,
        Self::Abductors,
        Self::Adductors,
        Self::Quadriceps,
        Self::Hamstrings,
        Self::Calves,
        Self::Soleus,
    ];

    /// Number of muscles.
    pub const COUNT: usize = Self::ALL.len();

    /// Stable index into per-muscle arrays.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// How an exercise involves a muscle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MuscleRole {
    /// Prime mover.
    Primary,
    /// Assists the prime mover.
    Secondary,
    /// Holds position; does not accumulate volume by default.
    Stabilizer,
}

impl MuscleRole {
    /// Default share of a set credited to the muscle (metric v1).
    #[must_use]
    pub const fn default_weight(self) -> Share {
        match self {
            Self::Primary => Share::FULL,
            Self::Secondary => Share::HALF,
            Self::Stabilizer => Share::NONE,
        }
    }
}

/// A fraction in `0 ..= 1`, stored exactly as basis points (1/10 000).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Share {
    basis_points: u16,
}

impl Share {
    /// Nothing.
    pub const NONE: Self = Self { basis_points: 0 };
    /// One half.
    pub const HALF: Self = Self {
        basis_points: 5_000,
    };
    /// Everything.
    pub const FULL: Self = Self {
        basis_points: 10_000,
    };

    /// `None` above 10 000 basis points.
    #[must_use]
    pub const fn from_basis_points(basis_points: u16) -> Option<Self> {
        if basis_points <= 10_000 {
            Some(Self { basis_points })
        } else {
            None
        }
    }

    /// Basis points, for storage.
    #[must_use]
    pub const fn basis_points(self) -> u16 {
        self.basis_points
    }

    /// The fraction.
    #[must_use]
    pub fn get(self) -> f64 {
        f64::from(self.basis_points) / 10_000.0
    }
}
/// One muscle an exercise works, with its role and an optional per-exercise
/// weight that overrides the role default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MuscleTarget {
    /// The muscle.
    pub muscle: Muscle,
    /// Its role in the exercise.
    pub role: MuscleRole,
    /// Curated weight; `None` uses [`MuscleRole::default_weight`].
    pub weight: Option<Share>,
}

impl MuscleTarget {
    /// A target with the role's default weight.
    #[must_use]
    pub const fn new(muscle: Muscle, role: MuscleRole) -> Self {
        Self {
            muscle,
            role,
            weight: None,
        }
    }

    /// Override the role's default weight.
    #[must_use]
    pub const fn with_weight(mut self, weight: Share) -> Self {
        self.weight = Some(weight);
        self
    }

    /// Share of a set credited to the muscle.
    #[must_use]
    pub fn share(&self) -> Share {
        self.weight.unwrap_or_else(|| self.role.default_weight())
    }
}

// Muscle codes leave room per region for subzones (muscle heads), which get
// codes inside their parent's hundred block.
crate::codes!(Muscle {
    Neck = 100,
    Trapezius = 200,
    FrontDeltoids = 300,
    RearDeltoids = 400,
    Chest = 500,
    Biceps = 600,
    Triceps = 700,
    Forearms = 800,
    Abs = 900,
    Obliques = 1000,
    UpperBack = 1100,
    LowerBack = 1200,
    Glutes = 1300,
    Abductors = 1400,
    Adductors = 1500,
    Quadriceps = 1600,
    Hamstrings = 1700,
    Calves = 1800,
    Soleus = 1900,
});

crate::codes!(MuscleRole {
    Primary = 1,
    Secondary = 2,
    Stabilizer = 3,
});
