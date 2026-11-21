//! `.myos` exports, imports and wiping the database (logout).
//!
//! An export is the whole user database re-encrypted under a passphrase
//! (`SQLCipher` KDF, PBKDF2-HMAC-SHA512 with [`EXPORT_KDF_ITER`] iterations
//! and the file's own salt): independent of the device key, so it restores
//! on any device, with or without an account.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use uuid::Uuid;

use crate::key::KeySource;
use crate::{Error, Store};

/// KDF iterations of an export (more than the interactive default: an
/// export is opened rarely and may sit on any storage).
pub const EXPORT_KDF_ITER: u32 = 600_000;

/// Files that make up a database at `path`.
fn database_files(path: &Path) -> Vec<PathBuf> {
    let derived = path.with_extension("derived.db");
    [path.to_owned(), derived]
        .into_iter()
        .flat_map(|base| {
            ["", "-wal", "-shm"].map(|suffix| {
                let mut name = base.as_os_str().to_owned();
                name.push(suffix);
                PathBuf::from(name)
            })
        })
        .collect()
}

fn remove_files(files: &[PathBuf]) -> Result<(), Error> {
    for file in files {
        match std::fs::remove_file(file) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Ok(())
}

/// Copy the main database of `conn` into `dest` (fresh file), keyed with
/// `key`; written to a temporary file then renamed.
fn export_main(
    conn: &Connection,
    dest: &Path,
    key: KeySource<'_>,
    kdf_iter: Option<u32>,
) -> Result<(), Error> {
    let mut temp = dest.as_os_str().to_owned();
    temp.push(".partial");
    let temp = PathBuf::from(temp);
    remove_files(std::slice::from_ref(&temp))?;
    let location = temp.to_str().ok_or(Error::OutOfRange("export path"))?;
    let secret = key.attach_value();
    conn.execute(
        "ATTACH DATABASE ?1 AS export KEY ?2",
        [location, secret.as_str()],
    )?;
    let copied = (|| -> Result<(), Error> {
        if let Some(iter) = kdf_iter {
            conn.pragma_update(Some("export"), "kdf_iter", iter)?;
        }
        conn.query_row("SELECT sqlcipher_export('export')", [], |_| Ok(()))?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        conn.pragma_update(Some("export"), "user_version", version)?;
        Ok(())
    })();
    conn.execute_batch("DETACH DATABASE export")?;
    if let Err(err) = copied {
        remove_files(std::slice::from_ref(&temp))?;
        return Err(err);
    }
    std::fs::rename(&temp, dest).map_err(Error::Io)
}

impl Store {
    /// Write an encrypted export of all user data to `dest`, protected by
    /// `passphrase`.
    pub fn export_to(&self, dest: &Path, passphrase: &str) -> Result<(), Error> {
        let key = KeySource::Passphrase(passphrase).check()?;
        // Pending WAL frames are part of the main database for the export.
        self.conn().execute_batch("PRAGMA wal_checkpoint(FULL);")?;
        export_main(self.conn(), dest, key, Some(EXPORT_KDF_ITER))
    }

    /// Restore an export into a new database at `dest` (which must not
    /// exist), unlocked afterwards by `key`. The restored database gets a
    /// new device identity; the account and its natural-key ids carry over.
    pub fn import<'k>(
        export: &Path,
        passphrase: &str,
        dest: &Path,
        key: impl Into<KeySource<'k>>,
    ) -> Result<Self, Error> {
        if dest.exists() {
            return Err(Error::OutOfRange("import destination exists"));
        }
        if !export.is_file() {
            return Err(Error::Io(std::io::ErrorKind::NotFound.into()));
        }
        // Read-write + create: attached files inherit the connection flags,
        // and the new database is created through this connection. The export
        // itself is never written.
        let source = Connection::open_with_flags(
            export,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        KeySource::Passphrase(passphrase).check()?.apply(&source)?;
        source.pragma_update(None, "kdf_iter", EXPORT_KDF_ITER)?;
        source
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|err| match err.sqlite_error_code() {
                Some(rusqlite::ErrorCode::NotADatabase) => Error::WrongKey,
                _ => Error::Sqlite(err),
            })?;
        let key = key.into().check()?;
        export_main(&source, dest, key, None)?;
        drop(source);
        let mut store = Self::open(dest, key)?;
        store.become_new_device()?;
        Ok(store)
    }

    /// New device id and writer node; nothing left in flight.
    fn become_new_device(&mut self) -> Result<(), Error> {
        let node = getrandom::u32().map_err(Error::Random)? & 0x7fff_ffff;
        let tx = self.conn_mut().transaction()?;
        tx.execute(
            "UPDATE sync_state SET device_id = ?1, node = ?2 WHERE k = 1",
            (Uuid::now_v7().as_bytes().as_slice(), node),
        )?;
        tx.execute("UPDATE sync_outbox SET state = 0 WHERE state = 1", [])?;
        tx.commit()?;
        Ok(())
    }

    /// Close and delete the database files (logout, account deletion). The
    /// caller also drops the key from the credential store.
    pub fn wipe(self) -> Result<(), Error> {
        let Self { conn, path } = self;
        conn.close().map_err(|(_, err)| Error::Sqlite(err))?;
        path.map_or(Ok(()), |path| remove_files(&database_files(&path)))
    }
}
