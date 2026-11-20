//! Sync protocol, client side: push the outbox, pull the change feed.
//!
//! Rules (docs/db/schema.md §8, §10):
//! - Nothing is pushed before an account is attached: natural-key ids minted
//!   under the device key are rewritten with the account key first.
//! - The server is the authority. A pulled row whose local copy has no
//!   unpushed change is applied as is. With an unpushed change, the server's
//!   rules decide: deletes win unless restored, otherwise the greater
//!   `(hlc, node)` wins; when the remote wins, the local entry is dropped.
//! - Each pulled row is applied in its own savepoint: a row this build cannot
//!   store goes to `sync_quarantine` and the cursor still advances.

use std::time::Duration;

use myos_core::code::Code;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use crate::payload::{self, Columns, RowStamp};
use crate::spec::{self, TableSpec};
use crate::sync::{Entity, Op, sql_hlc};
use crate::{Error, Hlc, Store, ids};

/// Protocol version this build speaks.
pub const PROTO: u32 = 1;
/// Version of the payload layout ([`crate`] spec tags).
pub const PAYLOAD_VERSION: u16 = 1;
/// Own data stream.
pub const STREAM_OWN: u16 = 1;

/// Largest push batch.
const PUSH_BATCH: u32 = 500;
/// Byte budget of one push batch.
const PUSH_BYTES: usize = 4 * 1024 * 1024;
/// Largest pull page requested.
const PULL_PAGE: u32 = 500;
/// Bound on push / pull rounds in one [`Store::sync_once`].
const MAX_ROUNDS: usize = 64;
/// Largest accepted clock offset from the server.
const MAX_OFFSET_MS: i64 = 24 * 3600 * 1000;
/// Remote clocks further ahead than this are not adopted locally.
const MAX_AHEAD_MS: u64 = 5 * 60 * 1000;
/// Longest retry delay.
const MAX_BACKOFF: Duration = Duration::from_secs(3600);

/// `sync_outbox.state` of entries kept for the UI.
const FAILED: i64 = 2;

/// `sync_outbox.error_code` for rows a newer build wrote (quarantined).
const ERROR_NEEDS_UPDATE: i64 = 1;

/// Quarantine reasons.
const UNKNOWN_ENTITY: i64 = 1;
const UNDECODABLE: i64 = 2;
const CONSTRAINT: i64 = 3;

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

/// The network side of sync.
pub trait Transport {
    /// Transport failure.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Send a batch.
    fn push(&mut self, batch: &PushBatch) -> Result<PushResponse, Self::Error>;

    /// Read the feed of `stream` after revision `after`.
    fn pull(&mut self, stream: u16, after: i64, limit: u32) -> Result<PullPage, Self::Error>;
}

/// What one sync round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Ops acknowledged as applied.
    pub pushed: u32,
    /// Ops the server superseded.
    pub superseded: u32,
    /// Ops the server rejected.
    pub rejected: u32,
    /// Remote rows written locally.
    pub applied: u32,
    /// Remote rows set aside for a newer build.
    pub quarantined: u32,
    /// The server requires a newer app: local writes are refused.
    pub read_only: bool,
}

/// Local copy of a row, for the merge decision.
struct Local {
    hlc: i64,
    node: i64,
    deleted: bool,
    day: Option<i64>,
    server_rev: Option<i64>,
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// Retry delay after `attempts` failures: exponential, capped, with jitter.
fn backoff(attempts: i64) -> Result<i64, Error> {
    let exponent = u32::try_from(attempts.clamp(0, 12)).unwrap_or(12);
    let base = Duration::from_secs(1 << exponent).min(MAX_BACKOFF);
    let jitter = u64::from(getrandom::u32().map_err(Error::Random)? % 1000);
    i64::try_from(base.as_millis() + u128::from(jitter)).map_err(|_| Error::OutOfRange("backoff"))
}

/// Return in-flight entries to the queue: dropped when a newer pending entry
/// for the same row exists (carrying an explicit restore over), otherwise
/// pending again with a later retry time.
fn requeue_in_flight(tx: &Transaction<'_>, delay: Option<i64>) -> Result<(), Error> {
    tx.execute(
        "UPDATE sync_outbox SET op = 3
         WHERE state = 0 AND op = 1 AND EXISTS (
           SELECT 1 FROM sync_outbox f
           WHERE f.state = 1 AND f.op = 3 AND f.entity = sync_outbox.entity
             AND f.row_id = sync_outbox.row_id)",
        [],
    )?;
    tx.execute(
        "DELETE FROM sync_outbox WHERE state = 1 AND EXISTS (
           SELECT 1 FROM sync_outbox p
           WHERE p.state = 0 AND p.entity = sync_outbox.entity AND p.row_id = sync_outbox.row_id)",
        [],
    )?;
    match delay {
        Some(delay) => {
            let mut statement =
                tx.prepare_cached("SELECT seq, attempts FROM sync_outbox WHERE state = 1")?;
            let entries: Vec<(i64, i64)> = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;
            for (seq, attempts) in entries {
                tx.execute(
                    "UPDATE sync_outbox SET state = 0, attempts = ?2, next_attempt_at = ?3
                     WHERE seq = ?1",
                    params![
                        seq,
                        attempts + 1,
                        now_ms() + delay.max(backoff(attempts + 1)?)
                    ],
                )?;
            }
        }
        None => {
            tx.execute("UPDATE sync_outbox SET state = 0 WHERE state = 1", [])?;
        }
    }
    Ok(())
}

impl Store {
    /// Return entries a crash left in flight to the queue (run at open).
    pub(crate) fn recover_in_flight(conn: &mut Connection) -> Result<(), Error> {
        let tx = conn.transaction()?;
        requeue_in_flight(&tx, None)?;
        tx.commit()?;
        Ok(())
    }

    /// Attach the account: natural-key ids minted under the device key are
    /// rewritten with `account_key` (so every device of the account mints
    /// the same ids), then sync is allowed.
    pub fn attach_account(&mut self, user_id: Uuid, account_key: &[u8; 32]) -> Result<(), Error> {
        let current: Option<[u8; 16]> =
            self.conn()
                .query_row("SELECT user_id FROM sync_state WHERE k = 1", [], |row| {
                    row.get(0)
                })?;
        match current {
            Some(current) if current == *user_id.as_bytes() => return Ok(()),
            // Another account's data is on this device: it has to be wiped
            // (logout) first, never mixed.
            Some(_) => return Err(Error::AccountMismatch),
            None => {}
        }
        let tx = self.conn_mut().transaction()?;
        let rows: Vec<([u8; 16], i64)> = tx
            .prepare("SELECT id, from_day FROM profile_goal")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (old, from_day) in rows {
            let new = ids::natural(account_key, Entity::ProfileGoal, &[&from_day.to_be_bytes()]);
            if new.as_bytes() == &old {
                continue;
            }
            // Another row may already hold the new id: keep the newer.
            tx.execute(
                "DELETE FROM profile_goal WHERE id = ?1 AND (hlc, node) < (
                   SELECT hlc, node FROM profile_goal WHERE id = ?2)",
                params![new.as_bytes().as_slice(), old.as_slice()],
            )?;
            let taken: bool = tx.query_row(
                "SELECT count(*) > 0 FROM profile_goal WHERE id = ?1",
                [new.as_bytes().as_slice()],
                |row| row.get(0),
            )?;
            if taken {
                tx.execute("DELETE FROM profile_goal WHERE id = ?1", [old.as_slice()])?;
            } else {
                tx.execute(
                    "UPDATE profile_goal SET id = ?2 WHERE id = ?1",
                    params![old.as_slice(), new.as_bytes().as_slice()],
                )?;
            }
            tx.execute(
                "UPDATE sync_outbox SET row_id = ?2 WHERE entity = ?3 AND row_id = ?1",
                params![
                    old.as_slice(),
                    new.as_bytes().as_slice(),
                    Entity::ProfileGoal.code()
                ],
            )?;
        }
        // External activities and range overrides have no writer yet; their
        // rewrite lands with their APIs.
        tx.execute(
            "UPDATE sync_state SET user_id = ?1, user_key = ?2, id_epoch = id_epoch + 1
             WHERE k = 1",
            params![user_id.as_bytes().as_slice(), account_key.as_slice()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Push the outbox, then pull and apply the change feed.
    pub fn sync_once<T: Transport>(&mut self, transport: &mut T) -> Result<SyncReport, Error> {
        let (account, read_only): (Option<Vec<u8>>, bool) = self.conn().query_row(
            "SELECT user_id, read_only FROM sync_state WHERE k = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if account.is_none() {
            return Err(Error::NoAccount);
        }
        let mut report = SyncReport {
            read_only,
            ..SyncReport::default()
        };
        // A failing push (rejected batch, server error) must not keep the
        // device from receiving changes: pull anyway, report the push error.
        let mut push_error = None;
        if !read_only {
            for _ in 0..MAX_ROUNDS {
                match self.push_round(transport, &mut report) {
                    Ok(true) if !report.read_only => {}
                    Ok(_) => break,
                    Err(error) => {
                        push_error = Some(error);
                        break;
                    }
                }
            }
        }
        for _ in 0..MAX_ROUNDS {
            if !self.pull_round(transport, &mut report)? || report.read_only {
                break;
            }
        }
        push_error.map_or(Ok(report), Err)
    }

    /// One push batch; `false` when nothing was pending.
    fn push_round<T: Transport>(
        &mut self,
        transport: &mut T,
        report: &mut SyncReport,
    ) -> Result<bool, Error> {
        let Some(batch) = self.take_batch()? else {
            return Ok(false);
        };
        if batch.ops.is_empty() {
            return Ok(true);
        }
        let sent = now_ms();
        let response = match transport.push(&batch) {
            Ok(response) => response,
            Err(err) => {
                let tx = self.conn_mut().transaction()?;
                requeue_in_flight(&tx, Some(0))?;
                tx.commit()?;
                return Err(Error::Transport(Box::new(err)));
            }
        };
        self.acknowledge(&batch, &response, sent, report)?;
        Ok(true)
    }

    /// Mark the next ready entries in flight and build their ops; `None`
    /// when nothing is ready.
    fn take_batch(&mut self) -> Result<Option<PushBatch>, Error> {
        type Entry = (i64, [u8; 16], i64, [u8; 16], i64, i64);
        let (device, node): (Vec<u8>, i64) = self.conn().query_row(
            "SELECT device_id, node FROM sync_state WHERE k = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let tx = self.conn_mut().transaction()?;
        // Anything still in flight here was left by a failed acknowledgement.
        requeue_in_flight(&tx, None)?;
        let entries: Vec<Entry> = tx
            .prepare_cached(
                "SELECT seq, op_id, entity, row_id, op, hlc FROM sync_outbox
                 WHERE state = 0 AND (next_attempt_at IS NULL OR next_attempt_at <= ?1)
                 ORDER BY seq LIMIT ?2",
            )?
            .query_map(params![now_ms(), PUSH_BATCH], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?
            .collect::<Result<_, _>>()?;
        if entries.is_empty() {
            return Ok(None);
        }
        let mut ops = Vec::with_capacity(entries.len());
        let mut bytes = 0_usize;
        for (seq, op_id, entity, row_id, op, hlc) in entries {
            let row_id = Uuid::from_bytes(row_id);
            let spec = u16::try_from(entity)
                .ok()
                .and_then(Entity::from_code)
                .and_then(spec::of);
            let payload = match spec {
                Some(spec) => payload::build(&tx, spec, row_id)?,
                None => None,
            };
            let Some(payload) = payload else {
                // The row is gone or the entity unknown: nothing to send.
                tx.execute("DELETE FROM sync_outbox WHERE seq = ?1", [seq])?;
                continue;
            };
            // Byte budget: the rest waits for the next round.
            if !ops.is_empty() && bytes + payload.len() > PUSH_BYTES {
                break;
            }
            bytes += payload.len();
            tx.execute("UPDATE sync_outbox SET state = 1 WHERE seq = ?1", [seq])?;
            ops.push(PushOp {
                op_id: Uuid::from_bytes(op_id),
                entity: u16::try_from(entity).map_err(|_| Error::OutOfRange("entity"))?,
                row_id,
                op: u16::try_from(op).map_err(|_| Error::OutOfRange("op"))?,
                hlc: u64::try_from(hlc).map_err(|_| Error::OutOfRange("hlc"))?,
                node,
                payload_ver: PAYLOAD_VERSION,
                payload,
            });
        }
        tx.commit()?;
        Ok(Some(PushBatch {
            device_id: Uuid::from_slice(&device).map_err(|_| Error::OutOfRange("device"))?,
            proto: PROTO,
            ops,
        }))
    }

    /// Apply the server's answers to a pushed batch.
    fn acknowledge(
        &mut self,
        batch: &PushBatch,
        response: &PushResponse,
        sent: i64,
        report: &mut SyncReport,
    ) -> Result<(), Error> {
        let tx = self.conn_mut().transaction()?;
        for outcome in &response.outcomes {
            let Some(op) = batch.ops.iter().find(|op| op.op_id == outcome.op_id) else {
                continue;
            };
            let op_id = op.op_id.as_bytes().as_slice();
            match outcome.result {
                PushResult::Applied { server_rev } => {
                    tx.execute("DELETE FROM sync_outbox WHERE op_id = ?1", [op_id])?;
                    if let Some(spec) = Entity::from_code(op.entity).and_then(spec::of) {
                        // Only if no newer local write happened meanwhile.
                        tx.execute(
                            &format!(
                                "UPDATE {} SET server_rev = ?2 WHERE id = ?1 AND hlc = ?3",
                                spec.table
                            ),
                            params![
                                op.row_id.as_bytes().as_slice(),
                                server_rev,
                                sql_hlc(Hlc::from_raw(op.hlc))?
                            ],
                        )?;
                    }
                    report.pushed += 1;
                }
                PushResult::Superseded { .. } => {
                    tx.execute("DELETE FROM sync_outbox WHERE op_id = ?1", [op_id])?;
                    report.superseded += 1;
                }
                PushResult::Rejected { code } => {
                    tx.execute(
                        "UPDATE sync_outbox SET state = 2, error_code = ?2 WHERE op_id = ?1",
                        params![op_id, code],
                    )?;
                    report.rejected += 1;
                }
            }
        }
        // Ops the server did not answer go back to the queue, with backoff.
        requeue_in_flight(&tx, Some(0))?;
        update_clock(
            &tx,
            response.server_time_ms,
            sent,
            response.min_proto,
            report,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// One pull page; `false` when the feed is drained.
    fn pull_round<T: Transport>(
        &mut self,
        transport: &mut T,
        report: &mut SyncReport,
    ) -> Result<bool, Error> {
        let cursor: i64 = self
            .conn()
            .query_row(
                "SELECT cursor FROM sync_cursor WHERE stream = ?1",
                [STREAM_OWN],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let sent = now_ms();
        let page = transport
            .pull(STREAM_OWN, cursor, PULL_PAGE)
            .map_err(|err| Error::Transport(Box::new(err)))?;
        let mut tx = self.conn_mut().transaction()?;
        update_clock(&tx, page.server_time_ms, sent, page.min_proto, report)?;
        if report.read_only {
            // An old build stops applying rows it may misread.
            tx.commit()?;
            return Ok(false);
        }
        tx.pragma_update(None, "defer_foreign_keys", true)?;
        let mut local_seq: i64 =
            tx.query_row("SELECT local_seq FROM sync_state WHERE k = 1", [], |row| {
                row.get(0)
            })?;
        let mut newest = 0_u64;
        for row in &page.rows {
            newest = newest.max(row.hlc);
            let savepoint = tx.savepoint()?;
            match apply_remote(&savepoint, row, &mut local_seq) {
                Ok(true) => {
                    savepoint.commit()?;
                    report.applied += 1;
                }
                Ok(false) => savepoint.commit()?,
                Err(ApplyError::Quarantine(reason)) => {
                    drop(savepoint);
                    quarantine(&tx, row, reason)?;
                    report.quarantined += 1;
                }
                // The page is rolled back and the cursor stays: retried later.
                Err(ApplyError::Fatal(error)) => return Err(error),
            }
        }
        receive_clock(&tx, newest)?;
        tx.execute(
            "UPDATE sync_state SET local_seq = ?1 WHERE k = 1",
            [local_seq],
        )?;
        tx.execute(
            "INSERT INTO sync_cursor (stream, cursor, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (stream) DO UPDATE SET cursor = excluded.cursor,
               updated_at = excluded.updated_at",
            params![STREAM_OWN, page.next.max(cursor), now_ms()],
        )?;
        tx.commit()?;
        Ok(page.more)
    }
}

/// Server clock offset (bounded) and protocol gate.
fn update_clock(
    tx: &Transaction<'_>,
    server_time_ms: i64,
    sent_ms: i64,
    min_proto: u32,
    report: &mut SyncReport,
) -> Result<(), Error> {
    let midpoint = sent_ms + (now_ms() - sent_ms) / 2;
    let offset = server_time_ms
        .saturating_sub(midpoint)
        .clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS);
    report.read_only = min_proto > PROTO;
    tx.execute(
        "UPDATE sync_state SET server_offset_ms = ?1, read_only = ?2 WHERE k = 1",
        params![offset, report.read_only],
    )?;
    Ok(())
}

/// HLC receive rule, bounded: never adopt a clock far ahead of ours.
fn receive_clock(tx: &Transaction<'_>, remote: u64) -> Result<(), Error> {
    let (last, offset): (i64, i64) = tx.query_row(
        "SELECT hlc_last, server_offset_ms FROM sync_state WHERE k = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let wall = u64::try_from(now_ms().saturating_add(offset)).unwrap_or(0);
    let remote = Hlc::from_raw(remote);
    if remote.millis() > wall.saturating_add(MAX_AHEAD_MS) {
        return Ok(());
    }
    let merged = Hlc::from_raw(u64::try_from(last).unwrap_or(0)).receive(remote, wall);
    tx.execute(
        "UPDATE sync_state SET hlc_last = ?1 WHERE k = 1",
        [sql_hlc(merged)?],
    )?;
    Ok(())
}

/// Why a remote row was not written.
enum ApplyError {
    /// This build cannot store it: set it aside, keep going.
    Quarantine(i64),
    /// The database failed (busy, I/O, …): abort the page, keep the cursor.
    Fatal(Error),
}

impl From<rusqlite::Error> for ApplyError {
    fn from(error: rusqlite::Error) -> Self {
        // Only a row the schema refuses is quarantined; anything else is the
        // database failing, and skipping the row would lose it.
        if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
            Self::Quarantine(CONSTRAINT)
        } else {
            Self::Fatal(Error::Sqlite(error))
        }
    }
}

impl From<Error> for ApplyError {
    fn from(error: Error) -> Self {
        match error {
            Error::Sqlite(error) => error.into(),
            Error::Payload(_) => Self::Quarantine(UNDECODABLE),
            other => Self::Fatal(other),
        }
    }
}

/// Write a remote row when the rules say so. `Ok(false)`: kept the local
/// version.
fn apply_remote(
    conn: &Connection,
    row: &RemoteRow,
    local_seq: &mut i64,
) -> Result<bool, ApplyError> {
    let spec = Entity::from_code(row.entity)
        .and_then(spec::of)
        .ok_or(ApplyError::Quarantine(UNKNOWN_ENTITY))?;
    let columns = Columns::of(conn, spec.table)?;
    let member_columns = match spec.members {
        Some(members) => Some(Columns::of(conn, members.table)?),
        None => None,
    };
    let decoded = payload::decode(&row.payload, spec, &columns, member_columns.as_ref())?;
    let hlc = i64::try_from(row.hlc).map_err(|_| ApplyError::Quarantine(UNDECODABLE))?;
    let remote_op = Op::from_code(row.op).ok_or(ApplyError::Quarantine(UNDECODABLE))?;
    let remote_deleted = !matches!(decoded.deleted_at, Value::Null);

    let local = read_local(conn, spec, row.row_id)?;
    // Failed entries (rejected, needs update) do not compete: the server
    // refused them or this build cannot see the newer row.
    let pending: Option<i64> = conn
        .query_row(
            "SELECT op FROM sync_outbox WHERE entity = ?1 AND row_id = ?2 AND state < 2
             ORDER BY state = 0 DESC, seq DESC LIMIT 1",
            params![row.entity, row.row_id.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(local) = &local {
        if (local.hlc, local.node) == (hlc, row.node) {
            // Our own write coming back: just record the revision.
            conn.execute(
                &format!("UPDATE {} SET server_rev = ?2 WHERE id = ?1", spec.table),
                params![row.row_id.as_bytes().as_slice(), row.server_rev],
            )?;
            return Ok(false);
        }
        if local
            .server_rev
            .is_some_and(|known| row.server_rev <= known)
        {
            // An older revision than the one already applied.
            return Ok(false);
        }
    }

    if let (Some(local), Some(pending)) = (&local, pending) {
        let newer = (hlc, row.node) > (local.hlc, local.node);
        let local_restore = pending == i64::from(Op::Restore.code());
        let wins = match (local.deleted, remote_deleted) {
            (false, true) => !local_restore || newer,
            (true, false) => remote_op == Op::Restore && newer,
            _ => newer,
        };
        if !wins {
            return Ok(false);
        }
    }

    *local_seq += 1;
    let has_field_hlc = columns_has(conn, spec.table, "field_hlc")?;
    payload::write(
        conn,
        spec,
        row.row_id,
        &decoded,
        RowStamp {
            hlc,
            node: row.node,
            server_rev: row.server_rev,
            local_seq: *local_seq,
        },
        has_field_hlc,
    )?;
    // The remote version wins: unpushed local changes of the row are moot,
    // and older quarantined versions of it must never be replayed.
    conn.execute(
        "DELETE FROM sync_outbox WHERE entity = ?1 AND row_id = ?2 AND state < 2",
        params![row.entity, row.row_id.as_bytes().as_slice()],
    )?;
    conn.execute(
        "DELETE FROM sync_quarantine WHERE entity = ?1 AND row_id = ?2 AND server_rev < ?3",
        params![row.entity, row.row_id.as_bytes().as_slice(), row.server_rev],
    )?;
    // Derived data of the day the row left.
    if let Some(old_day) = local.and_then(|local| local.day)
        && decoded.value(spec, "day") != Some(&Value::Integer(old_day))
    {
        conn.execute(
            "INSERT OR IGNORE INTO derived_touched_day (day) VALUES (?1)",
            [old_day],
        )?;
    }
    if let Some(Value::Blob(exercise)) = match spec.entity {
        Entity::SetEntry => decoded.value(spec, "exercise_id").cloned(),
        Entity::CustomExercise => Some(Value::Blob(row.row_id.as_bytes().to_vec())),
        _ => None,
    } {
        let source = match Uuid::from_slice(&exercise).map(|id| id.get_version_num()) {
            Ok(5) => 1,
            Ok(7) => 2,
            _ => 0,
        };
        conn.execute(
            "INSERT INTO exercise_ref (id, src) VALUES (?1, ?2) ON CONFLICT (id) DO NOTHING",
            params![exercise, source],
        )?;
    }
    Ok(true)
}

fn columns_has(conn: &Connection, table: &str, column: &str) -> Result<bool, Error> {
    Ok(conn.query_row(
        "SELECT count(*) > 0 FROM pragma_table_info(?1) WHERE name = ?2",
        params![table, column],
        |row| row.get(0),
    )?)
}

fn read_local(conn: &Connection, spec: &TableSpec, id: Uuid) -> Result<Option<Local>, Error> {
    let has_day = spec.columns.contains(&"day");
    let sql = format!(
        "SELECT hlc, node, deleted_at IS NOT NULL, {}, server_rev FROM {} WHERE id = ?1",
        if has_day { "day" } else { "NULL" },
        spec.table
    );
    Ok(conn
        .query_row(&sql, [id.as_bytes().as_slice()], |row| {
            Ok(Local {
                hlc: row.get(0)?,
                node: row.get(1)?,
                deleted: row.get(2)?,
                day: row.get(3)?,
                server_rev: row.get(4)?,
            })
        })
        .optional()?)
}

/// Set a row aside for a newer build; its unpushed local changes would
/// overwrite data this build cannot see, so they fail as "needs update".
fn quarantine(tx: &Transaction<'_>, row: &RemoteRow, reason: i64) -> Result<(), Error> {
    tx.execute(
        "DELETE FROM sync_quarantine WHERE entity = ?1 AND row_id = ?2 AND server_rev < ?3",
        params![row.entity, row.row_id.as_bytes().as_slice(), row.server_rev],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO sync_quarantine
           (stream, server_rev, entity, row_id, op, hlc, node, payload_ver, payload, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            STREAM_OWN,
            row.server_rev,
            row.entity,
            row.row_id.as_bytes().as_slice(),
            row.op,
            i64::try_from(row.hlc).unwrap_or(i64::MAX),
            row.node,
            row.payload_ver,
            row.payload,
            reason,
        ],
    )?;
    tx.execute(
        "UPDATE sync_outbox SET state = ?3, error_code = ?4 WHERE entity = ?1 AND row_id = ?2",
        params![
            row.entity,
            row.row_id.as_bytes().as_slice(),
            FAILED,
            ERROR_NEEDS_UPDATE
        ],
    )?;
    Ok(())
}
