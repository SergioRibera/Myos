//! Encrypted store: opening, training log, outbox and engine reads.
#![expect(clippy::unwrap_used, reason = "tests: a panic is the failure report")]

use std::path::PathBuf;

use jiff::Timestamp;
use jiff::civil::date;
use myos_core::exercise::{Exercise, Modality};
use myos_core::id::Id;
use myos_core::load::LoadModel;
use myos_core::muscle::{Muscle, MuscleRole, MuscleTarget};
use myos_core::session::{Effort, Rir, Rpe, SessionOrigin, SetKind, SetRecord, Side};
use myos_core::units::{HeartRate, Mass};
use myos_core::volume::{DaySpan, NoReferences, VolumeReport};
use myos_store::{DatabaseKey, Entity, Error, Op, SessionDraft, Store};

/// A database file removed when the test ends.
struct TempDb(PathBuf);

impl TempDb {
    fn new() -> Self {
        let name = format!("myos-store-{}.db", uuid::Uuid::now_v7());
        Self(std::env::temp_dir().join(name))
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.0.clone().into_os_string();
            path.push(suffix);
            let _removed = std::fs::remove_file(path);
        }
    }
}

fn draft() -> SessionDraft {
    SessionDraft {
        started_at: Timestamp::from_second(1_795_000_000).unwrap(),
        day: date(2026, 11, 18),
        tz_offset_min: -240,
        origin: SessionOrigin::Manual,
    }
}

fn bench() -> Exercise {
    Exercise {
        id: Id::generate(),
        modality: Modality::Strength,
        targets: vec![MuscleTarget::new(Muscle::Chest, MuscleRole::Primary)],
    }
}

fn working_set(exercise: &Exercise, kg: f64) -> SetRecord {
    SetRecord::new(exercise.id)
        .with_lift(Mass::from_kg(kg).unwrap(), 8)
        .with_effort(Effort::Rir(Rir::new(2).unwrap()))
}

#[test]
fn file_is_encrypted_and_reopens_only_with_its_key() {
    let db = TempDb::new();
    let key = DatabaseKey::generate().unwrap();
    {
        let mut store = Store::open(&db.0, &key).unwrap();
        store.start_session(draft()).unwrap();
    }
    let header = std::fs::read(&db.0).unwrap();
    assert!(!header.starts_with(b"SQLite format 3"));

    let same = DatabaseKey::take(&mut key.as_bytes().clone());
    let store = Store::open(&db.0, &same).unwrap();
    let week = DaySpan::iso_week(date(2026, 11, 18));
    assert_eq!(store.sessions_in(week).unwrap().sessions.len(), 1);
    drop(store);

    let other = DatabaseKey::generate().unwrap();
    assert!(matches!(Store::open(&db.0, &other), Err(Error::WrongKey)));
}

#[test]
fn key_debug_is_redacted() {
    let key = DatabaseKey::take(&mut [7; 32]);
    assert_eq!(format!("{key:?}"), "DatabaseKey(<redacted>)");
}

#[test]
fn logged_sets_round_trip_in_order() {
    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    let bench = bench();
    let session = store.start_session(draft()).unwrap();
    let first = working_set(&bench, 80.0);
    let second = SetRecord::new(bench.id)
        .with_kind(SetKind::Failure)
        .with_side(Side::Left)
        .with_lift(Mass::from_kg(22.5).unwrap(), 12)
        .with_effort(Effort::Rpe(Rpe::new(9.5).unwrap()))
        .with_duration(std::time::Duration::from_millis(41_250))
        .with_heart_rate(HeartRate::from_bpm(150).unwrap())
        .in_superset(1);
    let first_id = store.log_set(session, &first).unwrap();
    store.log_set(session, &second).unwrap();

    let sets = store.sets_of(session).unwrap().sets;
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0].id, first_id);
    assert_eq!(sets[0].record, first);
    assert_eq!(sets[1].record, second);
}

#[test]
fn edits_and_deletes_respect_liveness() {
    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    let bench = bench();
    let session = store.start_session(draft()).unwrap();
    let set = store.log_set(session, &working_set(&bench, 80.0)).unwrap();

    let fixed = working_set(&bench, 82.5);
    store.update_set(set, &fixed).unwrap();
    assert_eq!(store.sets_of(session).unwrap().sets[0].record, fixed);

    store.delete_set(set).unwrap();
    assert!(store.sets_of(session).unwrap().sets.is_empty());
    assert!(matches!(
        store.update_set(set, &fixed),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(store.delete_set(set), Err(Error::NotFound(_))));

    store.log_set(session, &fixed).unwrap();
    store.delete_session(session).unwrap();
    let week = DaySpan::iso_week(date(2026, 11, 18));
    assert!(store.sessions_in(week).unwrap().sessions.is_empty());
    assert!(matches!(
        store.log_set(session, &fixed),
        Err(Error::NotFound(_))
    ));
    // The set itself is not tombstoned, it only stops being visible.
    assert!(store.sets_of(session).unwrap().sets.is_empty());
}

#[test]
fn outbox_coalesces_writes_per_row() {
    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    let bench = bench();
    let session = store.start_session(draft()).unwrap();
    let set = store.log_set(session, &working_set(&bench, 80.0)).unwrap();
    let before = store.pending_outbox(10).unwrap();
    assert_eq!(before.len(), 2);

    store.update_set(set, &working_set(&bench, 85.0)).unwrap();
    store.delete_set(set).unwrap();
    let after = store.pending_outbox(10).unwrap();
    assert_eq!(after.len(), 2, "one pending entry per row");

    let set_entry = after
        .iter()
        .find(|entry| entry.entity == Some(Entity::SetEntry))
        .unwrap();
    let original = before
        .iter()
        .find(|entry| entry.entity == Some(Entity::SetEntry))
        .unwrap();
    assert_eq!(set_entry.row_id, set.uuid());
    assert_eq!(set_entry.op, Some(Op::Delete));
    assert!(set_entry.seq > original.seq, "moves behind earlier writes");
    assert!(set_entry.hlc > original.hlc);
}

#[test]
fn engine_reads_the_week_from_the_store() {
    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    let bench = bench();
    let session = store.start_session(draft()).unwrap();
    store
        .log_set(
            session,
            &SetRecord::new(bench.id)
                .with_kind(SetKind::Warmup)
                .with_lift(Mass::from_kg(40.0).unwrap(), 10),
        )
        .unwrap();
    store.log_set(session, &working_set(&bench, 80.0)).unwrap();
    store.log_set(session, &working_set(&bench, 80.0)).unwrap();

    let week = DaySpan::iso_week(date(2026, 11, 18));
    let read = store.sessions_in(week).unwrap();
    assert_eq!(read.undecodable_sets, 0);
    let report = VolumeReport::compute(
        week,
        LoadModel::WeightedV1,
        &read.sessions,
        std::slice::from_ref(&bench),
        &NoReferences,
    );
    assert_eq!(report.counted_sets, 2);
    assert!((report.volume.get(Muscle::Chest).total().get() - 2.0).abs() < 1e-9);

    let next_week = DaySpan::iso_week(date(2026, 11, 25));
    assert!(store.sessions_in(next_week).unwrap().sessions.is_empty());
}

#[test]
fn every_set_field_round_trips() {
    use myos_core::session::{HeartRateZones, Stroke};
    use std::time::Duration;

    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    let swim = bench();
    let session = store.start_session(draft()).unwrap();
    let mut set = SetRecord::new(swim.id)
        .with_distance(myos_core::units::Distance::from_meters(400))
        .with_duration(Duration::from_secs(420))
        .with_heart_rate(HeartRate::from_bpm(140).unwrap())
        .with_stroke(Stroke::Butterfly)
        .with_rest(Duration::from_secs(90))
        .with_assist(Mass::from_kg(10.0).unwrap());
    set.bodyweight = Mass::from_kg(78.5);
    set.max_heart_rate = HeartRate::from_bpm(171);
    set.zones = Some(HeartRateZones([
        Duration::from_secs(10),
        Duration::from_secs(60),
        Duration::from_secs(200),
        Duration::from_secs(150),
        Duration::ZERO,
    ]));
    store.log_set(session, &set).unwrap();
    assert_eq!(store.sets_of(session).unwrap().sets[0].record, set);

    let mut bad = set.clone();
    bad.max_heart_rate = HeartRate::from_bpm(120);
    assert!(matches!(
        store.log_set(session, &bad),
        Err(Error::OutOfRange("max_heart_rate"))
    ));
    let heavy = SetRecord::new(swim.id).with_lift(Mass::from_grams(4_000_001), 1);
    assert!(matches!(
        store.log_set(session, &heavy),
        Err(Error::OutOfRange(_))
    ));
}

#[test]
fn sets_of_deleted_sessions_cannot_change() {
    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    let bench = bench();
    let session = store.start_session(draft()).unwrap();
    let set = store.log_set(session, &working_set(&bench, 80.0)).unwrap();
    store.delete_session(session).unwrap();
    assert!(matches!(
        store.update_set(set, &working_set(&bench, 90.0)),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(store.delete_set(set), Err(Error::NotFound(_))));
}

#[test]
fn merged_outbox_entries_get_a_fresh_op_id() {
    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    let bench = bench();
    let session = store.start_session(draft()).unwrap();
    let set = store.log_set(session, &working_set(&bench, 80.0)).unwrap();
    let find = |store: &Store| {
        store
            .pending_outbox(10)
            .unwrap()
            .into_iter()
            .find(|entry| entry.row_id == set.uuid())
            .unwrap()
    };
    let before = find(&store);
    store.update_set(set, &working_set(&bench, 85.0)).unwrap();
    let after = find(&store);
    assert!(after.seq > before.seq);
    assert_ne!(before.op_id, after.op_id);
}
