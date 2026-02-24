//! # agent-meter-client
//!
//! Agent-side SDK for agent-meter. Handles request signing, logging, and reconciliation.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use agent_meter_client::{AgentClient, ClientConfig};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let client = AgentClient::new(ClientConfig {
//!     agent_id: "agent-y".to_string(),
//!     agent_name: Some("MyAgent".to_string()),
//!     signing_secret: Some("shared-secret".to_string()),
//!     service_url: "http://localhost:3000".to_string(),
//! });
//!
//! // Make a signed request
//! let body = serde_json::json!({"prompt": "hello"}).to_string();
//! let resp = client.call("POST", "/api/generate", Some(&body)).await?;
//! println!("status: {}, receipt: {:?}", resp.status_code, resp.receipt);
//!
//! // Reconcile against service records
//! let report = client.reconcile().await?;
//! println!("matched: {}", report.summary.matched);
//! println!("agent_only: {}", report.summary.agent_only_count);
//! println!("service_only: {}", report.summary.service_only_count);
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod log;
pub mod reconcile;

pub use client::{AgentClient, ClientConfig, ClientError, ClientResponse};
pub use log::{LoggedRequest, RequestLog};
pub use reconcile::{reconcile, MatchedRecord, ReconciliationReport, ReconciliationSummary};
