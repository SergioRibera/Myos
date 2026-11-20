//! `myos-core` — the Myos domain model and muscle-impact engine.
//!
//! Pure Rust, no I/O: storage, sync and UI build on top of these types.
//!
//! - [`muscle`]: the muscle taxonomy shared with the anatomical map.
//! - [`exercise`]: exercises and how they involve each muscle.
//! - [`session`]: raw, immutable sets as the user logged them.
//! - [`load`]: weighted load (versioned), the per-set metric behind the map.
//! - [`strength`]: estimated one-rep max.
//! - [`volume`]: per-muscle, per-side totals over a period.

pub mod exercise;
pub mod id;
pub mod load;
pub mod muscle;
pub mod session;
pub mod strength;
pub mod units;
pub mod volume;
