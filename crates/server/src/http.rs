//! HTTP surface: `POST /v1/push`, `GET /v1/pull`, `GET /health`.
//!
//! Bodies are TLV messages ([`myos_sync_wire::message`]), authenticated with
//! `Authorization: Bearer <token>`.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use myos_sync_wire::message::MESSAGE_LIMIT;
use myos_sync_wire::protocol::{PushBatch, PushResponse};
use tower::limit::ConcurrencyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use uuid::Uuid;

use crate::auth::Authenticator;
use crate::db::{Db, MAX_OPS, now_ms};

/// Shared state of the handlers.
#[derive(Clone)]
pub struct AppState {
    /// Sync store.
    pub db: Db,
    /// Token → user.
    pub auth: Arc<dyn Authenticator>,
    /// Oldest client protocol still allowed to write.
    pub min_proto: u32,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("min_proto", &self.min_proto)
            .finish_non_exhaustive()
    }
}

/// Requests handled at once.
const MAX_CONCURRENT: usize = 64;

/// Query of `GET /v1/pull`.
#[derive(Debug)]
struct PullQuery {
    stream: u16,
    after: i64,
    limit: u32,
}

impl PullQuery {
    fn parse(query: &std::collections::HashMap<String, String>) -> Option<Self> {
        Some(Self {
            stream: query
                .get("stream")
                .map_or(Some(1), |value| value.parse().ok())?,
            after: query
                .get("after")
                .map_or(Some(0), |value| value.parse().ok())?,
            limit: query
                .get("limit")
                .map_or(Some(500), |value| value.parse().ok())?,
        })
    }
}

/// The router with every route.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/push", post(push))
        .route("/v1/pull", get(pull))
        .layer(DefaultBodyLimit::max(MESSAGE_LIMIT))
        // Requests past these wait in line or time out instead of piling up
        // memory (a push decodes up to the message limit several times).
        .layer(ConcurrencyLimitLayer::new(MAX_CONCURRENT))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .with_state(state)
}

fn user(state: &AppState, headers: &HeaderMap) -> Result<Uuid, StatusCode> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .and_then(|token| state.auth.authenticate(token))
        .ok_or(StatusCode::UNAUTHORIZED)
}

fn binary(bytes: Vec<u8>) -> Response {
    ([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response()
}

fn internal(error: &dyn std::error::Error) -> StatusCode {
    let mut chain = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        chain.push_str(": ");
        chain.push_str(&cause.to_string());
        source = cause.source();
    }
    tracing::error!(error = %chain, "request failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

async fn push(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let user = match user(&state, &headers) {
        Ok(user) => user,
        Err(status) => return status.into_response(),
    };
    let Ok(batch) = PushBatch::decode(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if batch.ops.len() > MAX_OPS {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    // Clients older than the minimum protocol may not write; the response
    // tells them so and they turn read-only.
    let outcomes = if batch.proto < state.min_proto {
        Vec::new()
    } else {
        match state.db.push(user, &batch).await {
            Ok(outcomes) => outcomes,
            Err(error) => return internal(&error).into_response(),
        }
    };
    let response = PushResponse {
        outcomes,
        server_time_ms: now_ms(),
        min_proto: state.min_proto,
    };
    match response.encode() {
        Ok(bytes) => binary(bytes),
        Err(error) => internal(&error).into_response(),
    }
}

async fn pull(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let user = match user(&state, &headers) {
        Ok(user) => user,
        Err(status) => return status.into_response(),
    };
    let Some(query) = PullQuery::parse(&query) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if query.stream != myos_sync_wire::protocol::STREAM_OWN {
        return StatusCode::NOT_FOUND.into_response();
    }
    match state
        .db
        .pull(user, query.after, query.limit, state.min_proto)
        .await
    {
        Ok(page) => match page.encode() {
            Ok(bytes) => binary(bytes),
            Err(error) => internal(&error).into_response(),
        },
        Err(error) => internal(&error).into_response(),
    }
}
