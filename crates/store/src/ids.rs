//! Deterministic ids: the same natural key yields the same row id on every
//! device holding the same user key, so independent inserts merge.

use hmac::{Hmac, KeyInit, Mac};
use myos_core::code::Code;
use sha2::Sha256;
use uuid::Uuid;

use crate::Entity;

/// Fixed id of the profile row (one per user; the server keys rows by user).
pub(crate) const PROFILE: Uuid = Uuid::from_u128(0x6d79_6f73_0000_8001_8000_0000_0000_0001);
/// Fixed id of the preferences row.
pub(crate) const PREFERENCES: Uuid = Uuid::from_u128(0x6d79_6f73_0000_8001_8000_0000_0000_0002);

/// Id for `entity` with natural key `parts`: UUID v8 over
/// `HMAC-SHA256(user_key, entity code ‖ (len ‖ part)*)`. Length prefixes keep
/// `("ab", "c")` and `("a", "bc")` apart; the layout is recomputable by the
/// server with pgcrypto.
pub(crate) fn natural(user_key: &[u8; 32], entity: Entity, parts: &[&[u8]]) -> Uuid {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(user_key)
        .expect("invariant: HMAC accepts keys of any length");
    mac.update(&entity.code().to_be_bytes());
    for part in parts {
        let len = u32::try_from(part.len()).unwrap_or(u32::MAX);
        mac.update(&len.to_be_bytes());
        mac.update(part);
    }
    let digest = mac.finalize().into_bytes();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::new_v8(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_key_same_id_and_parts_are_unambiguous() {
        let key = [3_u8; 32];
        let a = natural(&key, Entity::ExternalActivity, &[b"ab", b"c"]);
        let b = natural(&key, Entity::ExternalActivity, &[b"a", b"bc"]);
        assert_eq!(a, natural(&key, Entity::ExternalActivity, &[b"ab", b"c"]));
        assert_ne!(a, b);
        assert_ne!(
            a,
            natural(&[4; 32], Entity::ExternalActivity, &[b"ab", b"c"])
        );
        assert_ne!(a, natural(&key, Entity::ProfileGoal, &[b"ab", b"c"]));
        assert_eq!(a.get_version_num(), 8);
        assert_eq!(PROFILE.get_version_num(), 8);
        assert_eq!(PREFERENCES.get_version_num(), 8);
    }
}
