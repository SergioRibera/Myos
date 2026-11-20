//! Profile, preferences, goals and body measurements.
#![expect(clippy::unwrap_used, reason = "tests: a panic is the failure report")]

use jiff::Timestamp;
use jiff::civil::date;
use myos_core::body::{
    BodyMeasurement, MeasurementError, MeasurementKind, MeasurementMethod, MeasurementSource,
};
use myos_core::profile::{
    Goal, GoalPeriod, Level, MassUnit, Preferences, Profile, ProfileError, Sex, Theme,
};
use myos_core::session::Side;
use myos_core::units::Length;
use myos_core::volume::DaySpan;
use myos_store::{DatabaseKey, Entity, Error, Store};

fn store() -> Store {
    Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap()
}

#[test]
fn profile_defaults_then_round_trips() {
    let mut store = store();
    assert_eq!(store.profile().unwrap(), Profile::default());
    let profile = Profile {
        birth_year: Some(1994),
        sex: Some(Sex::Female),
        height: Some(Length::from_millimeters(1_680)),
    };
    let today = date(2026, 11, 20);
    store.set_profile(&profile, today).unwrap();
    assert_eq!(store.profile().unwrap(), profile);

    let edited = Profile {
        height: Some(Length::from_millimeters(1_690)),
        ..profile
    };
    store.set_profile(&edited, today).unwrap();
    assert_eq!(store.profile().unwrap(), edited);
    let pending = store.pending_outbox(10).unwrap();
    assert_eq!(pending.len(), 1, "singleton row: one outbox entry");
    assert_eq!(pending[0].entity, Some(Entity::Profile));
}

#[test]
fn profile_enforces_minimum_age_and_height() {
    let mut store = store();
    let today = date(2026, 11, 20);
    let young = Profile {
        birth_year: Some(2012),
        ..Profile::default()
    };
    assert!(matches!(
        store.set_profile(&young, today),
        Err(Error::Profile(ProfileError::TooYoung))
    ));
    let tall = Profile {
        height: Some(Length::from_millimeters(3_000)),
        ..Profile::default()
    };
    assert!(matches!(
        store.set_profile(&tall, today),
        Err(Error::OutOfRange("height"))
    ));
    assert_eq!(store.profile().unwrap(), Profile::default());
}

#[test]
fn preferences_round_trip() {
    let mut store = store();
    assert_eq!(store.preferences().unwrap(), Preferences::default());
    let preferences = Preferences {
        theme: Theme::Dark,
        mass_unit: MassUnit::Pound,
        locale: Some("es-AR".into()),
        ..Preferences::default()
    };
    store.set_preferences(&preferences).unwrap();
    assert_eq!(store.preferences().unwrap(), preferences);
    let long = Preferences {
        locale: Some("x".repeat(36)),
        ..Preferences::default()
    };
    assert!(matches!(
        store.set_preferences(&long),
        Err(Error::OutOfRange("locale"))
    ));
}

#[test]
fn goals_apply_from_their_day_and_merge_per_day() {
    let mut store = store();
    assert_eq!(store.goal_on(date(2026, 11, 20)).unwrap(), None);
    let first = GoalPeriod {
        from: date(2026, 1, 1),
        goal: Goal::Hypertrophy,
        level: Level::Beginner,
    };
    let second = GoalPeriod {
        from: date(2026, 9, 1),
        goal: Goal::Strength,
        level: Level::Intermediate,
    };
    store.set_goal(first).unwrap();
    store.set_goal(second).unwrap();
    assert_eq!(store.goal_on(date(2026, 3, 1)).unwrap(), Some(first));
    assert_eq!(store.goal_on(date(2026, 11, 20)).unwrap(), Some(second));
    assert_eq!(store.goal_on(date(2025, 12, 31)).unwrap(), None);

    let corrected = GoalPeriod {
        level: Level::Advanced,
        ..second
    };
    store.set_goal(corrected).unwrap();
    assert_eq!(store.goal_on(date(2026, 11, 20)).unwrap(), Some(corrected));
    let goal_entries = store
        .pending_outbox(10)
        .unwrap()
        .into_iter()
        .filter(|entry| entry.entity == Some(Entity::ProfileGoal))
        .count();
    assert_eq!(goal_entries, 2, "same start day reuses its row");
}

fn weight(day: u8, grams: i64) -> BodyMeasurement {
    BodyMeasurement {
        kind: MeasurementKind::Weight,
        side: None,
        method: MeasurementMethod::Scale,
        value: grams,
        measured_at: Timestamp::from_second(1_795_000_000 + i64::from(day) * 86_400).unwrap(),
        day: date(2026, 11, i8::try_from(day).unwrap()),
        source: MeasurementSource::Manual,
        note: None,
    }
}

#[test]
fn measurements_record_query_and_delete() {
    let mut store = store();
    let monday = store.record_measurement(&weight(16, 82_400), -180).unwrap();
    store.record_measurement(&weight(18, 82_100), -180).unwrap();
    store.record_measurement(&weight(25, 81_900), -180).unwrap();
    let arm = BodyMeasurement {
        kind: MeasurementKind::Arm,
        side: Some(Side::Left),
        method: MeasurementMethod::Tape,
        value: 365,
        note: Some("relajado".into()),
        ..weight(17, 0)
    };
    store.record_measurement(&arm, -180).unwrap();

    let week = DaySpan::iso_week(date(2026, 11, 18));
    let weights = store.measurements(MeasurementKind::Weight, week).unwrap();
    let values: Vec<i64> = weights.measurements.iter().map(|(_, m)| m.value).collect();
    assert_eq!(values, vec![82_400, 82_100]);
    let arms = store.measurements(MeasurementKind::Arm, week).unwrap();
    assert_eq!(arms.measurements[0].1, arm);

    store.delete_measurement(monday).unwrap();
    let weights = store.measurements(MeasurementKind::Weight, week).unwrap();
    assert_eq!(weights.measurements.len(), 1);
    assert!(matches!(
        store.delete_measurement(monday),
        Err(Error::NotFound(_))
    ));

    let unsided = BodyMeasurement {
        side: None,
        ..arm.clone()
    };
    assert!(matches!(
        store.record_measurement(&unsided, 0),
        Err(Error::Measurement(MeasurementError::Side))
    ));
    let noted = BodyMeasurement {
        note: Some("x".repeat(501)),
        ..arm
    };
    assert!(matches!(
        store.record_measurement(&noted, 0),
        Err(Error::OutOfRange("note"))
    ));
}
