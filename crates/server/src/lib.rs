//! `myos-server`: the sync server. Devices push their outbox and pull the
//! change feed; the store is `PostgreSQL` with row-level security per user,
//! and conflicts follow the rules in [`myos_sync_wire::merge`], the same the
//! devices apply.

pub mod auth;
pub mod db;
pub mod http;
