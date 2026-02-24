//! # agent-meter
//!
//! Usage metering for the agent economy. Drop-in metering for APIs that serve AI agents.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use agent_meter::{AgentMeter, MeterConfig, MemoryTransport};
//! use std::sync::Arc;
//!
//! let transport = Arc::new(MemoryTransport::new());
//! let meter = AgentMeter::new(MeterConfig {
//!     service_id: "my-api".to_string(),
//!     transport: Some(Arc::new(MemoryTransport::new())),
//!     ..Default::default()
//! });
//! ```

pub mod meter;
pub mod signing;
pub mod transport;
pub mod types;

// Re-export at crate root
pub use meter::{AgentMeter, IncomingRequest, MeterConfig};
pub use signing::{sign_payload, verify_signature};
pub use transport::{AttestationTransport, HttpTransport, MemoryTransport, Transport};
pub use types::{
    AgentIdentity, AgentStats, Attestation, OperationStats, PricingModel, QueryFilter,
    RouteOptions, UsageRecord, UsageSummary,
};

#[cfg(feature = "sqlite")]
pub use transport::SqliteTransport;
