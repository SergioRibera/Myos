//! `myos-sync-wire`: what devices and the server agree on, without I/O.
//!
//! - [`tlv`]: the tag-length-value codec of every payload.
//! - [`protocol`]: push / pull messages.
//! - [`merge`]: the conflict rules both sides apply.
//! - [`message`]: binary encoding of the messages for transports.

pub mod merge;
pub mod message;
pub mod protocol;
pub mod tlv;
