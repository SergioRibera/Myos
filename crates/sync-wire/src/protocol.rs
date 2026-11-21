//! Protocol messages exchanged between devices and the server.

use uuid::Uuid;

/// Protocol version this build speaks.
pub const PROTO: u32 = 1;
/// Version of the payload layout.
pub const PAYLOAD_VERSION: u16 = 1;
/// Own data stream.
pub const STREAM_OWN: u16 = 1;

/// Operation codes.
pub mod op {
    /// Created or changed.
    pub const UPSERT: u16 = 1;
    /// Tombstoned.
    pub const DELETE: u16 = 2;
    /// Tombstone lifted on purpose (deletes otherwise win).
    pub const RESTORE: u16 = 3;
}

/// One outbox row sent to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushOp {
    /// Idempotency key.
    pub op_id: Uuid,
    /// Entity code.
    pub entity: u16,
    /// Row id.
    pub row_id: Uuid,
    /// Operation code.
    pub op: u16,
    /// Clock of the write.
    pub hlc: u64,
    /// Writer node (signed on both sides).
    pub node: i64,
    /// Payload layout version.
    pub payload_ver: u16,
    /// Row payload (TLV).
    pub payload: Vec<u8>,
}

/// A push request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushBatch {
    /// Sending device.
    pub device_id: Uuid,
    /// Protocol version.
    pub proto: u32,
    /// Rows, in queue order.
    pub ops: Vec<PushOp>,
}

/// What the server did with one op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushResult {
    /// Stored as the current version of the row.
    Applied {
        /// Revision assigned.
        server_rev: i64,
    },
    /// Stored after merging with fields of other devices: the stored
    /// version differs from the op and arrives by pull.
    Merged {
        /// Revision assigned.
        server_rev: i64,
    },
    /// The server holds a version that wins; it arrives by pull.
    Superseded {
        /// Revision of the winning version.
        server_rev: i64,
    },
    /// Refused (validation, permissions); kept locally as failed.
    Rejected {
        /// Reason code.
        code: u16,
    },
}

/// Answer for one op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushOutcome {
    /// The op answered.
    pub op_id: Uuid,
    /// What happened.
    pub result: PushResult,
}

/// A push response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushResponse {
    /// One outcome per op (missing ones are retried).
    pub outcomes: Vec<PushOutcome>,
    /// Server clock, Unix ms.
    pub server_time_ms: i64,
    /// Oldest protocol the server still accepts writes from.
    pub min_proto: u32,
}

/// One row of the change feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRow {
    /// Entity code.
    pub entity: u16,
    /// Row id.
    pub row_id: Uuid,
    /// Operation code of the version.
    pub op: u16,
    /// Clock of the version.
    pub hlc: u64,
    /// Writer node.
    pub node: i64,
    /// Revision (unique per row version, increasing).
    pub server_rev: i64,
    /// Payload layout version.
    pub payload_ver: u16,
    /// Row payload (TLV).
    pub payload: Vec<u8>,
}

/// A page of the change feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullPage {
    /// Rows, by increasing revision.
    pub rows: Vec<RemoteRow>,
    /// Cursor for the next page.
    pub next: i64,
    /// Whether more rows follow.
    pub more: bool,
    /// Server clock, Unix ms.
    pub server_time_ms: i64,
    /// Oldest protocol the server still accepts writes from.
    pub min_proto: u32,
}
