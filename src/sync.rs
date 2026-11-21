//! Account sync: a background thread pushes the outbox and pulls the change
//! feed with its own connection to the database.
//!
//! The account (server, user, token, account key) lives in the credential
//! store next to the database key. Signing in is the identity provider's
//! job (not chosen yet); [`Account::from_env`] reads `MYOS_SYNC_ACCOUNT`
//! (`server|user|token|key-hex`) for development.

use std::fmt::Write as _;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use myos_store::Store;
use myos_sync_http::{HttpError, HttpTransport};
use myos_vault::Vault;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::storage::{self, StorageError};

/// Credential-store entry of the account.
const ACCOUNT_SECRET: &str = "sync-account";
/// Pause between syncs while everything works.
const IDLE: Duration = Duration::from_secs(5 * 60);
/// First and largest pause after a failure.
const RETRY_FIRST: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(30 * 60);

/// A signed-in account.
#[derive(Clone)]
pub struct Account {
    /// Sync server base URL.
    pub server: String,
    /// User id.
    pub user: Uuid,
    /// Bearer token.
    pub token: Zeroizing<String>,
    /// Account key (natural-key ids are derived from it on every device).
    pub key: Zeroizing<[u8; 32]>,
}

// Manual impl: token and key must not reach logs.
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("server", &self.server)
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

impl Account {
    fn encode(&self) -> Zeroizing<String> {
        let mut text = Zeroizing::new(format!(
            "v1\n{}\n{}\n{}\n",
            self.server, self.user, *self.token
        ));
        for byte in self.key.iter() {
            let _infallible = write!(text, "{byte:02x}");
        }
        text
    }

    fn decode(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != "v1" {
            return None;
        }
        let server = lines.next()?.to_owned();
        let user = lines.next()?.parse().ok()?;
        let token = Zeroizing::new(lines.next()?.to_owned());
        let key = parse_key(lines.next()?)?;
        Some(Self {
            server,
            user,
            token,
            key,
        })
    }

    /// Development account from `MYOS_SYNC_ACCOUNT`.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let spec = Zeroizing::new(std::env::var("MYOS_SYNC_ACCOUNT").ok()?);
        let mut parts = spec.split('|');
        Some(Self {
            server: parts.next()?.to_owned(),
            user: parts.next()?.parse().ok()?,
            token: Zeroizing::new(parts.next()?.to_owned()),
            key: parse_key(parts.next()?)?,
        })
    }

    /// The stored account, if any.
    pub fn load() -> Result<Option<Self>, StorageError> {
        let vault = Vault::open(storage::APP_ID)?;
        Ok(vault
            .secret(ACCOUNT_SECRET)?
            .and_then(|text| Self::decode(&text)))
    }

    /// Store this account.
    pub fn save(&self) -> Result<(), StorageError> {
        Vault::open(storage::APP_ID)?.set_secret(ACCOUNT_SECRET, &self.encode())?;
        Ok(())
    }

    /// Delete the stored account.
    pub fn forget() -> Result<(), StorageError> {
        Vault::open(storage::APP_ID)?.forget_secret(ACCOUNT_SECRET)?;
        Ok(())
    }
}

fn parse_key(text: &str) -> Option<Zeroizing<[u8; 32]>> {
    let text = text.trim();
    if text.len() != 64 {
        return None;
    }
    let mut key = Zeroizing::new([0_u8; 32]);
    for (slot, pair) in key.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(key)
}

/// What the sync thread last did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SyncState {
    /// Not started yet.
    #[default]
    Idle,
    /// Last round succeeded at this Unix time (ms).
    Synced {
        /// When.
        at_ms: i64,
    },
    /// Last round failed; retrying.
    Failing {
        /// Error chain, for diagnostics.
        error: String,
    },
    /// The server refused the token: the user must sign in again.
    SignInRequired,
    /// The database could not be opened for sync.
    Unavailable {
        /// Error chain.
        error: String,
    },
}

/// Handle to the running sync thread. Dropping it stops the thread after
/// its current round.
#[derive(Debug)]
pub struct Syncer {
    nudge: Sender<()>,
    state: Arc<Mutex<SyncState>>,
}

fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

impl Syncer {
    /// Start syncing `account` (already attached to the database).
    pub fn start(account: &Account) -> std::io::Result<Self> {
        let (nudge, nudged) = mpsc::channel();
        let state = Arc::new(Mutex::new(SyncState::Idle));
        let shared = Arc::clone(&state);
        let account = account.clone();
        std::thread::Builder::new()
            .name("myos-sync".into())
            .spawn(move || {
                let set = |next: SyncState| {
                    *shared.lock().unwrap_or_else(PoisonError::into_inner) = next;
                };
                let mut store = match storage::open_secondary() {
                    Ok(store) => store,
                    Err(err) => return set(SyncState::Unavailable { error: chain(&err) }),
                };
                let mut transport = match HttpTransport::new(&account.server, &account.token) {
                    Ok(transport) => transport,
                    Err(err) => return set(SyncState::Unavailable { error: chain(&err) }),
                };
                let mut pause = Duration::ZERO;
                loop {
                    match nudged.recv_timeout(pause) {
                        Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    match sync_round(&mut store, &mut transport) {
                        Ok(()) => {
                            set(SyncState::Synced { at_ms: now_ms() });
                            pause = IDLE;
                        }
                        Err(Round::SignIn) => return set(SyncState::SignInRequired),
                        Err(Round::Failed(error)) => {
                            log::warn!("sync failed: {error}");
                            set(SyncState::Failing { error });
                            pause = (pause * 2).clamp(RETRY_FIRST, RETRY_MAX);
                        }
                    }
                }
            })?;
        Ok(Self { nudge, state })
    }

    /// Sync now instead of waiting (after a local write, on app resume).
    pub fn request(&self) {
        let _stopped = self.nudge.send(());
    }

    /// What the thread last did.
    #[must_use]
    pub fn state(&self) -> SyncState {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

enum Round {
    SignIn,
    Failed(String),
}

fn sync_round(store: &mut Store, transport: &mut HttpTransport) -> Result<(), Round> {
    match store.sync_once(transport) {
        Ok(report) => {
            log::debug!("sync round: {report:?}");
            Ok(())
        }
        Err(myos_store::Error::Transport(error))
            if matches!(
                error.downcast_ref::<HttpError>(),
                Some(HttpError::Unauthorized)
            ) =>
        {
            Err(Round::SignIn)
        }
        Err(error) => Err(Round::Failed(chain(&error))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_round_trip_without_leaking() {
        let account = Account {
            server: "https://sync.example.com".into(),
            user: Uuid::from_u128(7),
            token: Zeroizing::new("secret-token".into()),
            key: Zeroizing::new([0xab; 32]),
        };
        let decoded = Account::decode(&account.encode()).expect("round trip");
        assert_eq!(decoded.user, account.user);
        assert_eq!(*decoded.key, *account.key);
        assert!(!format!("{account:?}").contains("secret"));
        assert!(Account::decode("v2\nx").is_none());
    }
}
