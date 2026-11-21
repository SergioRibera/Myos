//! The sync store in `PostgreSQL`.

use std::time::{SystemTime, UNIX_EPOCH};

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use myos_sync_wire::merge::{self, Outcome, Version};
use myos_sync_wire::protocol::{
    PullPage, PushBatch, PushOp, PushOutcome, PushResult, RemoteRow, op,
};
use myos_sync_wire::tlv::Tlv;
use tokio_postgres::{NoTls, Transaction};
use uuid::Uuid;

/// Migrations, in order. Never edit a shipped entry: append.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_sync.sql")];

/// Largest payload of one row.
const PAYLOAD_LIMIT: usize = 1024 * 1024;
/// Largest number of ops in one push.
pub const MAX_OPS: usize = 500;
/// Largest pull page.
pub const MAX_PAGE: u32 = 1000;
/// Clocks further ahead of the server than this are refused.
const MAX_AHEAD_MS: u64 = 60 * 1000;
/// Receipts older than this are purged.
const RECEIPT_TTL_DAYS: i32 = 30;

/// Reasons a pushed op is refused.
pub mod reject {
    /// The op's clock is ahead of the server clock.
    pub const FUTURE_CLOCK: u16 = 1;
    /// Malformed op or payload.
    pub const INVALID: u16 = 2;
    /// Payload too large.
    pub const TOO_LARGE: u16 = 3;
}

/// Outcome codes stored in `push_receipt.outcome`.
const APPLIED: i16 = 1;
const SUPERSEDED: i16 = 2;
const REJECTED: i16 = 3;
const MERGED: i16 = 4;

/// Database failure.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// `PostgreSQL` reported an error.
    #[error("database error")]
    Postgres(#[from] tokio_postgres::Error),
    /// No connection could be taken from the pool.
    #[error("database pool error")]
    Pool(#[from] deadpool_postgres::PoolError),
    /// The pool could not be built.
    #[error("database pool setup failed")]
    Build(#[from] deadpool_postgres::BuildError),
    /// A stored row is corrupt.
    #[error("stored row is malformed")]
    Corrupt,
}

/// Server clock, Unix ms.
#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// Connection pool to the sync store.
#[derive(Clone)]
pub struct Db {
    pool: Pool,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db").finish_non_exhaustive()
    }
}

/// Bind every statement of `tx` to `user` (row-level security).
async fn act_as(tx: &Transaction<'_>, user: Uuid) -> Result<(), DbError> {
    tx.execute(
        "SELECT set_config('app.user_id', $1, true)",
        &[&user.to_string()],
    )
    .await?;
    Ok(())
}

impl Db {
    /// Pool over `config` (TLS is terminated in front of the server or the
    /// database is on a local socket).
    pub fn new(config: tokio_postgres::Config, max_connections: usize) -> Result<Self, DbError> {
        let manager = Manager::from_config(
            config,
            NoTls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(manager).max_size(max_connections).build()?;
        Ok(Self { pool })
    }

    /// Apply pending migrations (serialised across server instances).
    pub async fn migrate(&self) -> Result<(), DbError> {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        tx.execute("SELECT pg_advisory_xact_lock(7419)", &[])
            .await?;
        tx.batch_execute("CREATE TABLE IF NOT EXISTS schema_version (version integer PRIMARY KEY)")
            .await?;
        let applied: i32 = tx
            .query_one("SELECT coalesce(max(version), 0) FROM schema_version", &[])
            .await?
            .get(0);
        for (index, sql) in MIGRATIONS.iter().enumerate() {
            let version = i32::try_from(index + 1).unwrap_or(i32::MAX);
            if version <= applied {
                continue;
            }
            tx.batch_execute(sql).await?;
            tx.execute(
                "INSERT INTO schema_version (version) VALUES ($1)",
                &[&version],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Apply a push batch for `user` in one transaction.
    pub async fn push(&self, user: Uuid, batch: &PushBatch) -> Result<Vec<PushOutcome>, DbError> {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        act_as(&tx, user).await?;
        let mut outcomes = Vec::with_capacity(batch.ops.len());
        for push in batch.ops.iter().take(MAX_OPS) {
            let result = if let Some(result) = receipt(&tx, user, push.op_id).await? {
                result
            } else {
                let result = apply(&tx, user, push).await?;
                store_receipt(&tx, user, push.op_id, result).await?;
                result
            };
            outcomes.push(PushOutcome {
                op_id: push.op_id,
                result,
            });
        }
        tx.commit().await?;
        Ok(outcomes)
    }

    /// Rows of `user` with a revision above `after`, oldest first.
    pub async fn pull(
        &self,
        user: Uuid,
        after: i64,
        limit: u32,
        min_proto: u32,
    ) -> Result<PullPage, DbError> {
        let limit = limit.clamp(1, MAX_PAGE);
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        act_as(&tx, user).await?;
        let rows = tx
            .query(
                "SELECT entity, row_id, op, hlc, node, rev, payload_ver, payload FROM sync_row
                 WHERE user_id = $1 AND rev > $2 ORDER BY rev LIMIT $3",
                &[&user, &after, &(i64::from(limit) + 1)],
            )
            .await?;
        tx.commit().await?;
        let more = rows.len() > limit as usize;
        let mut page = Vec::with_capacity(rows.len().min(limit as usize));
        for row in rows.iter().take(limit as usize) {
            let entity: i16 = row.get(0);
            let op_code: i16 = row.get(2);
            let hlc: i64 = row.get(3);
            let payload_ver: i16 = row.get(6);
            page.push(RemoteRow {
                entity: u16::try_from(entity).map_err(|_| DbError::Corrupt)?,
                row_id: row.get(1),
                op: u16::try_from(op_code).map_err(|_| DbError::Corrupt)?,
                hlc: u64::try_from(hlc).map_err(|_| DbError::Corrupt)?,
                node: row.get(4),
                server_rev: row.get(5),
                payload_ver: u16::try_from(payload_ver).map_err(|_| DbError::Corrupt)?,
                payload: row.get(7),
            });
        }
        Ok(PullPage {
            next: page.last().map_or(after, |row| row.server_rev),
            rows: page,
            more,
            server_time_ms: now_ms(),
            min_proto,
        })
    }

    /// Delete receipts past their lifetime (runs periodically; receipts are
    /// only read for retries of recent pushes).
    pub async fn purge_receipts(&self) -> Result<u64, DbError> {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        // The maintenance policy only allows deleting receipts.
        tx.execute("SELECT set_config('app.maintenance', 'on', true)", &[])
            .await?;
        let purged = tx
            .execute(
                "DELETE FROM push_receipt WHERE at < now() - make_interval(days => $1)",
                &[&RECEIPT_TTL_DAYS],
            )
            .await?;
        tx.commit().await?;
        Ok(purged)
    }
}

async fn receipt(
    tx: &Transaction<'_>,
    user: Uuid,
    op_id: Uuid,
) -> Result<Option<PushResult>, DbError> {
    let Some(row) = tx
        .query_opt(
            "SELECT outcome, rev, code FROM push_receipt WHERE user_id = $1 AND op_id = $2",
            &[&user, &op_id],
        )
        .await?
    else {
        return Ok(None);
    };
    let outcome: i16 = row.get(0);
    let rev: Option<i64> = row.get(1);
    let code: Option<i32> = row.get(2);
    Ok(Some(match outcome {
        APPLIED => PushResult::Applied {
            server_rev: rev.ok_or(DbError::Corrupt)?,
        },
        SUPERSEDED => PushResult::Superseded {
            server_rev: rev.ok_or(DbError::Corrupt)?,
        },
        MERGED => PushResult::Merged {
            server_rev: rev.ok_or(DbError::Corrupt)?,
        },
        _ => PushResult::Rejected {
            code: code
                .and_then(|code| u16::try_from(code).ok())
                .ok_or(DbError::Corrupt)?,
        },
    }))
}

async fn store_receipt(
    tx: &Transaction<'_>,
    user: Uuid,
    op_id: Uuid,
    result: PushResult,
) -> Result<(), DbError> {
    let (outcome, rev, code) = match result {
        PushResult::Applied { server_rev } => (APPLIED, Some(server_rev), None),
        PushResult::Superseded { server_rev } => (SUPERSEDED, Some(server_rev), None),
        PushResult::Merged { server_rev } => (MERGED, Some(server_rev), None),
        PushResult::Rejected { code } => (REJECTED, None, Some(i32::from(code))),
    };
    tx.execute(
        "INSERT INTO push_receipt (user_id, op_id, outcome, rev, code) VALUES ($1, $2, $3, $4, $5)",
        &[&user, &op_id, &outcome, &rev, &code],
    )
    .await?;
    Ok(())
}

/// Validate one op; `Err(code)` refuses it.
fn validate(push: &PushOp) -> Result<(Tlv, i64), u16> {
    if !(op::UPSERT..=op::RESTORE).contains(&push.op) {
        return Err(reject::INVALID);
    }
    if push.payload.len() > PAYLOAD_LIMIT {
        return Err(reject::TOO_LARGE);
    }
    let now = u64::try_from(now_ms()).unwrap_or(0);
    if (push.hlc >> 16) > now.saturating_add(MAX_AHEAD_MS) {
        return Err(reject::FUTURE_CLOCK);
    }
    let hlc = i64::try_from(push.hlc).map_err(|_| reject::INVALID)?;
    let payload = Tlv::decode(&push.payload, PAYLOAD_LIMIT).map_err(|_| reject::INVALID)?;
    Ok((payload, hlc))
}

/// Merge one op into the stored row under the shared rules.
async fn apply(tx: &Transaction<'_>, user: Uuid, push: &PushOp) -> Result<PushResult, DbError> {
    let (incoming, hlc) = match validate(push) {
        Ok(valid) => valid,
        Err(code) => return Ok(PushResult::Rejected { code }),
    };
    let entity = i16::try_from(push.entity).map_err(|_| DbError::Corrupt)?;
    let stored = tx
        .query_opt(
            "SELECT op, hlc, node, rev, payload FROM sync_row
             WHERE user_id = $1 AND entity = $2 AND row_id = $3 FOR UPDATE",
            &[&user, &entity, &push.row_id],
        )
        .await?;
    let (op_code, hlc, node, payload, merged) = match stored {
        None => (push.op, hlc, push.node, push.payload.clone(), false),
        Some(row) => {
            let stored_op: i16 = row.get(0);
            let stored_hlc: i64 = row.get(1);
            let stored_rev: i64 = row.get(3);
            let stored_bytes: Vec<u8> = row.get(4);
            let stored_payload =
                Tlv::decode(&stored_bytes, PAYLOAD_LIMIT).map_err(|_| DbError::Corrupt)?;
            let stored_version = Version {
                op: u16::try_from(stored_op).map_err(|_| DbError::Corrupt)?,
                hlc: u64::try_from(stored_hlc).map_err(|_| DbError::Corrupt)?,
                node: row.get(2),
                payload: &stored_payload,
            };
            let incoming_version = Version {
                op: push.op,
                hlc: push.hlc,
                node: push.node,
                payload: &incoming,
            };
            let outcome =
                merge::resolve(&stored_version, &incoming_version).map_err(|_| DbError::Corrupt)?;
            match outcome {
                Outcome::KeepStored => {
                    return Ok(PushResult::Superseded {
                        server_rev: stored_rev,
                    });
                }
                Outcome::TakeIncoming => (
                    merge::op_after(&stored_version, &incoming_version),
                    hlc,
                    push.node,
                    push.payload.clone(),
                    false,
                ),
                Outcome::Merged {
                    payload,
                    hlc: merged_hlc,
                    node,
                } => (
                    op::UPSERT,
                    i64::try_from(merged_hlc).map_err(|_| DbError::Corrupt)?,
                    node,
                    payload.encode(),
                    true,
                ),
            }
        }
    };
    let rev: i64 = tx
        .query_one(
            "INSERT INTO user_rev (user_id, rev) VALUES ($1, 1)
             ON CONFLICT (user_id) DO UPDATE SET rev = user_rev.rev + 1
             RETURNING rev",
            &[&user],
        )
        .await?
        .get(0);
    let op_code = i16::try_from(op_code).map_err(|_| DbError::Corrupt)?;
    let payload_ver = i16::try_from(push.payload_ver).map_err(|_| DbError::Corrupt)?;
    tx.execute(
        "INSERT INTO sync_row (user_id, entity, row_id, op, hlc, node, rev, payload_ver, payload)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (user_id, entity, row_id) DO UPDATE SET
           op = excluded.op, hlc = excluded.hlc, node = excluded.node, rev = excluded.rev,
           payload_ver = excluded.payload_ver, payload = excluded.payload,
           received_at = now()",
        &[
            &user,
            &entity,
            &push.row_id,
            &op_code,
            &hlc,
            &node,
            &rev,
            &payload_ver,
            &payload,
        ],
    )
    .await?;
    Ok(if merged {
        PushResult::Merged { server_rev: rev }
    } else {
        PushResult::Applied { server_rev: rev }
    })
}
