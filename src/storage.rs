//! Opening the user's encrypted database.
//!
//! The key lives in the platform credential store ([`myos_vault`]); the
//! files live in a private app directory: on Android `noBackupFilesDir`
//! (the key is never in a backup, so the database must not be either), on
//! the other platforms the local data directory, flagged as excluded from
//! backups on iOS / macOS ([`myos_backup`]).

use std::path::{Path, PathBuf};

use myos_store::Store;
use myos_vault::{Stored, Vault, VaultError};

/// App identity, as in `istmo.toml` (`[app] id`): the credential-store
/// service name.
const APP_ID: &str = "rs.sergioribera.myos";

/// File name of the user database inside the data directory.
const DATABASE_FILE: &str = "user.db";

/// Files that belong to one database.
const DATABASE_FILES: [&str; 6] = [
    "user.db",
    "user.db-wal",
    "user.db-shm",
    "user.derived.db",
    "user.derived.db-wal",
    "user.derived.db-shm",
];

/// Why the database could not be opened.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StorageError {
    /// The data directory could not be created.
    #[error("cannot create the data directory {path}")]
    Directory {
        /// Directory that failed.
        path: PathBuf,
        /// Underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// An unreadable database could not be removed.
    #[error("cannot remove the unreadable database file {path}")]
    Remove {
        /// File that failed.
        path: PathBuf,
        /// Underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The credential store failed or is missing.
    #[error(transparent)]
    Vault(#[from] VaultError),
    /// The database failed to open.
    #[error(transparent)]
    Store(#[from] myos_store::Error),
}

/// Why the database started over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reset {
    /// The key was gone or unreadable (restored backup, reset credential
    /// store): the old database could never be opened again. The UI offers
    /// a server restore or an export import.
    KeyLost,
}

/// An opened database.
#[derive(Debug)]
pub struct Opened {
    /// The database.
    pub store: Store,
    /// Set when the previous database had to be discarded.
    pub reset: Option<Reset>,
}

/// Directory holding the database files.
fn database_dir() -> PathBuf {
    #[cfg(target_os = "android")]
    let base = istmo::path::state_dir();
    #[cfg(not(target_os = "android"))]
    let base = istmo::path::data_local_dir();
    base.join("database")
}

/// Create the database directory and keep it out of OS backups (the key
/// never leaves the device, so a restored copy could not be opened). Run on
/// every open: recreating the directory drops the flag.
fn prepare_dir() -> Result<PathBuf, StorageError> {
    let dir = database_dir();
    std::fs::create_dir_all(&dir).map_err(|source| StorageError::Directory {
        path: dir.clone(),
        source,
    })?;
    if let Err(err) = myos_backup::exclude_from_backup(&dir) {
        log::warn!("{err}");
    }
    Ok(dir)
}

/// Open (creating on first run) the user's database.
pub fn open() -> Result<Opened, StorageError> {
    let dir = prepare_dir()?;
    let path = dir.join(DATABASE_FILE);
    let vault = Vault::open(APP_ID)?;
    let (key, mut reset) = match vault.load()? {
        Stored::Key(key) => (key, None),
        Stored::Missing if !path.exists() => (vault.create()?, None),
        Stored::Missing | Stored::Lost => {
            remove_database(&dir)?;
            (vault.create()?, Some(Reset::KeyLost))
        }
    };
    let store = match Store::open(&path, &key) {
        Err(myos_store::Error::WrongKey) => {
            // The stored key is not this file's key: the file is lost.
            remove_database(&dir)?;
            reset = Some(Reset::KeyLost);
            Store::open(&path, &key)?
        }
        opened => opened?,
    };
    Ok(Opened { store, reset })
}

/// Delete the database files in `dir`. Without its key a database is noise:
/// keeping it would only leave undecryptable health data around.
fn remove_database(dir: &Path) -> Result<(), StorageError> {
    for name in DATABASE_FILES {
        let path = dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(StorageError::Remove { path, source }),
        }
    }
    log::warn!("database key lost: started a new database");
    Ok(())
}

/// Open the database with a passphrase, for systems without a credential
/// store ([`VaultError::Unavailable`] from [`open`]): the UI asks for it.
pub fn open_with_passphrase(passphrase: &str) -> Result<Opened, StorageError> {
    let dir = prepare_dir()?;
    let store = Store::open(
        dir.join(DATABASE_FILE),
        myos_store::KeySource::Passphrase(passphrase),
    )?;
    Ok(Opened { store, reset: None })
}

/// Log out: delete the database files and the stored key. Data that was not
/// synced or exported is gone.
pub fn logout(store: Store) -> Result<(), StorageError> {
    // Key first: if deleting the files fails, the next start finds a
    // database without key and discards it.
    match Vault::open(APP_ID) {
        Ok(vault) => vault.forget()?,
        // Passphrase-unlocked database: no stored key to drop.
        Err(VaultError::Unavailable(_)) => {}
        Err(other) => return Err(other.into()),
    }
    store.wipe()?;
    Ok(())
}
