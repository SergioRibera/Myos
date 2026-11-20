//! Myos on-device storage: encrypted `SQLite`, migrations and the sync outbox.
//!
//! [`Store`] owns the `user.db` connection. Every write runs through one
//! transaction that stamps the row with a hybrid logical clock, bumps the
//! local sequence and queues the row in the sync outbox.

mod convert;
mod hlc;
mod key;
mod migrate;
mod sync;
pub mod tlv;
mod training;

use std::path::Path;

use rusqlite::{Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

pub use hlc::Hlc;
pub use key::DatabaseKey;
pub use sync::{Entity, Op, OutboxEntry};
pub use training::{SessionDraft, SessionsRead, StoredSet};

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
    /// The row does not exist or was deleted.
    #[error("{0} not found")]
    NotFound(&'static str),
    /// A value does not fit its stored range.
    #[error("{0} is out of range")]
    OutOfRange(&'static str),
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
}

impl Store {
    /// Open (or create) `user.db` at `path` with `key`, and migrate it.
    ///
    /// # Errors
    ///
    /// [`Error::WrongKey`] when the key does not open the file,
    /// [`Error::NewerSchema`] when a newer app wrote it.
    pub fn open(path: impl AsRef<Path>, key: &DatabaseKey) -> Result<Self, Error> {
        Self::setup(Connection::open(path)?, key)
    }

    /// An encrypted in-memory database (tests, previews).
    pub fn open_in_memory(key: &DatabaseKey) -> Result<Self, Error> {
        Self::setup(Connection::open_in_memory()?, key)
    }

    fn setup(mut conn: Connection, key: &DatabaseKey) -> Result<Self, Error> {
        conn.execute_batch(&key.pragma())?;
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
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;",
        )?;
        migrate::run(&mut conn, migrate::USER)?;
        Self::init_sync_state(&conn)?;
        Ok(Self { conn })
    }

    fn init_sync_state(conn: &Connection) -> Result<(), Error> {
        let exists: Option<i64> = conn
            .query_row("SELECT k FROM sync_state", [], |row| row.get(0))
            .optional()?;
        if exists.is_none() {
            let node = getrandom::u32().map_err(Error::Random)? & 0x7fff_ffff;
            conn.execute(
                "INSERT INTO sync_state (k, device_id, node, hlc_last) VALUES (1, ?1, ?2, 0)",
                (Uuid::now_v7().as_bytes().as_slice(), node),
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
        let (node, last, local_seq, offset): (i64, i64, i64, i64) = tx.query_row(
            "SELECT node, hlc_last, local_seq, server_offset_ms FROM sync_state WHERE k = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let now_millis = jiff::Timestamp::now().as_millisecond();
        let wall = u64::try_from(now_millis.saturating_add(offset)).unwrap_or(0);
        let hlc = Hlc::from_raw(u64::try_from(last).unwrap_or(0)).tick(wall);
        let stamp = Stamp {
            hlc,
            node,
            local_seq: local_seq + 1,
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
}
