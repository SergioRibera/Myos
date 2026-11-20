//! `myos-vault`: the database key in the platform credential store.
//!
//! The key is readable without user interaction (background sync opens the
//! database), never synced to other devices and never written in plain text:
//!
//! - Linux / BSD: Secret Service (GNOME Keyring, `KWallet`) over D-Bus.
//! - macOS: login Keychain (access is tied to the code signature: release
//!   builds need a stable signing identity).
//! - iOS: data-protection Keychain, after first unlock, this device only.
//! - Windows: Credential Manager, local persistence (never roams).
//! - Android: `SharedPreferences` encrypted with an Android Keystore key (no
//!   user-authentication requirement). Needs the `ndk-context` application
//!   context, which the app's `GameActivity` host initializes; processes
//!   without an activity must not open the vault.
//!
//! The key is stored as lowercase hex: some stores (`KWallet`) only keep
//! UTF-8. Without a credential store (e.g. Linux without Secret Service)
//! opening fails with [`VaultError::Unavailable`]; the app then asks for a
//! passphrase instead of silently storing the key in a file.

use std::sync::Mutex;

use keyring_core::{Entry, Error as StoreError};
use myos_store::DatabaseKey;
use zeroize::{Zeroize, Zeroizing};

/// Account name of the database key inside the app's service.
const KEY_ACCOUNT: &str = "database-key";

/// Credential store failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VaultError {
    /// This platform or session has no usable credential store.
    #[error("no credential store available")]
    Unavailable(#[source] StoreError),
    /// The credential store failed.
    #[error("credential store error")]
    Store(#[source] StoreError),
    /// A new key could not be generated.
    #[error("cannot generate a database key")]
    Generate(#[source] myos_store::Error),
}

impl From<StoreError> for VaultError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NoStorageAccess(_)
            | StoreError::NoDefaultStore
            | StoreError::NotSupportedByStore(_) => Self::Unavailable(error),
            other => Self::Store(other),
        }
    }
}

/// What [`Vault::load`] found.
#[derive(Debug)]
pub enum Stored {
    /// The key.
    Key(DatabaseKey),
    /// Nothing stored.
    Missing,
    /// Something is stored but cannot be read back as a key (restored from a
    /// backup without its wrapping key, corrupted). It has been deleted: the
    /// database it protected can never be opened again.
    Lost,
}

/// The app's entry in the credential store.
#[derive(Debug)]
pub struct Vault {
    entry: Entry,
}

/// Whether the process-wide default store is installed. A failed install is
/// retried on the next open (a Secret Service may start after the app).
static INSTALLED: Mutex<bool> = Mutex::new(false);

fn ensure_store() -> Result<(), VaultError> {
    let mut installed = INSTALLED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !*installed {
        install_store()?;
        *installed = true;
    }
    drop(installed);
    Ok(())
}

fn decode_hex(text: &str) -> Option<Zeroizing<[u8; 32]>> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let mut key = Zeroizing::new([0_u8; 32]);
    for (slot, pair) in key.iter_mut().zip(bytes.chunks_exact(2)) {
        *slot = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(key)
}

fn encode_hex(key: &[u8; 32]) -> Zeroizing<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = Zeroizing::new(String::with_capacity(64));
    for byte in key {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    text
}

impl Vault {
    /// Open the vault of the app identified by `app_id` (e.g.
    /// `rs.sergioribera.myos`).
    pub fn open(app_id: &str) -> Result<Self, VaultError> {
        ensure_store()?;
        Ok(Self {
            entry: new_entry(app_id)?,
        })
    }

    /// Read the stored key (one read: no check-then-act race).
    pub fn load(&self) -> Result<Stored, VaultError> {
        match self.entry.get_password() {
            Ok(mut text) => {
                let key = decode_hex(&text);
                text.zeroize();
                key.map_or_else(
                    || self.lose(),
                    |mut bytes| Ok(Stored::Key(DatabaseKey::take(&mut bytes))),
                )
            }
            Err(StoreError::NoEntry) => Ok(Stored::Missing),
            // Undecryptable or non-UTF-8 value: the wrapping key is gone.
            Err(StoreError::BadDataFormat(..) | StoreError::BadEncoding(_)) => self.lose(),
            Err(other) => Err(other.into()),
        }
    }

    fn lose(&self) -> Result<Stored, VaultError> {
        self.forget()?;
        Ok(Stored::Lost)
    }

    /// Generate and store a new key, then return what the store holds (if
    /// another process stored one first, that one wins everywhere).
    pub fn create(&self) -> Result<DatabaseKey, VaultError> {
        let key = DatabaseKey::generate().map_err(VaultError::Generate)?;
        self.entry.set_password(&encode_hex(key.as_bytes()))?;
        match self.load()? {
            Stored::Key(stored) => Ok(stored),
            Stored::Missing | Stored::Lost => Err(VaultError::Store(StoreError::NoEntry)),
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

#[cfg(target_os = "windows")]
fn new_entry(app_id: &str) -> Result<Entry, StoreError> {
    // The default (`Enterprise`) roams with domain profiles.
    let modifiers = std::collections::HashMap::from([("persistence", "Local")]);
    Entry::new_with_modifiers(app_id, KEY_ACCOUNT, &modifiers)
}

#[cfg(not(any(target_os = "ios", target_os = "windows")))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_garbage() {
        let key = [0xab_u8; 32];
        let text = encode_hex(&key);
        assert_eq!(text.len(), 64);
        assert_eq!(decode_hex(&text).map(|bytes| *bytes), Some(key));
        assert!(decode_hex("ab").is_none());
        assert!(decode_hex(&"zz".repeat(32)).is_none());
        assert!(decode_hex(&"AB".repeat(32)).is_none());
    }
}
