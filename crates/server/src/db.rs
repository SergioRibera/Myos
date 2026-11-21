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
/// Largest pull page, in rows.
pub const MAX_PAGE: u32 = 1000;
/// Largest pull page, in payload bytes (one row always fits): keeps pages
/// under the message limit clients decode.
const PAGE_BYTES: i64 = 6 * 1024 * 1024;
/// Clocks further ahead of the server than this are refused.
const MAX_AHEAD_MS: u64 = 60 * 1000;
/// Receipts older than this are purged.
const RECEIPT_TTL_DAYS: i32 = 30;

/// Reasons a pushed op is refused.
pub mod reject {
    /// The op's clock (or one of its field clocks) is ahead of the server
    /// clock. Not remembered: the same op may be retried later.
    pub const FUTURE_CLOCK: u16 = 1;
    /// Malformed op or payload.
    pub const INVALID: u16 = 2;
    /// Payload too large.
    pub const TOO_LARGE: u16 = 3;
    /// The op id was already used for different content.
    pub const REPLAYED: u16 = 4;
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
    #[error("database error: {0}")]
    Postgres(#[from] tokio_postgres::Error),
    /// No connection could be taken from the pool.
    #[error("database pool error: {0}")]
    Pool(#[from] deadpool_postgres::PoolError),
    /// The pool could not be built.
    #[error("database pool setup failed")]
    Build(#[from] deadpool_postgres::BuildError),
    /// A stored row is corrupt.
    #[error("stored row is malformed")]
    Corrupt,
    /// The database was migrated by a newer server.
    #[error("database schema {found} is newer than this server ({supported})")]
    NewerSchema {
        /// Version in the database.
        found: i32,
        /// Newest version this build knows.
        supported: usize,
    },
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

/// Bind every statement of `tx` to `user` (row-level security) and bound
/// how long it may run or wait.
async fn act_as(tx: &Transaction<'_>, user: Uuid) -> Result<(), DbError> {
    tx.execute(
        "SELECT set_config('app.user_id', $1, true),
                set_config('statement_timeout', '15s', true),
                set_config('lock_timeout', '5s', true),
                set_config('idle_in_transaction_session_timeout', '30s', true)",
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
        if usize::try_from(applied).is_ok_and(|applied| applied > MIGRATIONS.len()) {
            return Err(DbError::NewerSchema {
                found: applied,
                supported: MIGRATIONS.len(),
            });
        }
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
        // The user's revision row is locked first and held to commit: pushes
        // of one user run one at a time (no lost update on new rows, no
        // lock-order deadlock between rows) and revisions commit in order.
        tx.execute(
            "INSERT INTO user_rev (user_id) VALUES ($1)
             ON CONFLICT (user_id) DO UPDATE SET rev = user_rev.rev",
            &[&user],
        )
        .await?;
        let mut outcomes = Vec::with_capacity(batch.ops.len());
        for push in batch.ops.iter().take(MAX_OPS) {
            let digest = op_digest(push);
            let result = match receipt(&tx, user, push.op_id).await? {
                Some((result, stored)) if stored == digest => result,
                Some(_) => PushResult::Rejected {
                    code: reject::REPLAYED,
                },
                None => {
                    let result = apply(&tx, user, push).await?;
                    if result
                        != (PushResult::Rejected {
                            code: reject::FUTURE_CLOCK,
                        })
                    {
                        store_receipt(&tx, user, push.op_id, digest, result).await?;
                    }
                    result
                }
            };
            outcomes.push(PushOutcome {
                op_id: push.op_id,
                result,
            });
        }
        tx.commit().await?;
        Ok(outcomes)
    }

    /// Rows of `user` with a revision above `after`, oldest first, at most
    /// `limit` rows and about [`PAGE_BYTES`] of payload.
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
        // `octet_length` reads the stored size without detoasting.
        let rows = tx
            .query(
                "SELECT entity, row_id, op, hlc, node, rev, payload_ver, payload FROM (
                   SELECT *, sum(octet_length(payload)) OVER (ORDER BY rev) AS running
                   FROM sync_row WHERE user_id = $1 AND rev > $2 ORDER BY rev LIMIT $3
                 ) page WHERE running - octet_length(payload) < $4 ORDER BY rev",
                &[&user, &after, &i64::from(limit), &PAGE_BYTES],
            )
            .await?;
        let mut page = Vec::with_capacity(rows.len());
        for row in &rows {
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
        let next = page.last().map_or(after, |row| row.server_rev);
        let more: bool = tx
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM sync_row WHERE user_id = $1 AND rev > $2)",
                &[&user, &next],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok(PullPage {
            next,
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

/// Digest of what an op carries, so a reused op id with other content is
/// told apart from a retry.
fn op_digest(push: &PushOp) -> i64 {
    let mut bytes = Vec::with_capacity(push.payload.len() + 48);
    bytes.extend_from_slice(&push.entity.to_le_bytes());
    bytes.extend_from_slice(push.row_id.as_bytes());
    bytes.extend_from_slice(&push.op.to_le_bytes());
    bytes.extend_from_slice(&push.hlc.to_le_bytes());
    bytes.extend_from_slice(&push.node.to_le_bytes());
    bytes.extend_from_slice(&push.payload_ver.to_le_bytes());
    bytes.extend_from_slice(&push.payload);
    i64::from_le_bytes(merge::value_hash(Some(&bytes)).to_le_bytes())
}

async fn receipt(
    tx: &Transaction<'_>,
    user: Uuid,
    op_id: Uuid,
) -> Result<Option<(PushResult, i64)>, DbError> {
    let Some(row) = tx
        .query_opt(
            "SELECT outcome, rev, code, digest FROM push_receipt WHERE user_id = $1 AND op_id = $2",
            &[&user, &op_id],
        )
        .await?
    else {
        return Ok(None);
    };
    let outcome: i16 = row.get(0);
    let rev: Option<i64> = row.get(1);
    let code: Option<i32> = row.get(2);
    let rev = || rev.ok_or(DbError::Corrupt);
    let result = match outcome {
        APPLIED => PushResult::Applied { server_rev: rev()? },
        SUPERSEDED => PushResult::Superseded { server_rev: rev()? },
        MERGED => PushResult::Merged { server_rev: rev()? },
        REJECTED => PushResult::Rejected {
            code: code
                .and_then(|code| u16::try_from(code).ok())
                .ok_or(DbError::Corrupt)?,
        },
        _ => return Err(DbError::Corrupt),
    };
    Ok(Some((result, row.get(3))))
}

async fn store_receipt(
    tx: &Transaction<'_>,
    user: Uuid,
    op_id: Uuid,
    digest: i64,
    result: PushResult,
) -> Result<(), DbError> {
    let (outcome, rev, code) = match result {
        PushResult::Applied { server_rev } => (APPLIED, Some(server_rev), None),
        PushResult::Superseded { server_rev } => (SUPERSEDED, Some(server_rev), None),
        PushResult::Merged { server_rev } => (MERGED, Some(server_rev), None),
        PushResult::Rejected { code } => (REJECTED, None, Some(i32::from(code))),
    };
    tx.execute(
        "INSERT INTO push_receipt (user_id, op_id, outcome, rev, code, digest)
         VALUES ($1, $2, $3, $4, $5, $6)",
        &[&user, &op_id, &outcome, &rev, &code, &digest],
    )
    .await?;
    Ok(())
}

/// An op that passed validation, in storage types.
struct Valid {
    payload: Tlv,
    entity: i16,
    payload_ver: i16,
    hlc: i64,
}

/// Validate one op; `Err(code)` refuses it. Everything stored here must be
/// readable by every later merge, or one bad op would block the row.
fn validate(push: &PushOp) -> Result<Valid, u16> {
    if !(op::UPSERT..=op::RESTORE).contains(&push.op) {
        return Err(reject::INVALID);
    }
    let entity = i16::try_from(push.entity)
        .ok()
        .filter(|entity| *entity > 0)
        .ok_or(reject::INVALID)?;
    let payload_ver = i16::try_from(push.payload_ver)
        .ok()
        .filter(|version| *version > 0)
        .ok_or(reject::INVALID)?;
    if push.payload.len() > PAYLOAD_LIMIT {
        return Err(reject::TOO_LARGE);
    }
    let now = u64::try_from(now_ms()).unwrap_or(0);
    let ahead = |hlc: u64| (hlc >> 16) > now.saturating_add(MAX_AHEAD_MS);
    if ahead(push.hlc) {
        return Err(reject::FUTURE_CLOCK);
    }
    let hlc = i64::try_from(push.hlc).map_err(|_| reject::INVALID)?;
    let payload = Tlv::decode(&push.payload, PAYLOAD_LIMIT).map_err(|_| reject::INVALID)?;
    let clocks = merge::read_clocks(&payload).map_err(|_| reject::INVALID)?;
    if clocks.values().any(|clock| ahead(clock.hlc)) {
        return Err(reject::FUTURE_CLOCK);
    }
    if clocks
        .values()
        .any(|clock| i64::try_from(clock.hlc).is_err())
    {
        return Err(reject::INVALID);
    }
    Ok(Valid {
        payload,
        entity,
        payload_ver,
        hlc,
    })
}

/// Merge one op into the stored row under the shared rules.
async fn apply(tx: &Transaction<'_>, user: Uuid, push: &PushOp) -> Result<PushResult, DbError> {
    let valid = match validate(push) {
        Ok(valid) => valid,
        Err(code) => return Ok(PushResult::Rejected { code }),
    };
    let stored = tx
        .query_opt(
            "SELECT op, hlc, node, rev, payload FROM sync_row
             WHERE user_id = $1 AND entity = $2 AND row_id = $3",
            &[&user, &valid.entity, &push.row_id],
        )
        .await?;
    let (op_code, hlc, node, payload, merged) = match stored {
        None => (push.op, valid.hlc, push.node, push.payload.clone(), false),
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
                payload: &valid.payload,
            };
            let Ok(outcome) = merge::resolve(&stored_version, &incoming_version) else {
                return Ok(PushResult::Rejected {
                    code: reject::INVALID,
                });
            };
            match outcome {
                Outcome::KeepStored => {
                    return Ok(PushResult::Superseded {
                        server_rev: stored_rev,
                    });
                }
                Outcome::TakeIncoming => (
                    merge::op_after(&stored_version, &incoming_version),
                    valid.hlc,
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
            "UPDATE user_rev SET rev = rev + 1 WHERE user_id = $1 RETURNING rev",
            &[&user],
        )
        .await?
        .get(0);
    let op_code = i16::try_from(op_code).map_err(|_| DbError::Corrupt)?;
    tx.execute(
        "INSERT INTO sync_row (user_id, entity, row_id, op, hlc, node, rev, payload_ver, payload)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (user_id, entity, row_id) DO UPDATE SET
           op = excluded.op, hlc = excluded.hlc, node = excluded.node, rev = excluded.rev,
           payload_ver = excluded.payload_ver, payload = excluded.payload,
           received_at = now()",
        &[
            &user,
            &valid.entity,
            &push.row_id,
            &op_code,
            &hlc,
            &node,
            &rev,
            &valid.payload_ver,
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
