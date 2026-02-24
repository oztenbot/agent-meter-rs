use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Identifies an AI agent making requests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentIdentity {
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The human or entity the agent acts on behalf of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shepherd_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
}

/// How usage is priced.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum PricingModel {
    PerCall,
    PerUnit,
    PerMinute,
    Tiered,
    Custom,
}

impl Default for PricingModel {
    fn default() -> Self {
        PricingModel::PerCall
    }
}

impl std::fmt::Display for PricingModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PricingModel::PerCall => "per-call",
            PricingModel::PerUnit => "per-unit",
            PricingModel::PerMinute => "per-minute",
            PricingModel::Tiered => "tiered",
            PricingModel::Custom => "custom",
        };
        write!(f, "{}", s)
    }
}

/// A single metered usage event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    pub id: String,
    pub timestamp: String,
    pub service_id: String,
    pub agent: AgentIdentity,
    pub operation: String,
    pub units: f64,
    pub unit_type: String,
    pub pricing_model: PricingModel,
    pub method: String,
    pub path: String,
    pub status_code: u16,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// A Merkle-attested batch of usage records.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attestation {
    pub batch_id: String,
    pub timestamp: String,
    pub service_id: String,
    pub record_count: usize,
    pub merkle_root: String,
    pub signature: String,
    pub records: Vec<UsageRecord>,
}

/// Filter for querying usage records.
#[derive(Debug, Clone, Default)]
pub struct QueryFilter {
    pub agent_id: Option<String>,
    pub operation: Option<String>,
    pub service_id: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub pricing_model: Option<PricingModel>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Aggregated usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub total_records: usize,
    pub total_units: f64,
    pub unique_agents: usize,
    pub by_operation: HashMap<String, OperationStats>,
    pub by_agent: HashMap<String, AgentStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperationStats {
    pub count: usize,
    pub units: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentStats {
    pub count: usize,
    pub units: f64,
}

/// Per-route metering options.
#[derive(Debug, Clone, Default)]
pub struct RouteOptions {
    /// Override the operation name. Default: "{METHOD} {path}".
    pub operation: Option<String>,
    /// Number of units for this request. Default: 1.
    pub units: Option<f64>,
    /// Unit type label. Default: "request".
    pub unit_type: Option<String>,
    /// Pricing model. Overrides the meter default.
    pub pricing: Option<PricingModel>,
    /// Arbitrary metadata to attach to the record.
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// Skip metering for this route entirely.
    pub skip: bool,
}
