//! Hybrid logical clock: orders writes across devices for last-writer-wins.

/// A timestamp: 48 bits of Unix milliseconds and a 16-bit counter that orders
/// writes within the same millisecond (or while the wall clock is behind).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc(u64);

impl Hlc {
    const COUNTER_BITS: u32 = 16;
    const COUNTER_MASK: u64 = (1 << Self::COUNTER_BITS) - 1;
    const MAX_MILLIS: u64 = (1 << 48) - 1;

    /// The value stored in the database.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// From a stored value.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Wall-clock part, Unix milliseconds.
    #[must_use]
    pub const fn millis(self) -> u64 {
        self.0 >> Self::COUNTER_BITS
    }

    /// Counter part.
    #[must_use]
    pub const fn counter(self) -> u16 {
        (self.0 & Self::COUNTER_MASK) as u16
    }

    const fn compose(millis: u64, counter: u64) -> Self {
        let millis = if millis > Self::MAX_MILLIS {
            Self::MAX_MILLIS
        } else {
            millis
        };
        Self((millis << Self::COUNTER_BITS) | counter)
    }

    /// Timestamp for a local write after `self`, at wall time `now_millis`.
    /// Always greater than `self`, even when the wall clock went backwards.
    #[must_use]
    pub const fn tick(self, now_millis: u64) -> Self {
        let last = self.millis();
        if now_millis > last {
            return Self::compose(now_millis, 0);
        }
        let counter = (self.0 & Self::COUNTER_MASK) + 1;
        if counter > Self::COUNTER_MASK {
            Self::compose(last + 1, 0)
        } else {
            Self::compose(last, counter)
        }
    }

    /// Clock after receiving `remote` at wall time `now_millis`: the next
    /// local write orders after both.
    #[must_use]
    pub fn receive(self, remote: Self, now_millis: u64) -> Self {
        self.max(remote).tick(now_millis)
    }
}

#[cfg(test)]
mod tests {
    use super::Hlc;

    #[test]
    fn tick_follows_wall_clock() {
        let first = Hlc::default().tick(1_000);
        assert_eq!((first.millis(), first.counter()), (1_000, 0));
        let later = first.tick(2_000);
        assert_eq!((later.millis(), later.counter()), (2_000, 0));
    }

    #[test]
    fn tick_stays_monotonic_when_clock_stalls_or_goes_back() {
        let first = Hlc::default().tick(5_000);
        let same_ms = first.tick(5_000);
        let backwards = same_ms.tick(4_000);
        assert!(same_ms > first);
        assert!(backwards > same_ms);
        assert_eq!((backwards.millis(), backwards.counter()), (5_000, 2));
    }

    #[test]
    fn counter_overflow_carries_into_millis() {
        let full = Hlc::from_raw((7_u64 << 16) | 0xffff);
        let next = full.tick(0);
        assert_eq!((next.millis(), next.counter()), (8, 0));
    }

    #[test]
    fn receive_orders_after_remote() {
        let local = Hlc::default().tick(1_000);
        let remote = Hlc::default().tick(9_000);
        let merged = local.receive(remote, 2_000);
        assert!(merged > remote);
    }
}
