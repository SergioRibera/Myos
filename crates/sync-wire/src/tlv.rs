//! Tag-length-value records: the format of every structured blob the store
//! writes (sync payloads, nutrient densities, routine content, heart-rate
//! zones).
//!
//! Layout: a sequence of `tag (varint) · length (varint) · bytes`, tags
//! strictly increasing. Tags are append-only per record type and never
//! reused, so an older reader skips what it does not know and a newer
//! reader treats a missing tag as "not set". Integers inside values are
//! LEB128 varints (signed ones zig-zag encoded).

use std::collections::BTreeMap;
use std::fmt;

/// Decoding failure. The input is untrusted (it may come from the network).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TlvError {
    /// The input ends inside a field.
    Truncated,
    /// The input is larger than the caller's limit.
    TooLarge,
    /// A varint does not fit its type.
    Overflow,
    /// A varint uses more bytes than needed, or tag 0 (reserved) is used.
    NonCanonical,
    /// Tags are not strictly increasing.
    Unordered {
        /// Tag that broke the order.
        tag: u16,
    },
    /// A field does not hold the expected type.
    Malformed {
        /// Field with the bad value.
        tag: u16,
    },
}

impl fmt::Display for TlvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("record is truncated"),
            Self::TooLarge => f.write_str("record exceeds the size limit"),
            Self::Overflow => f.write_str("integer overflows its type"),
            Self::NonCanonical => f.write_str("non-canonical encoding"),
            Self::Unordered { tag } => write!(f, "tag {tag} is out of order"),
            Self::Malformed { tag } => write!(f, "tag {tag} holds a malformed value"),
        }
    }
}

impl std::error::Error for TlvError {}

/// A decoded record: tag → raw value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tlv {
    fields: BTreeMap<u16, Vec<u8>>,
}

impl Tlv {
    /// Default size limit for [`Tlv::decode`].
    pub const DEFAULT_LIMIT: usize = 256 * 1024;

    /// Set `tag` to an unsigned integer.
    pub fn put_uint(&mut self, tag: u16, value: u64) -> &mut Self {
        let mut bytes = Vec::with_capacity(10);
        write_varint(&mut bytes, value);
        self.fields.insert(tag, bytes);
        self
    }

    /// Set `tag` to a signed integer.
    pub fn put_int(&mut self, tag: u16, value: i64) -> &mut Self {
        self.put_uint(tag, zigzag(value))
    }

    /// Set `tag` to raw bytes.
    pub fn put_bytes(&mut self, tag: u16, value: &[u8]) -> &mut Self {
        self.fields.insert(tag, value.to_vec());
        self
    }

    /// Set `tag` to UTF-8 text.
    pub fn put_text(&mut self, tag: u16, value: &str) -> &mut Self {
        self.put_bytes(tag, value.as_bytes())
    }

    /// Set `tag` to a nested record.
    pub fn put_record(&mut self, tag: u16, value: &Self) -> &mut Self {
        self.fields.insert(tag, value.encode());
        self
    }

    /// Set `tag` when `value` is `Some`, clear it otherwise.
    pub fn put_opt_uint(&mut self, tag: u16, value: Option<u64>) -> &mut Self {
        match value {
            Some(value) => self.put_uint(tag, value),
            None => self.remove(tag),
        }
    }

    /// Set `tag` when `value` is `Some`, clear it otherwise.
    pub fn put_opt_int(&mut self, tag: u16, value: Option<i64>) -> &mut Self {
        match value {
            Some(value) => self.put_int(tag, value),
            None => self.remove(tag),
        }
    }

    /// Clear `tag`.
    pub fn remove(&mut self, tag: u16) -> &mut Self {
        self.fields.remove(&tag);
        self
    }

    /// Whether `tag` is set.
    #[must_use]
    pub fn contains(&self, tag: u16) -> bool {
        self.fields.contains_key(&tag)
    }

    /// Raw bytes of `tag`.
    #[must_use]
    pub fn bytes(&self, tag: u16) -> Option<&[u8]> {
        self.fields.get(&tag).map(Vec::as_slice)
    }

    /// Unsigned integer at `tag`.
    pub fn uint(&self, tag: u16) -> Result<Option<u64>, TlvError> {
        self.bytes(tag)
            .map(|bytes| {
                let mut cursor = bytes;
                let value = read_varint(&mut cursor).map_err(|_| TlvError::Malformed { tag })?;
                if cursor.is_empty() {
                    Ok(value)
                } else {
                    Err(TlvError::Malformed { tag })
                }
            })
            .transpose()
    }

    /// Signed integer at `tag`.
    pub fn int(&self, tag: u16) -> Result<Option<i64>, TlvError> {
        Ok(self.uint(tag)?.map(unzigzag))
    }

    /// Text at `tag`.
    pub fn text(&self, tag: u16) -> Result<Option<&str>, TlvError> {
        self.bytes(tag)
            .map(|bytes| std::str::from_utf8(bytes).map_err(|_| TlvError::Malformed { tag }))
            .transpose()
    }

    /// Nested record at `tag`.
    pub fn record(&self, tag: u16, limit: usize) -> Result<Option<Self>, TlvError> {
        self.bytes(tag)
            .map(|bytes| Self::decode(bytes, limit).map_err(|_| TlvError::Malformed { tag }))
            .transpose()
    }

    /// Set tags, in order.
    pub fn tags(&self) -> impl Iterator<Item = u16> + '_ {
        self.fields.keys().copied()
    }

    /// Fields whose tag is not in `known` (to keep what a newer version wrote).
    #[must_use]
    pub fn unknown(&self, known: &[u16]) -> Self {
        Self {
            fields: self
                .fields
                .iter()
                .filter(|(tag, _)| !known.contains(tag))
                .map(|(tag, value)| (*tag, value.clone()))
                .collect(),
        }
    }

    /// Add every field of `other` that `self` does not set.
    pub fn fill_from(&mut self, other: &Self) {
        for (tag, value) in &other.fields {
            self.fields.entry(*tag).or_insert_with(|| value.clone());
        }
    }

    /// Whether no tag is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (tag, value) in &self.fields {
            write_varint(&mut out, u64::from(*tag));
            write_varint(&mut out, value.len() as u64);
            out.extend_from_slice(value);
        }
        out
    }

    /// Parse `input`, refusing more than `limit` bytes.
    pub fn decode(input: &[u8], limit: usize) -> Result<Self, TlvError> {
        if input.len() > limit {
            return Err(TlvError::TooLarge);
        }
        let mut cursor = input;
        let mut fields = BTreeMap::new();
        let mut previous: Option<u16> = None;
        while !cursor.is_empty() {
            let tag = u16::try_from(read_varint(&mut cursor)?).map_err(|_| TlvError::Overflow)?;
            if tag == 0 {
                return Err(TlvError::NonCanonical);
            }
            if previous.is_some_and(|previous| tag <= previous) {
                return Err(TlvError::Unordered { tag });
            }
            previous = Some(tag);
            let len = usize::try_from(read_varint(&mut cursor)?).map_err(|_| TlvError::Overflow)?;
            let Some((value, rest)) = cursor.split_at_checked(len) else {
                return Err(TlvError::Truncated);
            };
            fields.insert(tag, value.to_vec());
            cursor = rest;
        }
        Ok(Self { fields })
    }
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        // Low 7 bits, continuation flag when more remain.
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_varint(cursor: &mut &[u8]) -> Result<u64, TlvError> {
    let mut value: u64 = 0;
    for shift in (0..64).step_by(7) {
        let Some((&byte, rest)) = cursor.split_first() else {
            return Err(TlvError::Truncated);
        };
        *cursor = rest;
        let bits = u64::from(byte & 0x7f);
        if shift == 63 && bits > 1 {
            return Err(TlvError::Overflow);
        }
        value |= bits << shift;
        if byte & 0x80 == 0 {
            // Canonical form only: a zero final byte after the first would
            // encode the same value in more bytes.
            if shift > 0 && byte == 0 {
                return Err(TlvError::NonCanonical);
            }
            return Ok(value);
        }
    }
    Err(TlvError::Overflow)
}

const fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)).cast_unsigned()
}

const fn unzigzag(value: u64) -> i64 {
    (value >> 1).cast_signed() ^ -((value & 1).cast_signed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_type() -> Result<(), TlvError> {
        let mut nested = Tlv::default();
        nested.put_uint(1, 7);
        let mut record = Tlv::default();
        record
            .put_uint(1, u64::MAX)
            .put_int(2, i64::MIN)
            .put_int(3, -1)
            .put_text(4, "año ñandú")
            .put_bytes(5, &[0, 255])
            .put_record(6, &nested)
            .put_opt_uint(7, None);
        let decoded = Tlv::decode(&record.encode(), Tlv::DEFAULT_LIMIT)?;
        assert_eq!(decoded, record);
        assert_eq!(decoded.uint(1)?, Some(u64::MAX));
        assert_eq!(decoded.int(2)?, Some(i64::MIN));
        assert_eq!(decoded.int(3)?, Some(-1));
        assert_eq!(decoded.text(4)?, Some("año ñandú"));
        assert_eq!(decoded.record(6, 64)?, Some(nested));
        assert_eq!(decoded.uint(7)?, None);
        Ok(())
    }

    #[test]
    fn rejects_hostile_input() {
        assert_eq!(Tlv::decode(&[1, 5, 0], 64), Err(TlvError::Truncated));
        assert_eq!(Tlv::decode(&[1], 64), Err(TlvError::Truncated));
        assert_eq!(
            Tlv::decode(&[2, 0, 1, 0], 64),
            Err(TlvError::Unordered { tag: 1 })
        );
        assert_eq!(
            Tlv::decode(&[1, 0, 1, 0], 64),
            Err(TlvError::Unordered { tag: 1 })
        );
        assert_eq!(Tlv::decode(&[0; 65], 64), Err(TlvError::TooLarge));
        let huge = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f];
        assert_eq!(Tlv::decode(&huge, 64), Err(TlvError::Overflow));
        // Tag above u16.
        assert_eq!(
            Tlv::decode(&[0x80, 0x80, 0x04, 0], 64),
            Err(TlvError::Overflow)
        );
    }

    #[test]
    fn rejects_non_canonical_varints_and_tag_zero() {
        // 1 encoded in two bytes.
        assert_eq!(
            Tlv::decode(&[0x81, 0x00, 0], 64),
            Err(TlvError::NonCanonical)
        );
        assert_eq!(Tlv::decode(&[0, 0], 64), Err(TlvError::NonCanonical));
        // Largest u64: nine 0xff then 0x01 is canonical.
        let mut max = vec![1_u8];
        max.extend([0xff; 9]);
        max.push(0x01);
        max.push(0);
        assert!(
            Tlv::decode(&max, 64).is_err(),
            "tag above u16 still overflows"
        );
        let mut record = Tlv::default();
        record.put_uint(1, u64::MAX);
        assert_eq!(
            Tlv::decode(&record.encode(), 64).map(|r| r.uint(1)),
            Ok(Ok(Some(u64::MAX)))
        );
    }

    #[test]
    fn typed_reads_reject_wrong_shapes() {
        let mut record = Tlv::default();
        record
            .put_bytes(1, &[0x80])
            .put_bytes(2, &[1, 2])
            .put_bytes(3, &[0xff]);
        assert_eq!(record.uint(1), Err(TlvError::Malformed { tag: 1 }));
        assert_eq!(record.uint(2), Err(TlvError::Malformed { tag: 2 }));
        assert_eq!(record.text(3), Err(TlvError::Malformed { tag: 3 }));
    }

    #[test]
    fn keeps_unknown_fields() {
        let mut newer = Tlv::default();
        newer.put_uint(1, 1).put_uint(2, 2).put_uint(9, 9);
        let extra = newer.unknown(&[1, 2]);
        assert_eq!(extra.tags().collect::<Vec<_>>(), vec![9]);
        let mut rebuilt = Tlv::default();
        rebuilt.put_uint(1, 10).put_uint(2, 20);
        rebuilt.fill_from(&extra);
        assert_eq!(rebuilt.tags().collect::<Vec<_>>(), vec![1, 2, 9]);
        assert_eq!(rebuilt.uint(1), Ok(Some(10)));
    }
}
