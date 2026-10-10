//! One error type for everything that talks to Basecamp, with the
//! questions the UI asks of it: is it worth retrying, is the token dead?

use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    /// Basecamp answered with a status the request can't recover from.
    #[error("HTTP {status}: {message}")]
    Http { status: u16, message: String },
    /// Too many requests; wait this long before the next one.
    #[error("rate limited, retry after {0:?}")]
    RateLimited(Duration),
    /// The token was rejected: expired beyond refresh, or revoked.
    #[error("not signed in")]
    Unauthorized,
    #[error("unexpected response: {0}")]
    Decode(#[from] serde_json::Error),
    /// Something in the OAuth dance itself: a bad state, a denied prompt.
    #[error("sign-in failed: {0}")]
    Auth(String),
    /// Writes to this account are off by the development guard
    /// (`CAMPFE_WRITE_ACCOUNTS`).
    #[error("writes to this account are turned off")]
    ReadOnly,
    #[error("keyring: {0}")]
    Keyring(String),
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl ApiError {
    /// A later try may succeed without anyone changing anything.
    pub fn is_transient(&self) -> bool {
        match self {
            ApiError::Network(_) | ApiError::RateLimited(_) => true,
            ApiError::Http { status, .. } => *status >= 500,
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;
