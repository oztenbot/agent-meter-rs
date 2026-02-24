use crate::signing::verify_signature;
use crate::transport::{MemoryTransport, Transport, TransportError};
use crate::types::{AgentIdentity, PricingModel, RouteOptions, UsageRecord};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Configuration for AgentMeter.
pub struct MeterConfig {
    /// Required: identifies your service in all usage records.
    pub service_id: String,
    /// Where records are sent. Defaults to MemoryTransport if None.
    pub transport: Option<Arc<dyn Transport>>,
    /// Default pricing model for all routes.
    pub default_pricing: Option<PricingModel>,
    /// Custom agent identity extractor. Defaults to reading agent_id field.
    pub identify_agent:
        Option<Box<dyn Fn(&IncomingRequest) -> Option<AgentIdentity> + Send + Sync>>,
    /// HMAC secret for verifying request signatures.
    pub signing_secret: Option<String>,
    /// Transform or filter records before they're emitted. Return None to drop.
    pub before_emit: Option<Box<dyn Fn(UsageRecord) -> Option<UsageRecord> + Send + Sync>>,
    /// Whether to meter 4xx/5xx responses. Default: false.
    pub meter_errors: bool,
}

impl Default for MeterConfig {
    fn default() -> Self {
        Self {
            service_id: String::new(),
            transport: None,
            default_pricing: None,
            identify_agent: None,
            signing_secret: None,
            before_emit: None,
            meter_errors: false,
        }
    }
}

/// The core metering engine. Cheap to clone — wraps an Arc internally.
#[derive(Clone)]
pub struct AgentMeter {
    config: Arc<MeterConfigInner>,
    transport: Arc<dyn Transport>,
}

// Non-Clone inner config (closures aren't Clone)
struct MeterConfigInner {
    service_id: String,
    default_pricing: Option<PricingModel>,
    identify_agent: Option<Box<dyn Fn(&IncomingRequest) -> Option<AgentIdentity> + Send + Sync>>,
    signing_secret: Option<String>,
    before_emit: Option<Box<dyn Fn(UsageRecord) -> Option<UsageRecord> + Send + Sync>>,
    meter_errors: bool,
}

impl AgentMeter {
    pub fn new(config: MeterConfig) -> Self {
        let transport: Arc<dyn Transport> = config
            .transport
            .unwrap_or_else(|| Arc::new(MemoryTransport::new()));

        Self {
            config: Arc::new(MeterConfigInner {
                service_id: config.service_id,
                default_pricing: config.default_pricing,
                identify_agent: config.identify_agent,
                signing_secret: config.signing_secret,
                before_emit: config.before_emit,
                meter_errors: config.meter_errors,
            }),
            transport,
        }
    }

    /// Record usage for a request. Non-blocking — spawns async task.
    pub fn record(&self, req: IncomingRequest, options: Option<RouteOptions>) {
        if options.as_ref().map_or(false, |o| o.skip) {
            return;
        }

        let agent = match self.identify_agent(&req) {
            Some(a) => a,
            None => return,
        };

        if let Some(ref secret) = self.config.signing_secret {
            if let Some(ref sig) = req.request_signature {
                let body = req.body.as_deref().unwrap_or("");
                if !verify_signature(body, sig, secret) {
                    return;
                }
            }
        }

        if !self.config.meter_errors && req.status_code.map_or(false, |s| s >= 400) {
            return;
        }

        let units = options.as_ref().and_then(|o| o.units).unwrap_or(1.0);

        let mut record = UsageRecord {
            id: Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            service_id: self.config.service_id.clone(),
            agent,
            operation: options
                .as_ref()
                .and_then(|o| o.operation.clone())
                .unwrap_or_else(|| {
                    format!(
                        "{} {}",
                        req.method.as_deref().unwrap_or("UNKNOWN"),
                        req.path.as_deref().unwrap_or("/")
                    )
                }),
            units,
            unit_type: options
                .as_ref()
                .and_then(|o| o.unit_type.clone())
                .unwrap_or_else(|| "request".to_string()),
            pricing_model: options
                .as_ref()
                .and_then(|o| o.pricing.clone())
                .or_else(|| self.config.default_pricing.clone())
                .unwrap_or(PricingModel::PerCall),
            method: req.method.clone().unwrap_or_else(|| "UNKNOWN".to_string()),
            path: req.path.clone().unwrap_or_else(|| "/".to_string()),
            status_code: req.status_code.unwrap_or(0),
            duration_ms: req.duration_ms.unwrap_or(0),
            request_signature: req.request_signature.clone(),
            metadata: options.as_ref().and_then(|o| o.metadata.clone()),
        };

        if let Some(ref hook) = self.config.before_emit {
            record = match hook(record) {
                Some(r) => r,
                None => return,
            };
        }

        let transport = Arc::clone(&self.transport);
        tokio::spawn(async move {
            let _ = transport.send(record).await;
        });
    }

    fn identify_agent(&self, req: &IncomingRequest) -> Option<AgentIdentity> {
        if let Some(ref identify) = self.config.identify_agent {
            return identify(req);
        }

        req.agent_id.as_ref().map(|id| AgentIdentity {
            agent_id: id.clone(),
            name: req.agent_name.clone(),
            shepherd_id: None,
            tier: None,
        })
    }

    pub fn transport(&self) -> Arc<dyn Transport> {
        Arc::clone(&self.transport)
    }

    pub async fn flush(&self) -> Result<(), TransportError> {
        self.transport.flush().await
    }
}

/// Framework-agnostic request/response representation.
#[derive(Debug, Default, Clone)]
pub struct IncomingRequest {
    pub method: Option<String>,
    pub path: Option<String>,
    pub agent_id: Option<String>,
    pub agent_name: Option<String>,
    pub request_signature: Option<String>,
    pub body: Option<String>,
    pub status_code: Option<u16>,
    pub duration_ms: Option<u64>,
    /// Extra headers — used by framework adapters (e.g. axum).
    pub headers: HashMap<String, String>,
}
