//! Exports, imports, passphrase-unlocked databases and wiping.
#![expect(clippy::unwrap_used, reason = "tests: a panic is the failure report")]

use std::path::PathBuf;

use jiff::Timestamp;
use jiff::civil::date;
use myos_core::id::Id;
use myos_core::session::{SessionOrigin, SetRecord};
use myos_core::units::Mass;
use myos_core::volume::DaySpan;
use myos_store::{DatabaseKey, Error, KeySource, SessionDraft, Store};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("myos-export-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _removed = std::fs::remove_dir_all(&self.0);
    }
}

fn log_one_set(store: &mut Store) {
    let session = store
        .start_session(SessionDraft {
            started_at: Timestamp::from_second(1_795_000_000).unwrap(),
            day: date(2026, 11, 18),
            tz_offset_min: 0,
            origin: SessionOrigin::Manual,
        })
        .unwrap();
    store
        .log_set(
            session,
            &SetRecord::new(Id::generate()).with_lift(Mass::from_kg(80.0).unwrap(), 8),
        )
        .unwrap();
}

fn week() -> DaySpan {
    DaySpan::iso_week(date(2026, 11, 18))
}

#[test]
fn export_restores_on_another_device() {
    let dir = TempDir::new();
    let key = DatabaseKey::generate().unwrap();
    let mut store = Store::open(dir.0.join("user.db"), &key).unwrap();
    log_one_set(&mut store);
    let export = dir.0.join("backup.myos");
    store.export_to(&export, "correct horse battery").unwrap();
    // The export opens with the passphrase only, not the device key.
    let header = std::fs::read(&export).unwrap();
    assert!(!header.starts_with(b"SQLite format 3"));

    let other_key = DatabaseKey::generate().unwrap();
    let restored_path = dir.0.join("restored.db");
    assert!(matches!(
        Store::import(&export, "wrong passphrase", &restored_path, &other_key),
        Err(Error::WrongKey)
    ));
    let restored =
        Store::import(&export, "correct horse battery", &restored_path, &other_key).unwrap();
    assert_eq!(
        restored.sessions_in(week()).unwrap(),
        store.sessions_in(week()).unwrap()
    );
    // Reopens with the new device's key.
    drop(restored);
    let reopened = Store::open(&restored_path, &other_key).unwrap();
    assert_eq!(reopened.sessions_in(week()).unwrap().sessions.len(), 1);
    assert!(matches!(
        Store::import(&export, "correct horse battery", &restored_path, &other_key),
        Err(Error::OutOfRange(_))
    ));
}

#[test]
fn short_passphrases_are_refused() {
    let dir = TempDir::new();
    let store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    assert!(matches!(
        store.export_to(&dir.0.join("x.myos"), "short"),
        Err(Error::OutOfRange("passphrase"))
    ));
    assert!(matches!(
        Store::open(dir.0.join("p.db"), KeySource::Passphrase("short")),
        Err(Error::OutOfRange("passphrase"))
    ));
}

#[test]
fn a_passphrase_unlocks_a_database_without_credential_store() {
    let dir = TempDir::new();
    let path = dir.0.join("user.db");
    {
        let mut store = Store::open(&path, KeySource::Passphrase("una frase larga")).unwrap();
        log_one_set(&mut store);
    }
    assert!(matches!(
        Store::open(&path, KeySource::Passphrase("otra frase larga")),
        Err(Error::WrongKey)
    ));
    let store = Store::open(&path, KeySource::Passphrase("una frase larga")).unwrap();
    assert_eq!(store.sessions_in(week()).unwrap().sessions.len(), 1);
}

#[test]
fn wipe_deletes_every_file() {
    let dir = TempDir::new();
    let path = dir.0.join("user.db");
    let mut store = Store::open(&path, &DatabaseKey::generate().unwrap()).unwrap();
    log_one_set(&mut store);
    store.wipe().unwrap();
    let left: Vec<_> = std::fs::read_dir(&dir.0).unwrap().collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn raw_key_literals_are_not_passphrases() {
    let dir = TempDir::new();
    assert!(matches!(
        Store::open(
            dir.0.join("p.db"),
            KeySource::Passphrase("x'00112233445566'")
        ),
        Err(Error::OutOfRange("passphrase"))
    ));
}

#[test]
fn rekey_moves_between_passphrase_and_device_key() {
    let dir = TempDir::new();
    let path = dir.0.join("user.db");
    let device = DatabaseKey::generate().unwrap();
    {
        let mut store = Store::open(&path, KeySource::Passphrase("primera frase")).unwrap();
        log_one_set(&mut store);
        store.rekey(&device).unwrap();
        assert_eq!(store.sessions_in(week()).unwrap().sessions.len(), 1);
    }
    assert!(matches!(
        Store::open(&path, KeySource::Passphrase("primera frase")),
        Err(Error::WrongKey)
    ));
    let mut store = Store::open(&path, &device).unwrap();
    store.rekey(KeySource::Passphrase("segunda frase")).unwrap();
    drop(store);
    let store = Store::open(&path, KeySource::Passphrase("segunda frase")).unwrap();
    assert_eq!(store.sessions_in(week()).unwrap().sessions.len(), 1);
}

#[test]
fn imported_databases_keep_incremental_vacuum() {
    let dir = TempDir::new();
    let key = DatabaseKey::generate().unwrap();
    let mut store = Store::open(dir.0.join("user.db"), &key).unwrap();
    log_one_set(&mut store);
    let export = dir.0.join("b.myos");
    store.export_to(&export, "frase de prueba").unwrap();
    let restored = dir.0.join("r.db");
    drop(Store::import(&export, "frase de prueba", &restored, &key).unwrap());
    let mut hex = String::new();
    for byte in key.as_bytes() {
        std::fmt::Write::write_fmt(&mut hex, format_args!("{byte:02x}")).unwrap();
    }
    let conn = rusqlite::Connection::open(&restored).unwrap();
    conn.execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
        .unwrap();
    let mode: i64 = conn
        .query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, 2, "incremental");
}
