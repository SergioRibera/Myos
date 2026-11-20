//! Opening the user's encrypted database.
//!
//! The key lives in the platform credential store ([`myos_vault`]); the
//! files live in the app's private data directory, which the platform
//! excludes from cloud backups only where the native project says so
//! (Android `dataExtractionRules`, iOS `isExcludedFromBackup`).

use std::path::PathBuf;

use myos_store::Store;
use myos_vault::{Vault, VaultError};

/// App identity, as in `istmo.toml` (`[app] id`): the credential-store
/// service name.
const APP_ID: &str = "rs.sergioribera.myos";

/// File name of the user database inside the data directory.
const DATABASE_FILE: &str = "user.db";

/// Why the database could not be opened.
#[derive(Debug, thiserror::Error)]
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
    /// A database without its key could not be moved aside.
    #[error("cannot move the unreadable database {path} aside")]
    MoveAside {
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

/// Directory holding `user.db` and its derived cache: private to the app and
/// never shown in a document browser.
fn database_dir() -> PathBuf {
    // iOS `data_dir` is the user-visible Documents folder; Application
    // Support (`config_dir` there) is private.
    #[cfg(target_os = "ios")]
    let base = istmo::path::config_dir();
    #[cfg(not(target_os = "ios"))]
    let base = istmo::path::data_local_dir();
    base.join("database")
}

/// Open (creating on first run) the user's database.
///
/// A database whose key is gone (OS restored the file from a backup but not
/// the key, or the credential store was reset) can never be read again: it
/// is moved aside and a fresh one starts, to be filled from the server or an
/// encrypted export.
pub fn open() -> Result<Store, StorageError> {
    let dir = database_dir();
    std::fs::create_dir_all(&dir).map_err(|source| StorageError::Directory {
        path: dir.clone(),
        source,
    })?;
    let path = dir.join(DATABASE_FILE);
    let vault = Vault::open(APP_ID)?;
    if path.exists() && !vault.has_key()? {
        move_aside(&path)?;
    }
    let key = vault.database_key()?;
    match Store::open(&path, &key) {
        Err(myos_store::Error::WrongKey) => {
            move_aside(&path)?;
            Ok(Store::open(&path, &key)?)
        }
        opened => Ok(opened?),
    }
}

/// Rename `path` (and its WAL files) to `<name>.unreadable-<unix ms>`.
fn move_aside(path: &std::path::Path) -> Result<(), StorageError> {
    let stamp = unix_millis();
    for suffix in ["", "-wal", "-shm"] {
        let mut from = path.as_os_str().to_owned();
        from.push(suffix);
        let from = PathBuf::from(from);
        if !from.exists() {
            continue;
        }
        let mut to = path.as_os_str().to_owned();
        to.push(format!(".unreadable-{stamp}{suffix}"));
        std::fs::rename(&from, &to).map_err(|source| StorageError::MoveAside {
            path: from.clone(),
            source,
        })?;
    }
    log::warn!("database key missing: moved the unreadable database aside");
    Ok(())
}

/// Milliseconds since the Unix epoch (0 if the clock is before it).
fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}
