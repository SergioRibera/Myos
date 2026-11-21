//! Database encryption key.

use std::fmt;

use zeroize::Zeroizing;

use crate::Error;

/// A 256-bit `SQLCipher` key, used raw (no key derivation: it is random, and
/// the KDF would cost hundreds of milliseconds per open on low-end phones).
///
/// The platform keeps it in its keystore; it is wiped from memory on drop.
pub struct DatabaseKey {
    bytes: Zeroizing<[u8; 32]>,
}

impl DatabaseKey {
    /// A fresh random key.
    pub fn generate() -> Result<Self, Error> {
        let mut bytes = Zeroizing::new([0_u8; 32]);
        getrandom::fill(bytes.as_mut_slice()).map_err(Error::Random)?;
        Ok(Self { bytes })
    }

    /// The raw bytes, to hand to the platform keystore.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// `x'<hex>'` raw-key literal, wiped on drop.
    pub(crate) fn raw_literal(&self) -> Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut literal = Zeroizing::new(String::with_capacity(67));
        literal.push_str("x'");
        for byte in self.bytes.iter() {
            literal.push(char::from(HEX[usize::from(byte >> 4)]));
            literal.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        literal.push('\'');
        literal
    }

    /// `PRAGMA key` statement for this key, wiped on drop.
    pub(crate) fn pragma(&self) -> Zeroizing<String> {
        let mut statement = Zeroizing::new(String::with_capacity(84));
        statement.push_str("PRAGMA key = \"");
        statement.push_str(&self.raw_literal());
        statement.push_str("\";");
        statement
    }
}

impl DatabaseKey {
    /// Take the key out of `bytes` (read back from the keystore), wiping the
    /// caller's buffer.
    pub fn take(bytes: &mut [u8; 32]) -> Self {
        let key = Self {
            bytes: Zeroizing::new(*bytes),
        };
        zeroize::Zeroize::zeroize(bytes);
        key
    }
}

// Manual impl: the key must never reach a log.
impl fmt::Debug for DatabaseKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DatabaseKey(<redacted>)")
    }
}

/// Shortest accepted passphrase (database fallback without a credential
/// store, `.myos` exports).
pub const MIN_PASSPHRASE_CHARS: usize = 8;

/// How a database file is unlocked.
#[derive(Clone, Copy)]
pub enum KeySource<'a> {
    /// A random key from the credential store, used raw.
    Raw(&'a DatabaseKey),
    /// A passphrase, stretched by `SQLCipher`'s KDF with the file's own salt
    /// (when no credential store exists, and for exports).
    Passphrase(&'a str),
}

impl<'a> From<&'a DatabaseKey> for KeySource<'a> {
    fn from(key: &'a DatabaseKey) -> Self {
        Self::Raw(key)
    }
}

// Manual impl: neither form may reach a log.
impl fmt::Debug for KeySource<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raw(_) => f.write_str("KeySource::Raw(<redacted>)"),
            Self::Passphrase(_) => f.write_str("KeySource::Passphrase(<redacted>)"),
        }
    }
}

impl KeySource<'_> {
    /// Reject passphrases that are too short.
    pub(crate) fn check(self) -> Result<Self, Error> {
        match self {
            Self::Passphrase(passphrase) if passphrase.chars().count() < MIN_PASSPHRASE_CHARS => {
                Err(Error::OutOfRange("passphrase"))
            }
            // SQLCipher reads `x'…'` as a raw key, not a passphrase.
            Self::Passphrase(passphrase)
                if passphrase
                    .get(..2)
                    .is_some_and(|start| start.eq_ignore_ascii_case("x'")) =>
            {
                Err(Error::OutOfRange("passphrase"))
            }
            other => Ok(other),
        }
    }

    /// Value bound as `KEY ?` of an `ATTACH` (the raw form is `SQLCipher`'s
    /// `x'…'` literal), wiped on drop.
    pub(crate) fn attach_value(self) -> Zeroizing<String> {
        match self {
            Self::Raw(key) => key.raw_literal(),
            Self::Passphrase(passphrase) => Zeroizing::new(passphrase.to_owned()),
        }
    }

    /// Unlock the main database of `conn`.
    pub(crate) fn apply(self, conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
        match self {
            Self::Raw(key) => conn.execute_batch(&key.pragma()),
            Self::Passphrase(passphrase) => conn.pragma_update(None, "key", passphrase),
        }
    }
}

impl DatabaseKey {
    /// `PRAGMA rekey` statement for this key, wiped on drop.
    pub(crate) fn rekey_pragma(&self) -> Zeroizing<String> {
        let mut statement = Zeroizing::new(String::with_capacity(86));
        statement.push_str("PRAGMA rekey = \"");
        statement.push_str(&self.raw_literal());
        statement.push_str("\";");
        statement
    }
}
