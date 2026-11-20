//! `myos-core` — the Myos domain model and muscle-impact engine.
//!
//! Pure Rust, no I/O: storage, sync and UI build on top of these types.
//!
//! - [`muscle`]: the muscle taxonomy shared with the anatomical map.
//! - [`exercise`]: exercises and how they involve each muscle.
//! - [`session`]: sets and sessions with the raw values the user logged.
//! - [`profile`]: profile, goals and preferences.
//! - [`body`]: body measurements.
//! - [`diet`]: meals, foods and nutrients.
//! - [`routine`]: routine templates and their versions.
//! - [`code`]: stable storage codes for the enums above.
//! - [`load`]: weighted load (versioned), the per-set metric behind the map.
//! - [`strength`]: estimated one-rep max.
//! - [`volume`]: per-muscle, per-side totals over a period.

pub mod body;
pub mod code;
pub mod diet;
pub mod exercise;
pub mod id;
pub mod load;
pub mod muscle;
pub mod profile;
pub mod routine;
pub mod session;
pub mod strength;
pub mod units;
pub mod volume;
