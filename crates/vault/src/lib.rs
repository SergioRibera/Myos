//! `myos-vault`: the database key in the platform credential store.
//!
//! The key is readable without user interaction (background sync opens the
//! database), never synced to other devices and never written in plain text:
//!
//! - Linux / BSD: Secret Service (GNOME Keyring, `KWallet`) over D-Bus.
//! - macOS: login Keychain.
//! - iOS: data-protection Keychain, after first unlock, this device only.
//! - Windows: Credential Manager.
//! - Android: `SharedPreferences` encrypted with an Android Keystore key (no
//!   user-authentication requirement). Needs the `ndk-context` application
//!   context, which the app's `GameActivity` host initializes.
//!
//! Without a credential store (e.g. Linux without Secret Service) opening
//! fails with [`VaultError::Unavailable`]; the app then asks for a passphrase
//! instead of silently storing the key in a file.

use std::sync::OnceLock;

use keyring_core::{Entry, Error as StoreError};
use myos_store::DatabaseKey;
use zeroize::Zeroize;

/// Account name of the database key inside the app's service.
const KEY_ACCOUNT: &str = "database-key";

/// Credential store failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VaultError {
    /// This platform or session has no usable credential store.
    #[error("no credential store available: {0}")]
    Unavailable(String),
    /// The credential store failed.
    #[error("credential store error")]
    Store(#[source] StoreError),
    /// A stored value is not a database key (wrong length).
    #[error("stored database key is malformed")]
    Malformed,
    /// A new key could not be generated.
    #[error("cannot generate a database key")]
    Generate(#[source] myos_store::Error),
}

impl From<StoreError> for VaultError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NoStorageAccess(_)
            | StoreError::NoDefaultStore
            | StoreError::NotSupportedByStore(_) => Self::Unavailable(error.to_string()),
            other => Self::Store(other),
        }
    }
}

/// The app's entry in the credential store.
#[derive(Debug)]
pub struct Vault {
    entry: Entry,
}

/// Result of the one-time store installation (stores are process-wide).
static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();

impl Vault {
    /// Open the vault of the app identified by `app_id` (e.g.
    /// `rs.sergioribera.myos`).
    pub fn open(app_id: &str) -> Result<Self, VaultError> {
        INSTALLED
            .get_or_init(|| install_store().map_err(|err| err.to_string()))
            .clone()
            .map_err(VaultError::Unavailable)?;
        let entry = new_entry(app_id)?;
        Ok(Self { entry })
    }

    /// The database key, created and stored on first use.
    pub fn database_key(&self) -> Result<DatabaseKey, VaultError> {
        match self.entry.get_secret() {
            Ok(mut secret) => {
                let key = <&mut [u8; 32]>::try_from(secret.as_mut_slice())
                    .map(DatabaseKey::take)
                    .map_err(|_| VaultError::Malformed);
                secret.zeroize();
                key
            }
            Err(StoreError::NoEntry) => {
                let key = DatabaseKey::generate().map_err(VaultError::Generate)?;
                self.entry.set_secret(key.as_bytes())?;
                Ok(key)
            }
            Err(other) => Err(other.into()),
        }
    }

    /// Whether a key is stored (a database file without one cannot be opened
    /// and must be restored from the server or an export).
    pub fn has_key(&self) -> Result<bool, VaultError> {
        match self.entry.get_secret() {
            Ok(mut secret) => {
                secret.zeroize();
                Ok(true)
            }
            Err(StoreError::NoEntry) => Ok(false),
            Err(other) => Err(other.into()),
        }
    }

    /// Delete the stored key (logout wipes the database with it).
    pub fn forget(&self) -> Result<(), VaultError> {
        match self.entry.delete_credential() {
            Ok(()) | Err(StoreError::NoEntry) => Ok(()),
            Err(other) => Err(other.into()),
        }
    }
}

#[cfg(target_os = "ios")]
fn new_entry(app_id: &str) -> Result<Entry, StoreError> {
    let modifiers =
        std::collections::HashMap::from([("access-policy", "after-first-unlock-this-device-only")]);
    Entry::new_with_modifiers(app_id, KEY_ACCOUNT, &modifiers)
}

#[cfg(not(target_os = "ios"))]
fn new_entry(app_id: &str) -> Result<Entry, StoreError> {
    Entry::new(app_id, KEY_ACCOUNT)
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn install_store() -> Result<(), StoreError> {
    keyring_core::set_default_store(zbus_secret_service_keyring_store::Store::new()?);
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_store() -> Result<(), StoreError> {
    keyring_core::set_default_store(apple_native_keyring_store::keychain::Store::new()?);
    Ok(())
}

#[cfg(target_os = "ios")]
fn install_store() -> Result<(), StoreError> {
    keyring_core::set_default_store(apple_native_keyring_store::protected::Store::new()?);
    Ok(())
}

#[cfg(target_os = "windows")]
fn install_store() -> Result<(), StoreError> {
    keyring_core::set_default_store(windows_native_keyring_store::Store::new()?);
    Ok(())
}

#[cfg(target_os = "android")]
fn install_store() -> Result<(), StoreError> {
    keyring_core::set_default_store(android_native_keyring_store::Store::new()?);
    Ok(())
}

#[cfg(not(any(unix, target_os = "windows")))]
fn install_store() -> Result<(), StoreError> {
    Err(StoreError::NotSupportedByStore(
        "no credential store on this platform".to_owned(),
    ))
}
