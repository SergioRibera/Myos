//! `myos-sync-http`: the sync [`Transport`] over HTTPS.
//!
//! Blocking (ureq) on purpose: sync runs off the UI thread and the store is
//! executor-agnostic. TLS is rustls with the Mozilla roots built in, so no
//! platform TLS library or certificate store is needed on any target.

use std::time::Duration;

use myos_store::tlv::TlvError;
use myos_store::{PullPage, PushBatch, PushResponse, Transport};

/// Largest response body read.
const RESPONSE_LIMIT: u64 = 16 * 1024 * 1024;

/// Transport failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HttpError {
    /// The request could not be sent or the response read.
    #[error("request failed")]
    Request(#[from] ureq::Error),
    /// The server answered with an error status.
    #[error("server answered {0}")]
    Status(u16),
    /// The token was refused: sign in again.
    #[error("not authorized")]
    Unauthorized,
    /// A message could not be encoded or decoded.
    #[error("malformed message")]
    Message(#[from] TlvError),
}

/// HTTPS transport to a sync server.
pub struct HttpTransport {
    agent: ureq::Agent,
    base: String,
    token: String,
}

// Manual impl: the token must not reach logs.
impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpTransport")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl HttpTransport {
    /// Transport to the server at `base` (e.g. `https://sync.example.com`),
    /// authenticated with `token`.
    #[must_use]
    pub fn new(base: &str, token: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            base: base.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
        }
    }

    fn authorization(&self) -> String {
        format!("Bearer {}", self.token)
    }

    fn read(response: ureq::http::Response<ureq::Body>) -> Result<Vec<u8>, HttpError> {
        match response.status().as_u16() {
            200 => Ok(response
                .into_body()
                .into_with_config()
                .limit(RESPONSE_LIMIT)
                .read_to_vec()?),
            401 => Err(HttpError::Unauthorized),
            status => Err(HttpError::Status(status)),
        }
    }
}

impl Transport for HttpTransport {
    type Error = HttpError;

    fn push(&mut self, batch: &PushBatch) -> Result<PushResponse, HttpError> {
        let response = self
            .agent
            .post(format!("{}/v1/push", self.base))
            .header("Authorization", self.authorization())
            .header("Content-Type", "application/octet-stream")
            .send(batch.encode()?)?;
        Ok(PushResponse::decode(&Self::read(response)?)?)
    }

    fn pull(&mut self, stream: u16, after: i64, limit: u32) -> Result<PullPage, HttpError> {
        let response = self
            .agent
            .get(format!("{}/v1/pull", self.base))
            .header("Authorization", self.authorization())
            .query("stream", stream.to_string())
            .query("after", after.to_string())
            .query("limit", limit.to_string())
            .call()?;
        Ok(PullPage::decode(&Self::read(response)?)?)
    }
}
