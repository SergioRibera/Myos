//! Devices syncing through the real server and a throwaway `PostgreSQL`
//! cluster. Needs `initdb` / `pg_ctl` on PATH (the nix devshell has them);
//! skipped otherwise. Run with `--features dev-auth`.
#![cfg(feature = "dev-auth")]
#![expect(clippy::unwrap_used, reason = "tests: a panic is the failure report")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use jiff::Timestamp;
use jiff::civil::date;
use myos_core::id::Id;
use myos_core::profile::{Preferences, Theme};
use myos_core::session::{SessionOrigin, SetRecord};
use myos_core::units::Mass;
use myos_core::volume::DaySpan;
use myos_server::auth::DevTokens;
use myos_server::db::Db;
use myos_server::db::reject;
use myos_server::http::{AppState, router};
use myos_store::{DatabaseKey, SessionDraft, Store};
use myos_sync_http::HttpTransport;
use myos_sync_wire::merge;
use myos_sync_wire::protocol::{PushBatch, PushOp, PushResult};
use myos_sync_wire::tlv::Tlv;
use uuid::Uuid;

/// A `PostgreSQL` cluster on a unix socket in a temporary directory.
struct Cluster {
    dir: PathBuf,
}

impl Cluster {
    fn start() -> Option<Self> {
        let available = Command::new("initdb").arg("--version").output().is_ok();
        if !available {
            eprintln!("skipping: initdb not on PATH");
            return None;
        }
        let dir = std::env::temp_dir().join(format!("myos-pg-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let data = dir.join("data");
        let ok = Command::new("initdb")
            .args([
                "-D",
                data.to_str().unwrap(),
                "-U",
                "postgres",
                "--auth=trust",
                "-E",
                "UTF8",
            ])
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "initdb failed");
        let options = format!("-k {} -c listen_addresses=''", dir.display());
        let ok = Command::new("pg_ctl")
            .args([
                "-D",
                data.to_str().unwrap(),
                "-o",
                &options,
                "-l",
                dir.join("log").to_str().unwrap(),
                "-w",
                "start",
            ])
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "pg_ctl start failed");
        let cluster = Self { dir };
        // The server runs as an ordinary role: superusers bypass row-level
        // security, which these tests also check.
        cluster.psql("CREATE ROLE myos_app LOGIN");
        cluster.psql("CREATE DATABASE myos OWNER myos_app");
        Some(cluster)
    }

    fn psql(&self, sql: &str) {
        let ok = Command::new("psql")
            .args([
                "-h",
                self.dir.to_str().unwrap(),
                "-U",
                "postgres",
                "-d",
                "postgres",
                "-c",
                sql,
            ])
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "psql failed: {sql}");
    }

    fn config(&self) -> tokio_postgres::Config {
        let mut config = tokio_postgres::Config::new();
        config.host_path(&self.dir).user("myos_app").dbname("myos");
        config
    }

    fn socket_dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Cluster {
    fn drop(&mut self) {
        let _stopped = Command::new("pg_ctl")
            .args([
                "-D",
                self.dir.join("data").to_str().unwrap(),
                "-m",
                "immediate",
                "stop",
            ])
            .output();
        let _removed = std::fs::remove_dir_all(&self.dir);
    }
}

/// Server on a random local port; returns its base URL.
fn serve(runtime: &tokio::runtime::Runtime, db: Db, tokens: DevTokens) -> String {
    let state = AppState {
        db,
        auth: Arc::new(tokens),
        min_proto: 1,
    };
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();
    runtime.spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    format!("http://{address}")
}

const ACCOUNT_KEY: [u8; 32] = [9; 32];

fn device(user: Uuid) -> Store {
    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    store.attach_account(user, &ACCOUNT_KEY).unwrap();
    store
}

fn week() -> DaySpan {
    DaySpan::iso_week(date(2026, 11, 18))
}

#[test]
fn devices_sync_through_postgres() {
    let Some(cluster) = Cluster::start() else {
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let db = Db::new(cluster.config(), 4).unwrap();
    runtime.block_on(db.migrate()).unwrap();
    // Migrating twice is a no-op.
    runtime.block_on(db.migrate()).unwrap();

    let alice = Uuid::now_v7();
    let bob = Uuid::now_v7();
    let base = serve(
        &runtime,
        db.clone(),
        DevTokens::default().with("alice", alice).with("bob", bob),
    );

    let mut phone = device(alice);
    let mut tablet = device(alice);
    let mut phone_link = HttpTransport::new(&base, "alice").unwrap();
    let mut tablet_link = HttpTransport::new(&base, "alice").unwrap();

    let session = phone
        .start_session(SessionDraft {
            started_at: Timestamp::from_second(1_795_000_000).unwrap(),
            day: date(2026, 11, 18),
            tz_offset_min: 0,
            origin: SessionOrigin::Manual,
        })
        .unwrap();
    for kg in [80.0, 82.5, 85.0] {
        phone
            .log_set(
                session,
                &SetRecord::new(Id::generate()).with_lift(Mass::from_kg(kg).unwrap(), 8),
            )
            .unwrap();
    }
    phone
        .set_preferences(&Preferences {
            theme: Theme::Dark,
            ..Preferences::default()
        })
        .unwrap();

    let report = phone.sync_once(&mut phone_link).unwrap();
    assert_eq!(report.pushed, 5);
    let report = tablet.sync_once(&mut tablet_link).unwrap();
    assert_eq!(report.applied, 5);
    assert_eq!(
        tablet.sessions_in(week()).unwrap(),
        phone.sessions_in(week()).unwrap()
    );
    assert_eq!(tablet.preferences().unwrap().theme, Theme::Dark);

    // A retried push (same op ids) is answered from receipts, not applied twice.
    tablet.delete_session(session).unwrap();
    tablet.sync_once(&mut tablet_link).unwrap();
    phone.sync_once(&mut phone_link).unwrap();
    assert_eq!(phone.sessions_in(week()).unwrap().sessions, []);

    // Row-level security: another user sees nothing of Alice's.
    let mut stranger = device(bob);
    let mut stranger_link = HttpTransport::new(&base, "bob").unwrap();
    let report = stranger.sync_once(&mut stranger_link).unwrap();
    assert_eq!(report.applied, 0);

    // A bad token is refused.
    let mut intruder = HttpTransport::new(&base, "nobody").unwrap();
    assert!(device(alice).sync_once(&mut intruder).is_err());

    // The receipt purge runs under its own policy.
    runtime.block_on(db.purge_receipts()).unwrap();
    assert!(cluster.socket_dir().exists());
}

fn op(entity: u16, row_id: Uuid, payload: Vec<u8>) -> PushOp {
    let now = u64::try_from(myos_server::db::now_ms()).unwrap();
    PushOp {
        op_id: Uuid::now_v7(),
        entity,
        row_id,
        op: 1,
        hlc: now << 16,
        node: 1,
        payload_ver: 1,
        payload,
    }
}

fn batch(ops: Vec<PushOp>) -> PushBatch {
    PushBatch {
        device_id: Uuid::now_v7(),
        proto: 1,
        ops,
    }
}

/// Ops the server must refuse: entity and payload version out of range,
/// garbage TLV, a future field clock, a future row clock, an oversized
/// payload.
fn poison() -> Vec<PushOp> {
    let mut future_clock = Tlv::default();
    let mut clocks = std::collections::BTreeMap::new();
    clocks.insert(
        10,
        merge::FieldClock {
            hlc: u64::MAX >> 2,
            node: 1,
            hash: 0,
        },
    );
    future_clock.put_record(merge::TAG_FIELD_CLOCKS, &merge::write_clocks(&clocks));
    vec![
        op(40_000, Uuid::now_v7(), vec![]),
        PushOp {
            payload_ver: 40_000,
            ..op(2, Uuid::now_v7(), vec![])
        },
        op(2, Uuid::now_v7(), vec![0xff, 0xff]),
        op(2, Uuid::now_v7(), future_clock.encode()),
        PushOp {
            hlc: u64::MAX,
            ..op(2, Uuid::now_v7(), vec![])
        },
        op(2, Uuid::now_v7(), vec![0; 2 * 1024 * 1024]),
    ]
}

#[test]
fn server_rules_hold() {
    let Some(cluster) = Cluster::start() else {
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let db = Db::new(cluster.config(), 8).unwrap();
    runtime.block_on(db.migrate()).unwrap();
    let alice = Uuid::now_v7();
    let bob = Uuid::now_v7();
    let mut payload = Tlv::default();
    payload.put_uint(10, 5);
    let good = op(2, Uuid::now_v7(), payload.encode());
    let receipt = good.op_id;

    // A true retry is answered from the receipt; the same id with other
    // content is refused.
    let first = runtime
        .block_on(db.push(alice, &batch(vec![good.clone()])))
        .unwrap();
    let retry = runtime
        .block_on(db.push(alice, &batch(vec![good.clone()])))
        .unwrap();
    assert_eq!(first, retry);
    let reused = PushOp {
        payload: vec![],
        ..good
    };
    let replayed = runtime
        .block_on(db.push(alice, &batch(vec![reused])))
        .unwrap();
    assert_eq!(
        replayed[0].result,
        PushResult::Rejected {
            code: reject::REPLAYED
        }
    );

    // Poison inputs are refused, not stored, and do not fail the batch.
    let answers = runtime.block_on(db.push(alice, &batch(poison()))).unwrap();
    let codes: Vec<_> = answers
        .iter()
        .map(|answer| match answer.result {
            PushResult::Rejected { code } => code,
            other => panic!("accepted poison: {other:?}"),
        })
        .collect();
    assert_eq!(
        codes,
        vec![
            reject::INVALID,
            reject::INVALID,
            reject::INVALID,
            reject::FUTURE_CLOCK,
            reject::FUTURE_CLOCK,
            reject::TOO_LARGE
        ]
    );

    // Concurrent pushes of one user to the same new row: one wins, the
    // other merges or is superseded; revisions stay unique and contiguous.
    let row = Uuid::now_v7();
    let pushes: Vec<_> = (0..8)
        .map(|value| {
            let db = db.clone();
            let mut payload = Tlv::default();
            payload.put_uint(10, value);
            let push = op(2, row, payload.encode());
            runtime.spawn(async move { db.push(alice, &batch(vec![push])).await })
        })
        .collect();
    for push in pushes {
        runtime.block_on(push).unwrap().unwrap();
    }
    let page = runtime.block_on(db.pull(alice, 0, 1000, 1)).unwrap();
    let revs: Vec<i64> = page.rows.iter().map(|row| row.server_rev).collect();
    assert!(revs.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        page.rows
            .iter()
            .filter(|stored| stored.row_id == row)
            .count(),
        1
    );
    assert!(!page.more);

    // Paging: `next` / `more` at the row limit.
    let first_page = runtime.block_on(db.pull(alice, 0, 1, 1)).unwrap();
    assert_eq!(first_page.rows.len(), 1);
    assert!(first_page.more);
    assert_eq!(first_page.next, first_page.rows[0].server_rev);

    runtime.block_on(check_row_level_security(&cluster, alice, bob, receipt));
    assert_eq!(runtime.block_on(db.purge_receipts()).unwrap(), 1);
}

/// Row-level security straight in SQL, as the server role; backdates the
/// receipt of `old_op` so the purge has one to delete.
async fn check_row_level_security(cluster: &Cluster, alice: Uuid, bob: Uuid, old_op: Uuid) {
    let (mut client, connection) = cluster
        .config()
        .connect(tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(connection);
    let count = |sql: &'static str| {
        let client = &client;
        async move { client.query_one(sql, &[]).await.unwrap().get::<_, i64>(0) }
    };
    assert_eq!(
        count("SELECT count(*) FROM sync_row").await,
        0,
        "no user set"
    );
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('app.user_id', $1, true)",
        &[&bob.to_string()],
    )
    .await
    .unwrap();
    let seen: i64 = tx
        .query_one("SELECT count(*) FROM sync_row", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(seen, 0, "bob sees none of alice's rows");
    let forged = tx
        .execute(
            "INSERT INTO user_rev (user_id, rev) VALUES ($1, 0)",
            &[&alice],
        )
        .await;
    assert!(forged.is_err(), "WITH CHECK refuses another user's id");
    tx.rollback().await.unwrap();

    let tx = client.transaction().await.unwrap();
    tx.execute("SELECT set_config('app.maintenance', 'on', true)", &[])
        .await
        .unwrap();
    let read: i64 = tx
        .query_one("SELECT count(*) FROM push_receipt", &[])
        .await
        .unwrap()
        .get(0);
    assert!(read > 0, "maintenance reads receipts to purge them");
    let written = tx
        .execute("UPDATE push_receipt SET outcome = 1", &[])
        .await
        .unwrap();
    assert_eq!(written, 0, "maintenance never writes");
    tx.rollback().await.unwrap();

    // Purge removes old receipts only.
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('app.user_id', $1, true)",
        &[&alice.to_string()],
    )
    .await
    .unwrap();
    tx.execute(
        "UPDATE push_receipt SET at = now() - interval '40 days' WHERE op_id = $1",
        &[&old_op],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}
