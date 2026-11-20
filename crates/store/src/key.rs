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

    /// `PRAGMA key` statement for this key, wiped on drop.
    pub(crate) fn pragma(&self) -> Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut statement = Zeroizing::new(String::with_capacity(84));
        statement.push_str("PRAGMA key = \"x'");
        for byte in self.bytes.iter() {
            statement.push(char::from(HEX[usize::from(byte >> 4)]));
            statement.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        statement.push_str("'\";");
        statement
    }
}

impl From<[u8; 32]> for DatabaseKey {
    fn from(bytes: [u8; 32]) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
        }
    }
}

// Manual impl: the key must never reach a log.
impl fmt::Debug for DatabaseKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DatabaseKey(<redacted>)")
    }
}
