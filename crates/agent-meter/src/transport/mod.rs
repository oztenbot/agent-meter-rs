use crate::types::UsageRecord;
use std::future::Future;
use std::pin::Pin;

pub mod attestation;
pub mod http;
pub mod memory;

#[cfg(feature = "sqlite")]
pub mod sqlite;

/// Error type for transport failures.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Rate limited: retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    #[cfg(feature = "sqlite")]
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Other(String),
}

/// Core transport trait. Implement this to send records anywhere.
pub trait Transport: Send + Sync {
    fn send(
        &self,
        record: UsageRecord,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>>;

    fn flush(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

pub use attestation::AttestationTransport;
pub use http::HttpTransport;
pub use memory::MemoryTransport;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteTransport;
