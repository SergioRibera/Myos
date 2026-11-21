//! Sync server binary.
//!
//! Environment: `MYOS_DATABASE_URL` (libpq style), `MYOS_LISTEN`
//! (default `127.0.0.1:8080`), `MYOS_MIN_PROTO` (default 1), `RUST_LOG`;
//! with the `dev-auth` feature, `MYOS_DEV_TOKENS` (`token=uuid,…`).

use std::sync::Arc;
use std::time::Duration;

use myos_server::auth::Authenticator;
use myos_server::db::Db;
use myos_server::http::{AppState, router};

#[derive(Debug, thiserror::Error)]
enum StartError {
    #[error("MYOS_DATABASE_URL is not set")]
    NoDatabase,
    #[error("MYOS_DATABASE_URL is invalid")]
    DatabaseUrl(#[source] tokio_postgres::Error),
    #[error("database setup failed")]
    Database(#[from] myos_server::db::DbError),
    #[error("no authenticator configured (build with an identity provider or `dev-auth`)")]
    NoAuth,
    #[error("cannot listen")]
    Listen(#[source] std::io::Error),
}

fn authenticator() -> Result<Arc<dyn Authenticator>, StartError> {
    #[cfg(feature = "dev-auth")]
    if let Ok(spec) = std::env::var("MYOS_DEV_TOKENS") {
        tracing::warn!("development tokens enabled");
        return Ok(Arc::new(myos_server::auth::DevTokens::parse(&spec)));
    }
    Err(StartError::NoAuth)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let url = std::env::var("MYOS_DATABASE_URL").map_err(|_| StartError::NoDatabase)?;
    let config = url.parse().map_err(StartError::DatabaseUrl)?;
    let db = Db::new(config, 16).map_err(StartError::Database)?;
    db.migrate().await.map_err(StartError::Database)?;
    let state = AppState {
        db: db.clone(),
        auth: authenticator()?,
        min_proto: std::env::var("MYOS_MIN_PROTO")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1),
    };
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            if let Err(error) = db.purge_receipts().await {
                tracing::warn!(%error, "receipt purge failed");
            }
        }
    });
    let listen = std::env::var("MYOS_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .map_err(StartError::Listen)?;
    tracing::info!(%listen, "myos server listening");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _signal = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(StartError::Listen)?;
    Ok(())
}
