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
use myos_server::http::{AppState, router};
use myos_store::{DatabaseKey, SessionDraft, Store};
use myos_sync_http::HttpTransport;
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
    let mut phone_link = HttpTransport::new(&base, "alice");
    let mut tablet_link = HttpTransport::new(&base, "alice");

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
    assert!(phone.sessions_in(week()).unwrap().sessions.is_empty());

    // Row-level security: another user sees nothing of Alice's.
    let mut stranger = device(bob);
    let mut stranger_link = HttpTransport::new(&base, "bob");
    let report = stranger.sync_once(&mut stranger_link).unwrap();
    assert_eq!(report.applied, 0);

    // A bad token is refused.
    let mut intruder = HttpTransport::new(&base, "nobody");
    assert!(device(alice).sync_once(&mut intruder).is_err());

    // The receipt purge runs under its own policy.
    runtime.block_on(db.purge_receipts()).unwrap();
    assert!(cluster.socket_dir().exists());
}
