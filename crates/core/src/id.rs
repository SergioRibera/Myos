//! Typed identifiers.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use uuid::Uuid;

/// Stable Myos identifier of a `T`.
///
/// UUID v7: minted offline on any device, time-ordered, never reused. The
/// marker keeps an exercise id from being passed where a session id is
/// expected.
pub struct Id<T> {
    uuid: Uuid,
    marker: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    /// A new id for a record created now.
    #[must_use]
    pub fn generate() -> Self {
        Self::from(Uuid::now_v7())
    }

    /// The underlying UUID, for storage and the wire.
    #[must_use]
    pub const fn uuid(self) -> Uuid {
        self.uuid
    }
}

impl<T> From<Uuid> for Id<T> {
    fn from(uuid: Uuid) -> Self {
        Self {
            uuid,
            marker: PhantomData,
        }
    }
}

// Manual impls: derives would require `T: Clone + Eq + …`, and the marker
// types (`Exercise`, `Session`) are not meant to implement them.
impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.uuid == other.uuid
    }
}

impl<T> Eq for Id<T> {}

impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.uuid.cmp(&other.uuid)
    }
}

impl<T> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.uuid.hash(state);
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.uuid)
    }
}

impl<T> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.uuid.fmt(f)
    }
}
