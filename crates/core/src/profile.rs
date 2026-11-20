//! Who trains: profile, goal and app preferences.

use jiff::civil::Date;

use crate::units::Length;

/// Minimum age to use the app.
pub const MIN_AGE: i16 = 16;

/// Sex, used only to pick reference ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sex {
    /// Female.
    Female,
    /// Male.
    Male,
    /// Other or prefers not to say.
    Other,
}

crate::codes!(Sex {
    Female = 1,
    Male = 2,
    Other = 3,
});

/// What the user trains for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Goal {
    /// Muscle size.
    Hypertrophy,
    /// Maximal strength.
    Strength,
    /// Endurance.
    Endurance,
    /// Losing fat.
    FatLoss,
    /// General health.
    Health,
    /// Sport performance.
    Performance,
}

crate::codes!(Goal {
    Hypertrophy = 1,
    Strength = 2,
    Endurance = 3,
    FatLoss = 4,
    Health = 5,
    Performance = 6,
});

/// Training experience.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Less than about a year of consistent training.
    Beginner,
    /// One to a few years.
    Intermediate,
    /// Many years, close to genetic potential.
    Advanced,
}

crate::codes!(Level {
    Beginner = 1,
    Intermediate = 2,
    Advanced = 3,
});

/// Why a profile is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileError {
    /// Younger than [`MIN_AGE`] in the current year.
    TooYoung,
    /// Birth year in the future or implausibly old.
    BirthYear,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooYoung => write!(f, "users must be at least {MIN_AGE}"),
            Self::BirthYear => f.write_str("birth year is not plausible"),
        }
    }
}

impl std::error::Error for ProfileError {}

/// Body facts that rarely change. Only the birth year is kept, never the
/// full date.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Profile {
    /// Year of birth.
    pub birth_year: Option<i16>,
    /// Sex.
    pub sex: Option<Sex>,
    /// Height.
    pub height: Option<Length>,
}

impl Profile {
    /// Check the profile on `today`. Age counts by year: a user who turns
    /// [`MIN_AGE`] this year is accepted (onboarding also asks them to
    /// confirm their age).
    pub fn validate(&self, today: Date) -> Result<(), ProfileError> {
        let Some(year) = self.birth_year else {
            return Ok(());
        };
        let age = i32::from(today.year()) - i32::from(year);
        if age > 120 || year < 1900 {
            return Err(ProfileError::BirthYear);
        }
        if age < i32::from(MIN_AGE) {
            return Err(ProfileError::TooYoung);
        }
        Ok(())
    }
}

/// Goal and level from a day on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoalPeriod {
    /// First day it applies.
    pub from: Date,
    /// Goal.
    pub goal: Goal,
    /// Level.
    pub level: Level,
}

/// Colour theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Theme {
    /// Follow the system.
    #[default]
    System,
    /// Light.
    Light,
    /// Dark.
    Dark,
}

crate::codes!(Theme {
    System = 1,
    Light = 2,
    Dark = 3,
});

/// Unit for masses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MassUnit {
    /// Kilograms.
    #[default]
    Kilogram,
    /// Pounds.
    Pound,
}

crate::codes!(MassUnit {
    Kilogram = 1,
    Pound = 2,
});

/// Unit for distances.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DistanceUnit {
    /// Kilometers.
    #[default]
    Kilometer,
    /// Miles.
    Mile,
}

crate::codes!(DistanceUnit {
    Kilometer = 1,
    Mile = 2,
});

/// Unit for food energy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum EnergyUnit {
    /// Kilocalories.
    #[default]
    Kilocalorie,
    /// Kilojoules.
    Kilojoule,
}

crate::codes!(EnergyUnit {
    Kilocalorie = 1,
    Kilojoule = 2,
});

/// Effort scale offered when logging sets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum EffortScale {
    /// Reps in reserve.
    #[default]
    Rir,
    /// Rate of perceived exertion.
    Rpe,
}

crate::codes!(EffortScale { Rir = 1, Rpe = 2 });

/// How meals are logged by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DietMode {
    /// Portion size and protein level.
    #[default]
    Quick,
    /// Foods and grams.
    Detailed,
}

crate::codes!(DietMode {
    Quick = 1,
    Detailed = 2,
});

/// App preferences.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preferences {
    /// Colour theme.
    pub theme: Theme,
    /// Mass unit.
    pub mass_unit: MassUnit,
    /// Distance unit.
    pub distance_unit: DistanceUnit,
    /// Energy unit.
    pub energy_unit: EnergyUnit,
    /// Effort scale.
    pub effort_scale: EffortScale,
    /// Default diet logging mode.
    pub diet_mode: DietMode,
    /// BCP 47 locale override; `None` follows the system.
    pub locale: Option<String>,
}
