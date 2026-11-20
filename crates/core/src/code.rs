//! Stable storage codes for domain enums.
//!
//! Codes are what databases and sync payloads store. They are append-only and
//! never reused; 0 is reserved for "unspecified" and never maps to a variant.
//! A code this build does not know decodes to `None`, and the storage layer
//! keeps the raw value so it survives a round trip.

/// An enum with a stable storage code per variant.
pub trait Code: Sized + Copy {
    /// Storage code of the variant.
    fn code(self) -> u16;

    /// Variant for `code`, if this build knows it.
    fn from_code(code: u16) -> Option<Self>;
}

/// Implements [`Code`] from a `Variant = code` table. `code` is an exhaustive
/// match (a new variant without a code does not compile) and a duplicated code
/// is an unreachable pattern in `from_code`.
macro_rules! codes {
    ($ty:ty { $($variant:ident = $code:literal),+ $(,)? }) => {
        impl $crate::code::Code for $ty {
            fn code(self) -> u16 {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            fn from_code(code: u16) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

pub(crate) use codes;
