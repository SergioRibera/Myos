//! Physical quantities. Everything is stored metric; display units are a
//! preference of the UI.

use std::fmt;

/// A mass, stored as whole grams so plate math (`22.5 kg`) stays exact.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mass {
    grams: u32,
}

impl Mass {
    /// Zero mass (bodyweight-only sets carry no external load).
    pub const ZERO: Self = Self { grams: 0 };

    /// A mass of `grams` grams.
    #[must_use]
    pub const fn from_grams(grams: u32) -> Self {
        Self { grams }
    }

    /// The mass closest to `kg` kilograms; `None` for negative, non-finite
    /// or absurd (> 4 000 kg) values.
    #[must_use]
    pub fn from_kg(kg: f64) -> Option<Self> {
        if !kg.is_finite() || !(0.0..=4_000.0).contains(&kg) {
            return None;
        }
        // In range: 0 ..= 4e6 grams fits in u32 and is non-negative.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "range checked above"
        )]
        let grams = (kg * 1_000.0).round() as u32;
        Some(Self { grams })
    }

    /// Whole grams.
    #[must_use]
    pub const fn grams(self) -> u32 {
        self.grams
    }

    /// Kilograms, for arithmetic.
    #[must_use]
    pub fn kg(self) -> f64 {
        f64::from(self.grams) / 1_000.0
    }
}

impl fmt::Display for Mass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} kg", self.kg())
    }
}

impl From<Mass> for f64 {
    fn from(mass: Mass) -> Self {
        mass.kg()
    }
}

/// A distance, stored as whole meters (cardio and swimming).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Distance {
    meters: u32,
}

impl Distance {
    /// A distance of `meters` meters.
    #[must_use]
    pub const fn from_meters(meters: u32) -> Self {
        Self { meters }
    }

    /// Whole meters.
    #[must_use]
    pub const fn meters(self) -> u32 {
        self.meters
    }
}

/// Average heart rate in beats per minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HeartRate {
    bpm: u16,
}

impl HeartRate {
    /// `None` outside the physiological range 25 ..= 250 bpm.
    #[must_use]
    pub const fn from_bpm(bpm: u16) -> Option<Self> {
        if bpm >= 25 && bpm <= 250 {
            Some(Self { bpm })
        } else {
            None
        }
    }

    /// Beats per minute.
    #[must_use]
    pub const fn bpm(self) -> u16 {
        self.bpm
    }
}

/// A body length (height, circumference), stored as whole millimeters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Length {
    millimeters: u32,
}

impl Length {
    /// A length of `millimeters` millimeters.
    #[must_use]
    pub const fn from_millimeters(millimeters: u32) -> Self {
        Self { millimeters }
    }

    /// Whole millimeters.
    #[must_use]
    pub const fn millimeters(self) -> u32 {
        self.millimeters
    }
}
