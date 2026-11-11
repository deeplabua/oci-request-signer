//! Errors.

use std::fmt;
use std::path::PathBuf;

use thiserror::Error as ThisError;

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Anything reading the config, loading the key or calling OCI can fail with.
#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum Error {
    /// The OCI config file is missing, unreadable or incomplete.
    #[error("OCI config `{}`: {reason}", path.display())]
    Config {
        /// The config file we tried to use.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },

    /// The API signing key could not be loaded.
    #[error("API signing key `{}`: {reason}", path.display())]
    Key {
        /// The key file named in the OCI config.
        path: PathBuf,
        /// Why it cannot be used.
        reason: String,
    },

    /// OCI answered with an error status.
    #[error(transparent)]
    Api(#[from] ApiError),

    /// The request never got an answer: DNS, TLS, connection reset, timeout.
    #[error("network error: {0}")]
    Transport(String),
}

/// An error response from the OCI API.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ApiError {
    /// HTTP status code.
    pub status: u16,
    /// OCI error code, e.g. `InternalError` or `NotAuthenticated`.
    pub code: String,
    /// Human-readable message from OCI.
    pub message: String,
    /// `opc-request-id`, which Oracle support asks for.
    pub request_id: Option<String>,
    /// `Retry-After`, in seconds, when the service sent one (with a `429`).
    pub retry_after: Option<std::time::Duration>,
}

impl ApiError {
    /// An error with no request id and no `Retry-After` — for tests and for callers that
    /// map their own failures onto OCI's shape. (The struct is `#[non_exhaustive]`.)
    pub fn new(status: u16, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            request_id: None,
            retry_after: None,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} (HTTP {})", self.code, self.message, self.status)
    }
}

impl std::error::Error for ApiError {}
