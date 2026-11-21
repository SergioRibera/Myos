//! Binary encoding of protocol messages (TLV, same codec as payloads).
//!
//! - `PushBatch`: 1 device id, 2 proto, 3 ops list; op: 1 op id, 2 entity,
//!   3 row id, 4 op, 5 hlc, 6 node, 7 payload version, 8 payload.
//! - `PushResponse`: 1 outcomes list; outcome: 1 op id, 2 kind (1 applied,
//!   2 superseded, 3 rejected, 4 merged), 3 revision, 4 code; 2 server time, 3 min proto.
//! - `PullPage`: 1 rows list; row: 1 entity, 2 row id, 3 op, 4 hlc, 5 node,
//!   6 revision, 7 payload version, 8 payload; 2 next, 3 more, 4 server time,
//!   5 min proto.
//!
//! Lists are records whose tag `n` (from 1) holds element `n`.

use uuid::Uuid;

use crate::protocol::{
    PullPage, PushBatch, PushOp, PushOutcome, PushResponse, PushResult, RemoteRow,
};
use crate::tlv::{Tlv, TlvError};

/// Largest encoded message accepted.
pub const MESSAGE_LIMIT: usize = 8 * 1024 * 1024;

/// Largest number of elements in one list.
const MAX_LIST: usize = 10_000;

fn put_list(
    record: &mut Tlv,
    tag: u16,
    items: impl IntoIterator<Item = Tlv>,
) -> Result<(), TlvError> {
    let mut list = Tlv::default();
    for (index, item) in items.into_iter().enumerate() {
        let position = u16::try_from(index + 1).map_err(|_| TlvError::TooLarge)?;
        list.put_record(position, &item);
    }
    record.put_record(tag, &list);
    Ok(())
}

fn list(record: &Tlv, tag: u16) -> Result<Vec<Tlv>, TlvError> {
    let Some(list) = record.record(tag, MESSAGE_LIMIT)? else {
        return Ok(Vec::new());
    };
    let items: Vec<Tlv> = list
        .tags()
        .map(|position| {
            list.record(position, MESSAGE_LIMIT)?
                .ok_or(TlvError::Malformed { tag: position })
        })
        .collect::<Result<_, _>>()?;
    if items.len() > MAX_LIST {
        return Err(TlvError::TooLarge);
    }
    Ok(items)
}

fn uuid(record: &Tlv, tag: u16) -> Result<Uuid, TlvError> {
    record
        .bytes(tag)
        .and_then(|bytes| Uuid::from_slice(bytes).ok())
        .ok_or(TlvError::Malformed { tag })
}

fn required<T>(value: Option<T>, tag: u16) -> Result<T, TlvError> {
    value.ok_or(TlvError::Malformed { tag })
}

fn narrow<T: TryFrom<u64>>(record: &Tlv, tag: u16) -> Result<T, TlvError> {
    T::try_from(required(record.uint(tag)?, tag)?).map_err(|_| TlvError::Malformed { tag })
}

fn decode(bytes: &[u8]) -> Result<Tlv, TlvError> {
    Tlv::decode(bytes, MESSAGE_LIMIT)
}

impl PushBatch {
    /// Wire bytes.
    pub fn encode(&self) -> Result<Vec<u8>, TlvError> {
        let mut record = Tlv::default();
        record
            .put_bytes(1, self.device_id.as_bytes())
            .put_uint(2, u64::from(self.proto));
        put_list(
            &mut record,
            3,
            self.ops.iter().map(|op| {
                let mut item = Tlv::default();
                item.put_bytes(1, op.op_id.as_bytes())
                    .put_uint(2, u64::from(op.entity))
                    .put_bytes(3, op.row_id.as_bytes())
                    .put_uint(4, u64::from(op.op))
                    .put_uint(5, op.hlc)
                    .put_int(6, op.node)
                    .put_uint(7, u64::from(op.payload_ver))
                    .put_bytes(8, &op.payload);
                item
            }),
        )?;
        Ok(record.encode())
    }

    /// Parse wire bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, TlvError> {
        let record = decode(bytes)?;
        let ops = list(&record, 3)?
            .iter()
            .map(|item| {
                Ok(PushOp {
                    op_id: uuid(item, 1)?,
                    entity: narrow(item, 2)?,
                    row_id: uuid(item, 3)?,
                    op: narrow(item, 4)?,
                    hlc: required(item.uint(5)?, 5)?,
                    node: required(item.int(6)?, 6)?,
                    payload_ver: narrow(item, 7)?,
                    payload: required(item.bytes(8), 8)?.to_vec(),
                })
            })
            .collect::<Result<_, TlvError>>()?;
        Ok(Self {
            device_id: uuid(&record, 1)?,
            proto: narrow(&record, 2)?,
            ops,
        })
    }
}

impl PushResponse {
    /// Wire bytes.
    pub fn encode(&self) -> Result<Vec<u8>, TlvError> {
        let mut record = Tlv::default();
        put_list(
            &mut record,
            1,
            self.outcomes.iter().map(|outcome| {
                let mut item = Tlv::default();
                item.put_bytes(1, outcome.op_id.as_bytes());
                match outcome.result {
                    PushResult::Applied { server_rev } => {
                        item.put_uint(2, 1).put_int(3, server_rev);
                    }
                    PushResult::Superseded { server_rev } => {
                        item.put_uint(2, 2).put_int(3, server_rev);
                    }
                    PushResult::Rejected { code } => {
                        item.put_uint(2, 3).put_uint(4, u64::from(code));
                    }
                    PushResult::Merged { server_rev } => {
                        item.put_uint(2, 4).put_int(3, server_rev);
                    }
                }
                item
            }),
        )?;
        record
            .put_int(2, self.server_time_ms)
            .put_uint(3, u64::from(self.min_proto));
        Ok(record.encode())
    }

    /// Parse wire bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, TlvError> {
        let record = decode(bytes)?;
        let outcomes = list(&record, 1)?
            .iter()
            .map(|item| {
                let result = match required(item.uint(2)?, 2)? {
                    1 => PushResult::Applied {
                        server_rev: required(item.int(3)?, 3)?,
                    },
                    2 => PushResult::Superseded {
                        server_rev: required(item.int(3)?, 3)?,
                    },
                    3 => PushResult::Rejected {
                        code: narrow(item, 4)?,
                    },
                    4 => PushResult::Merged {
                        server_rev: required(item.int(3)?, 3)?,
                    },
                    _ => return Err(TlvError::Malformed { tag: 2 }),
                };
                Ok(PushOutcome {
                    op_id: uuid(item, 1)?,
                    result,
                })
            })
            .collect::<Result<_, TlvError>>()?;
        Ok(Self {
            outcomes,
            server_time_ms: required(record.int(2)?, 2)?,
            min_proto: narrow(&record, 3)?,
        })
    }
}

impl PullPage {
    /// Wire bytes.
    pub fn encode(&self) -> Result<Vec<u8>, TlvError> {
        let mut record = Tlv::default();
        put_list(
            &mut record,
            1,
            self.rows.iter().map(|row| {
                let mut item = Tlv::default();
                item.put_uint(1, u64::from(row.entity))
                    .put_bytes(2, row.row_id.as_bytes())
                    .put_uint(3, u64::from(row.op))
                    .put_uint(4, row.hlc)
                    .put_int(5, row.node)
                    .put_int(6, row.server_rev)
                    .put_uint(7, u64::from(row.payload_ver))
                    .put_bytes(8, &row.payload);
                item
            }),
        )?;
        record
            .put_int(2, self.next)
            .put_uint(3, u64::from(self.more))
            .put_int(4, self.server_time_ms)
            .put_uint(5, u64::from(self.min_proto));
        Ok(record.encode())
    }

    /// Parse wire bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, TlvError> {
        let record = decode(bytes)?;
        let rows = list(&record, 1)?
            .iter()
            .map(|item| {
                Ok(RemoteRow {
                    entity: narrow(item, 1)?,
                    row_id: uuid(item, 2)?,
                    op: narrow(item, 3)?,
                    hlc: required(item.uint(4)?, 4)?,
                    node: required(item.int(5)?, 5)?,
                    server_rev: required(item.int(6)?, 6)?,
                    payload_ver: narrow(item, 7)?,
                    payload: required(item.bytes(8), 8)?.to_vec(),
                })
            })
            .collect::<Result<_, TlvError>>()?;
        Ok(Self {
            rows,
            next: required(record.int(2)?, 2)?,
            more: required(record.uint(3)?, 3)? != 0,
            server_time_ms: required(record.int(4)?, 4)?,
            min_proto: narrow(&record, 5)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() -> Result<(), TlvError> {
        let batch = PushBatch {
            device_id: Uuid::from_u128(6),
            proto: 1,
            ops: vec![PushOp {
                op_id: Uuid::from_u128(9),
                entity: 2,
                row_id: Uuid::from_u128(11),
                op: 1,
                hlc: u64::MAX >> 1,
                node: -5,
                payload_ver: 1,
                payload: vec![1, 2, 3],
            }],
        };
        assert_eq!(PushBatch::decode(&batch.encode()?)?, batch);

        let response = PushResponse {
            outcomes: vec![
                PushOutcome {
                    op_id: Uuid::from_u128(24),
                    result: PushResult::Applied { server_rev: 7 },
                },
                PushOutcome {
                    op_id: Uuid::from_u128(28),
                    result: PushResult::Superseded { server_rev: 3 },
                },
                PushOutcome {
                    op_id: Uuid::from_u128(32),
                    result: PushResult::Rejected { code: 2 },
                },
                PushOutcome {
                    op_id: Uuid::from_u128(33),
                    result: PushResult::Merged { server_rev: 8 },
                },
            ],
            server_time_ms: 1_795_000_000_000,
            min_proto: 1,
        };
        assert_eq!(PushResponse::decode(&response.encode()?)?, response);

        let page = PullPage {
            rows: vec![RemoteRow {
                entity: 1,
                row_id: Uuid::from_u128(44),
                op: 2,
                hlc: 42,
                node: 9,
                server_rev: 11,
                payload_ver: 1,
                payload: vec![],
            }],
            next: 11,
            more: true,
            server_time_ms: 5,
            min_proto: 1,
        };
        assert_eq!(PullPage::decode(&page.encode()?)?, page);
        assert_eq!(
            PullPage::decode(
                &PullPage {
                    rows: vec![],
                    ..page
                }
                .encode()?
            )?
            .rows,
            vec![]
        );
        Ok(())
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(PushBatch::decode(&[1, 2, 3]).is_err());
        assert!(PullPage::decode(&[]).is_err());
    }
}
