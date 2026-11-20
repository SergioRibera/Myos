//! Domain values ↔ stored integers.

use std::time::Duration;

use jiff::ToSpan;
use jiff::civil::Date;

use crate::Error;

/// Days are stored as days since 1970-01-01, within `0 ..= 73000` (~2169).
pub(crate) const MAX_DAY: i64 = 73_000;

const EPOCH: Date = jiff::civil::date(1970, 1, 1);

/// Stored day number of `day`.
pub(crate) fn day_number(day: Date) -> Result<i64, Error> {
    let days = i64::from(
        day.since(EPOCH)
            .map_err(|_| Error::OutOfRange("day"))?
            .get_days(),
    );
    if (0..=MAX_DAY).contains(&days) {
        Ok(days)
    } else {
        Err(Error::OutOfRange("day"))
    }
}

/// Day for a stored day number.
pub(crate) fn day_from_number(days: i64) -> Result<Date, Error> {
    if !(0..=MAX_DAY).contains(&days) {
        return Err(Error::OutOfRange("day"));
    }
    EPOCH
        .checked_add(days.days())
        .map_err(|_| Error::OutOfRange("day"))
}

/// Whole milliseconds of `duration`.
pub(crate) fn millis(duration: Duration) -> Result<i64, Error> {
    i64::try_from(duration.as_millis()).map_err(|_| Error::OutOfRange("duration"))
}

/// Duration for stored milliseconds.
pub(crate) fn duration(millis: i64) -> Result<Duration, Error> {
    u64::try_from(millis)
        .map(Duration::from_millis)
        .map_err(|_| Error::OutOfRange("duration"))
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    #[test]
    fn day_numbers_round_trip() {
        assert_eq!(day_number(date(1970, 1, 1)).ok(), Some(0));
        assert_eq!(day_number(date(1970, 1, 8)).ok(), Some(7));
        let today = date(2026, 11, 20);
        let number = day_number(today).ok();
        assert_eq!(number.and_then(|n| day_from_number(n).ok()), Some(today));
        assert!(day_number(date(1969, 12, 31)).is_err());
        assert!(day_from_number(MAX_DAY + 1).is_err());
    }
}
