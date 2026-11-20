//! Body measurements.

use jiff::Timestamp;
use jiff::civil::Date;

use crate::id::Id;
use crate::session::Side;

/// Identifier of a [`BodyMeasurement`].
pub type MeasurementId = Id<BodyMeasurement>;

/// Unit a measurement kind is stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeasurementUnit {
    /// Grams.
    Gram,
    /// Basis points (1/100 of a percent).
    BasisPoint,
    /// Millimeters.
    Millimeter,
    /// Tenths of a millimeter (skinfolds).
    TenthMillimeter,
}

/// What was measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeasurementKind {
    /// Body weight.
    Weight,
    /// Body fat percentage.
    BodyFat,
    /// Waist circumference.
    Waist,
    /// Hip circumference.
    Hips,
    /// Chest circumference.
    Chest,
    /// Neck circumference.
    Neck,
    /// Upper arm circumference (per side).
    Arm,
    /// Forearm circumference (per side).
    Forearm,
    /// Thigh circumference (per side).
    Thigh,
    /// Calf circumference (per side).
    Calf,
    /// Skinfold thickness.
    Skinfold,
}

crate::codes!(MeasurementKind {
    Weight = 1,
    BodyFat = 2,
    Waist = 3,
    Hips = 4,
    Chest = 5,
    Neck = 6,
    Arm = 7,
    Forearm = 8,
    Thigh = 9,
    Calf = 10,
    Skinfold = 11,
});

impl MeasurementKind {
    /// Unit of the stored value.
    #[must_use]
    pub const fn unit(self) -> MeasurementUnit {
        match self {
            Self::Weight => MeasurementUnit::Gram,
            Self::BodyFat => MeasurementUnit::BasisPoint,
            Self::Waist
            | Self::Hips
            | Self::Chest
            | Self::Neck
            | Self::Arm
            | Self::Forearm
            | Self::Thigh
            | Self::Calf => MeasurementUnit::Millimeter,
            Self::Skinfold => MeasurementUnit::TenthMillimeter,
        }
    }

    /// Whether the kind is measured per side.
    #[must_use]
    pub const fn lateral(self) -> bool {
        matches!(self, Self::Arm | Self::Forearm | Self::Thigh | Self::Calf)
    }

    /// Plausible stored values (inclusive).
    #[must_use]
    pub const fn range(self) -> (i64, i64) {
        match self {
            Self::Weight => (20_000, 400_000),
            Self::BodyFat => (100, 7_000),
            Self::Waist | Self::Hips | Self::Chest => (300, 2_500),
            Self::Neck => (200, 700),
            Self::Arm | Self::Thigh | Self::Calf => (100, 1_200),
            Self::Forearm => (100, 600),
            Self::Skinfold => (10, 1_000),
        }
    }
}

/// How a measurement was taken.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MeasurementMethod {
    /// Not stated.
    #[default]
    Unspecified,
    /// Tape measure.
    Tape,
    /// Scale.
    Scale,
    /// Bioelectrical impedance.
    Bioimpedance,
    /// DEXA scan.
    Dexa,
    /// Skinfold caliper.
    Caliper,
}

crate::codes!(MeasurementMethod {
    Unspecified = 1,
    Tape = 2,
    Scale = 3,
    Bioimpedance = 4,
    Dexa = 5,
    Caliper = 6,
});

/// Where a measurement came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MeasurementSource {
    /// Entered by hand.
    #[default]
    Manual,
    /// Connected scale.
    SmartScale,
    /// Apple Health.
    HealthKit,
    /// Android Health Connect.
    HealthConnect,
    /// Garmin.
    Garmin,
    /// File import.
    Import,
}

crate::codes!(MeasurementSource {
    Manual = 1,
    SmartScale = 2,
    HealthKit = 3,
    HealthConnect = 4,
    Garmin = 5,
    Import = 6,
});

/// Why a measurement is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MeasurementError {
    /// Value outside [`MeasurementKind::range`].
    OutOfRange,
    /// A lateral kind without a side, or a side on a non-lateral kind.
    Side,
}

impl std::fmt::Display for MeasurementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfRange => f.write_str("measurement value is not plausible"),
            Self::Side => f.write_str("measurement side does not match its kind"),
        }
    }
}

impl std::error::Error for MeasurementError {}

/// One body measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyMeasurement {
    /// What was measured.
    pub kind: MeasurementKind,
    /// Side for lateral kinds (`Left` / `Right`), `None` otherwise.
    pub side: Option<Side>,
    /// How it was taken.
    pub method: MeasurementMethod,
    /// Value in [`MeasurementKind::unit`].
    pub value: i64,
    /// When it was taken.
    pub measured_at: Timestamp,
    /// Day on the user's clock.
    pub day: Date,
    /// Origin.
    pub source: MeasurementSource,
    /// Free note.
    pub note: Option<String>,
}

impl BodyMeasurement {
    /// Check value range and side.
    pub fn validate(&self) -> Result<(), MeasurementError> {
        let (min, max) = self.kind.range();
        if !(min..=max).contains(&self.value) {
            return Err(MeasurementError::OutOfRange);
        }
        let side_ok = match self.side {
            None => !self.kind.lateral(),
            Some(Side::Left | Side::Right) => self.kind.lateral(),
            Some(Side::Both) => false,
        };
        if side_ok {
            Ok(())
        } else {
            Err(MeasurementError::Side)
        }
    }
}
