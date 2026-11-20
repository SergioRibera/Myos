//! Profile and body measurement validation.

use jiff::Timestamp;
use jiff::civil::date;
use myos_core::body::{
    BodyMeasurement, MeasurementError, MeasurementKind, MeasurementMethod, MeasurementSource,
};
use myos_core::profile::{Profile, ProfileError};
use myos_core::session::Side;

#[test]
fn age_gate_counts_by_year() {
    let today = date(2026, 11, 20);
    let with_year = |birth_year| Profile {
        birth_year: Some(birth_year),
        ..Profile::default()
    };
    assert_eq!(with_year(2010).validate(today), Ok(()));
    assert_eq!(with_year(2011).validate(today), Err(ProfileError::TooYoung));
    assert_eq!(
        with_year(1880).validate(today),
        Err(ProfileError::BirthYear)
    );
    assert_eq!(with_year(2030).validate(today), Err(ProfileError::TooYoung));
    assert_eq!(Profile::default().validate(today), Ok(()));
}

const fn measurement(kind: MeasurementKind, side: Option<Side>, value: i64) -> BodyMeasurement {
    BodyMeasurement {
        kind,
        side,
        method: MeasurementMethod::Tape,
        value,
        measured_at: Timestamp::UNIX_EPOCH,
        day: date(2026, 11, 20),
        source: MeasurementSource::Manual,
        note: None,
    }
}

#[test]
fn measurements_check_range_and_side() {
    assert_eq!(
        measurement(MeasurementKind::Weight, None, 82_500).validate(),
        Ok(())
    );
    assert_eq!(
        measurement(MeasurementKind::Weight, None, 5).validate(),
        Err(MeasurementError::OutOfRange)
    );
    assert_eq!(
        measurement(MeasurementKind::Arm, Some(Side::Left), 380).validate(),
        Ok(())
    );
    assert_eq!(
        measurement(MeasurementKind::Arm, None, 380).validate(),
        Err(MeasurementError::Side)
    );
    assert_eq!(
        measurement(MeasurementKind::Waist, Some(Side::Right), 800).validate(),
        Err(MeasurementError::Side)
    );
    assert_eq!(
        measurement(MeasurementKind::Arm, Some(Side::Both), 380).validate(),
        Err(MeasurementError::Side)
    );
}

#[test]
fn extreme_birth_years_do_not_overflow() {
    let today = date(2026, 11, 20);
    for year in [i16::MIN, i16::MAX, 2101] {
        let profile = Profile {
            birth_year: Some(year),
            ..Profile::default()
        };
        assert!(profile.validate(today).is_err(), "{year}");
    }
}
