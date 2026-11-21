//! Myos on-device storage: encrypted `SQLite`, migrations and the sync outbox.
//!
//! [`Store`] owns the `user.db` connection. Every write runs through one
//! transaction that stamps the row with a hybrid logical clock, bumps the
//! local sequence and queues the row in the sync outbox.

mod content;
mod convert;
mod derived;
mod diet;
mod exercises;
mod export;
mod hlc;
mod ids;
mod key;
mod migrate;
mod payload;
mod plan;
mod profile;
mod replication;
mod routines;
mod spec;
mod sync;
pub mod tlv;
mod training;

use std::path::{Path, PathBuf};

use rusqlite::{Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

pub use derived::RefreshStats;
pub use diet::{FoodId, MealsRead};
pub use exercises::CustomExercise;
pub use export::EXPORT_KDF_ITER;
pub use hlc::Hlc;
pub use key::{DatabaseKey, KeySource, MIN_PASSPHRASE_CHARS};
pub use plan::Read;
pub use profile::MeasurementsRead;
pub use replication::{
    PAYLOAD_VERSION, PROTO, PullPage, PushBatch, PushOp, PushOutcome, PushResponse, PushResult,
    RemoteRow, STREAM_OWN, SyncReport, Transport,
};
pub use routines::{RoutineSummary, StoredRoutine};
pub use sync::{Entity, Op, OutboxEntry};
pub use training::{SessionDraft, SessionsRead, SetsRead, StoredSet};

/// Storage failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// `SQLite` reported an error.
    #[error("database error")]
    Sqlite(#[from] rusqlite::Error),
    /// The file is not a database under this key (wrong or lost key, or a
    /// corrupt file).
    #[error("wrong key or not a Myos database")]
    WrongKey,
    /// The derived cache could not be opened even after recreating it (the
    /// user database itself is fine).
    #[error("derived cache cannot be opened")]
    DerivedUnreadable,
    /// The file was written by a newer app version.
    #[error("database schema {found} is newer than this app supports ({supported})")]
    NewerSchema {
        /// Schema version in the file.
        found: usize,
        /// Latest schema this build knows.
        supported: usize,
    },
    /// A migration left foreign-key violations; it was rolled back.
    #[error("migration {version} left foreign key violations")]
    MigrationIntegrity {
        /// Migration that failed.
        version: usize,
    },
    /// The OS could not provide randomness for a key.
    #[error("system randomness unavailable: {0}")]
    Random(getrandom::Error),
    /// The server requires a newer app: local writes are refused until it
    /// updates.
    #[error("this app version is read-only until it is updated")]
    ReadOnly,
    /// The profile breaks a rule (minimum age, implausible birth year).
    #[error(transparent)]
    Profile(#[from] myos_core::profile::ProfileError),
    /// A body measurement is implausible or has the wrong side.
    #[error(transparent)]
    Measurement(#[from] myos_core::body::MeasurementError),
    /// The row does not exist or was deleted.
    #[error("{0} not found")]
    NotFound(&'static str),
    /// The file system cannot run the database in WAL mode.
    #[error("journal mode {0} instead of WAL")]
    JournalMode(String),
    /// A value does not fit its stored range.
    #[error("{0} is out of range")]
    OutOfRange(&'static str),
    /// Sync needs an account (natural-key ids are rewritten when attaching one).
    #[error("no account attached")]
    NoAccount,
    /// The device already holds another account's data.
    #[error("another account is attached")]
    AccountMismatch,
    /// The transport failed; queued writes are retried later.
    #[error("sync transport failed")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// A file operation failed (export, import, wipe).
    #[error("file operation failed")]
    Io(#[source] std::io::Error),
    /// A structured column or payload is malformed.
    #[error("malformed record: {0}")]
    Payload(#[from] crate::tlv::TlvError),
}

/// Where the database lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Durability {
    File,
    Memory,
}

/// Stamp shared by every row written in one transaction.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Stamp {
    pub(crate) hlc: Hlc,
    pub(crate) node: i64,
    pub(crate) local_seq: i64,
    pub(crate) now_millis: i64,
}

/// The user's database.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
    /// File of the main database; `None` in memory.
    path: Option<PathBuf>,
}

impl Store {
    /// Open (or create) `user.db` at `path`, unlocked by `key` (a
    /// [`DatabaseKey`] or a [`KeySource::Passphrase`]), and migrate it. The
    /// derived cache lives next to it (`<name>.derived.db`, same key).
    ///
    /// # Errors
    ///
    /// [`Error::WrongKey`] when the key does not open the file,
    /// [`Error::NewerSchema`] when a newer app wrote it.
    pub fn open<'k>(path: impl AsRef<Path>, key: impl Into<KeySource<'k>>) -> Result<Self, Error> {
        let path = path.as_ref();
        let derived = path.with_extension("derived.db");
        let mut store = Self::setup(
            Connection::open(path)?,
            key.into().check()?,
            Durability::File,
            &derived,
        )?;
        store.path = Some(path.to_owned());
        Ok(store)
    }

    /// An in-memory database (tests, previews). `SQLCipher` does not encrypt
    /// memory databases; the key only exercises the same setup path.
    pub fn open_in_memory(key: &DatabaseKey) -> Result<Self, Error> {
        Self::setup(
            Connection::open_in_memory()?,
            KeySource::Raw(key),
            Durability::Memory,
            Path::new(":memory:"),
        )
    }

    fn setup(
        mut conn: Connection,
        key: KeySource<'_>,
        durability: Durability,
        derived: &Path,
    ) -> Result<Self, Error> {
        key.apply(&conn)?;
        // Pin the format so a future SQLCipher major still opens these files.
        conn.execute_batch("PRAGMA cipher_compatibility = 4;")?;
        let tables: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
            .map_err(|err| match err.sqlite_error_code() {
                Some(ErrorCode::NotADatabase) => Error::WrongKey,
                _ => Error::Sqlite(err),
            })?;
        if tables == 0 {
            // Only takes effect before the first table exists.
            conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
        }
        let journal: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if durability == Durability::File && !journal.eq_ignore_ascii_case("wal") {
            return Err(Error::JournalMode(journal));
        }
        conn.execute_batch(
            "PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;",
        )?;
        migrate::run(&mut conn, migrate::USER)?;
        Self::recover_in_flight(&mut conn)?;
        Self::init_sync_state(&conn)?;
        // Passphrase-unlocked: the cache gets its own random raw key.
        let derived_key = match key {
            KeySource::Raw(_) => None,
            KeySource::Passphrase(_) => Some(Self::local_derived_key(&conn)?),
        };
        derived::attach(
            &conn,
            derived,
            derived_key.as_ref().map_or(key, KeySource::Raw),
        )?;
        Ok(Self { conn, path: None })
    }

    fn init_sync_state(conn: &Connection) -> Result<(), Error> {
        let exists: Option<i64> = conn
            .query_row("SELECT k FROM sync_state", [], |row| row.get(0))
            .optional()?;
        // `OR IGNORE`: another connection may initialize it concurrently.
        if exists.is_none() {
            let node = getrandom::u32().map_err(Error::Random)? & 0x7fff_ffff;
            let mut user_key = zeroize::Zeroizing::new([0_u8; 32]);
            getrandom::fill(user_key.as_mut_slice()).map_err(Error::Random)?;
            conn.execute(
                "INSERT OR IGNORE INTO sync_state (k, device_id, node, user_key, hlc_last)
                 VALUES (1, ?1, ?2, ?3, 0)",
                (
                    Uuid::now_v7().as_bytes().as_slice(),
                    node,
                    user_key.as_slice(),
                ),
            )?;
        }
        Ok(())
    }

    /// Run `write` in one immediate transaction with a fresh [`Stamp`].
    pub(crate) fn write<T>(
        &mut self,
        write: impl FnOnce(&Transaction<'_>, Stamp) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (node, last, local_seq, offset, read_only): (i64, i64, i64, i64, bool) = tx.query_row(
            "SELECT node, hlc_last, local_seq, server_offset_ms, read_only
                 FROM sync_state WHERE k = 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
        if read_only {
            return Err(Error::ReadOnly);
        }
        let now_millis = jiff::Timestamp::now().as_millisecond();
        let wall = u64::try_from(now_millis.saturating_add(offset)).unwrap_or(0);
        let hlc = Hlc::from_raw(u64::try_from(last).unwrap_or(0)).tick(wall);
        let stamp = Stamp {
            hlc,
            node,
            local_seq: local_seq
                .checked_add(1)
                .ok_or(Error::OutOfRange("local_seq"))?,
            now_millis,
        };
        tx.execute(
            "UPDATE sync_state SET hlc_last = ?1, local_seq = ?2 WHERE k = 1",
            (sync::sql_hlc(hlc)?, stamp.local_seq),
        )?;
        let value = write(&tx, stamp)?;
        tx.commit()?;
        Ok(value)
    }

    /// Connection for read-only queries.
    pub(crate) const fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Connection for transactions outside [`Store::write`] (caches).
    pub(crate) const fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

impl Store {
    /// Raw key of `derived.db` for passphrase-unlocked databases, created on
    /// first use.
    fn local_derived_key(conn: &Connection) -> Result<DatabaseKey, Error> {
        let stored: Option<[u8; 32]> = conn
            .query_row(
                "SELECT derived_key FROM local_secret WHERE k = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(mut bytes) = stored {
            return Ok(DatabaseKey::take(&mut bytes));
        }
        let key = DatabaseKey::generate()?;
        conn.execute(
            "INSERT INTO local_secret (k, derived_key) VALUES (1, ?1)",
            [key.as_bytes().as_slice()],
        )?;
        Ok(key)
    }

    /// Re-encrypt the database under `new` (change the passphrase, or move
    /// to a credential-store key once one exists). The derived cache is
    /// rebuilt under the matching key.
    pub fn rekey<'k>(&mut self, new: impl Into<KeySource<'k>>) -> Result<(), Error> {
        let new = new.into().check()?;
        let Some(path) = self.path.clone() else {
            return Err(Error::OutOfRange("in-memory database"));
        };
        self.conn.execute_batch("DETACH DATABASE derived")?;
        match new {
            KeySource::Raw(key) => self.conn.execute_batch(&key.rekey_pragma())?,
            KeySource::Passphrase(passphrase) => {
                self.conn.pragma_update(None, "rekey", passphrase)?;
            }
        }
        let derived = path.with_extension("derived.db");
        for suffix in ["", "-wal", "-shm"] {
            let mut file = derived.as_os_str().to_owned();
            file.push(suffix);
            match std::fs::remove_file(PathBuf::from(file)) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(Error::Io(err)),
            }
        }
        let derived_key = match new {
            KeySource::Raw(_) => None,
            KeySource::Passphrase(_) => Some(Self::local_derived_key(&self.conn)?),
        };
        derived::attach(
            &self.conn,
            &derived,
            derived_key.as_ref().map_or(new, KeySource::Raw),
        )
    }
}
