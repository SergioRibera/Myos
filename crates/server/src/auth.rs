//! Who is calling. The identity provider is not chosen yet: anything that
//! maps a bearer token to a user id plugs in here.

use uuid::Uuid;

/// Maps a bearer token to the user it belongs to.
pub trait Authenticator: Send + Sync + 'static {
    /// The user of `token`, or `None` when it is not valid.
    fn authenticate(&self, token: &str) -> Option<Uuid>;
}

/// Static tokens for development and tests, from `MYOS_DEV_TOKENS`
/// (`token=uuid,token=uuid`). Never enabled in release builds.
#[cfg(feature = "dev-auth")]
#[derive(Debug, Default, Clone)]
pub struct DevTokens {
    tokens: std::collections::HashMap<String, Uuid>,
}

#[cfg(feature = "dev-auth")]
impl DevTokens {
    /// Parse `token=uuid` pairs separated by commas; malformed pairs are
    /// skipped.
    #[must_use]
    pub fn parse(spec: &str) -> Self {
        let tokens = spec
            .split(',')
            .filter_map(|pair| {
                let (token, user) = pair.split_once('=')?;
                Some((token.trim().to_owned(), user.trim().parse().ok()?))
            })
            .collect();
        Self { tokens }
    }

    /// Add one token.
    #[must_use]
    pub fn with(mut self, token: &str, user: Uuid) -> Self {
        self.tokens.insert(token.to_owned(), user);
        self
    }
}

#[cfg(feature = "dev-auth")]
impl Authenticator for DevTokens {
    fn authenticate(&self, token: &str) -> Option<Uuid> {
        self.tokens.get(token).copied()
    }
}
