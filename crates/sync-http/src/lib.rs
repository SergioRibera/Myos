//! `myos-sync-http`: the sync [`Transport`] over HTTPS.
//!
//! Blocking (ureq) on purpose: sync runs off the UI thread and the store is
//! executor-agnostic. TLS is rustls with the Mozilla roots built in, so no
//! platform TLS library or certificate store is needed on any target.

use std::time::Duration;

use myos_store::tlv::TlvError;
use myos_store::{PullPage, PushBatch, PushResponse, Transport};

/// Largest response body read: the largest message a client decodes.
const RESPONSE_LIMIT: u64 = myos_sync_wire::message::MESSAGE_LIMIT as u64;

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
    /// The server URL is not `https://` (plain HTTP is allowed only to a
    /// loopback address, for development).
    #[error("sync server must be reached over https")]
    Insecure,
    /// A message could not be encoded or decoded.
    #[error("malformed message")]
    Message(#[from] TlvError),
}

/// `https://`, or `http://` to a loopback host.
fn secure(base: &str) -> bool {
    if base.starts_with("https://") {
        return true;
    }
    let Some(rest) = base.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority
        .strip_prefix('[')
        .and_then(|bracketed| bracketed.split_once(']'))
        .map_or_else(
            || {
                authority
                    .rsplit_once(':')
                    .map_or(authority, |(host, _)| host)
            },
            |(host, _)| host,
        );
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
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
    /// authenticated with `token`. The token never travels in clear: plain
    /// `http://` is refused unless the host is loopback.
    pub fn new(base: &str, token: &str) -> Result<Self, HttpError> {
        if !secure(base) {
            return Err(HttpError::Insecure);
        }
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build();
        Ok(Self {
            agent: ureq::Agent::new_with_config(config),
            base: base.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
        })
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

#[cfg(test)]
mod tests {
    use super::secure;

    #[test]
    fn plain_http_only_to_loopback() {
        assert!(secure("https://sync.example.com"));
        assert!(secure("http://127.0.0.1:8080"));
        assert!(secure("http://[::1]:8080/"));
        assert!(secure("http://localhost"));
        assert!(!secure("http://sync.example.com"));
        assert!(!secure("http://127.0.0.1.evil.com"));
        assert!(!secure("ftp://127.0.0.1"));
    }
}
