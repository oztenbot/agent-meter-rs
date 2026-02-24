use crate::log::RequestLog;
use crate::reconcile::{reconcile, ReconciliationReport};
use agent_meter::{sign_payload, UsageRecord};
use reqwest::Client;
use std::time::Instant;
use thiserror::Error;

/// Errors returned by AgentClient.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Configuration for AgentClient.
#[derive(Clone)]
pub struct ClientConfig {
    /// Stable agent identifier. Sent as X-Agent-Id. The accounting key.
    pub agent_id: String,
    /// Human-friendly display name. Sent as X-Agent-Name. Cosmetic only.
    pub agent_name: Option<String>,
    /// HMAC secret shared with Service X. Signs request bodies.
    /// If None, requests are sent unsigned (no X-Agent-Signature header).
    pub signing_secret: Option<String>,
    /// Base URL of Service X (e.g. "http://localhost:3000").
    pub service_url: String,
}

/// Response from a signed request.
pub struct ClientResponse {
    pub status_code: u16,
    pub body: String,
    /// X-Usage-Receipt returned by Service X. Proves the request was metered.
    pub receipt: Option<String>,
}

/// Agent-side client. Handles signing, request logging, and reconciliation.
///
/// Cheap to clone — all clones share the same `RequestLog`.
#[derive(Clone)]
pub struct AgentClient {
    pub config: ClientConfig,
    http: Client,
    /// All requests made by this client, for reconciliation.
    pub log: RequestLog,
}

impl AgentClient {
    pub fn new(config: ClientConfig) -> Self {
        Self {
            config,
            http: Client::new(),
            log: RequestLog::new(),
        }
    }

    /// Make a signed request to the service and log it.
    ///
    /// The request body is HMAC-signed before sending. The signature is sent
    /// as `X-Agent-Signature` and stored in the local `RequestLog` for reconciliation.
    pub async fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<ClientResponse, ClientError> {
        let url = format!("{}{}", self.config.service_url.trim_end_matches('/'), path);
        let body_str = body.unwrap_or("");

        let signature = self
            .config
            .signing_secret
            .as_ref()
            .map(|secret| sign_payload(body_str, secret));

        let log_id = self.log.log(method, &url, signature.clone());

        let start = Instant::now();

        let mut builder = match method.to_uppercase().as_str() {
            "POST" => self.http.post(&url),
            "PUT" => self.http.put(&url),
            "PATCH" => self.http.patch(&url),
            "DELETE" => self.http.delete(&url),
            _ => self.http.get(&url),
        };

        builder = builder
            .header("X-Agent-Id", &self.config.agent_id)
            .header("Content-Type", "application/json");

        if let Some(name) = &self.config.agent_name {
            builder = builder.header("X-Agent-Name", name);
        }
        if let Some(sig) = &signature {
            builder = builder.header("X-Agent-Signature", sig);
        }
        if !body_str.is_empty() {
            builder = builder.body(body_str.to_string());
        }

        let resp = builder.send().await?;
        let duration_ms = start.elapsed().as_millis() as u64;
        let status_code = resp.status().as_u16();

        let receipt = resp
            .headers()
            .get("x-usage-receipt")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let body_text = resp.text().await?;

        self.log
            .update_response(&log_id, status_code, duration_ms, receipt.clone());

        Ok(ClientResponse {
            status_code,
            body: body_text,
            receipt,
        })
    }

    /// Download usage records from Service X's `/v1/usage/me` endpoint.
    ///
    /// Filters by this agent's ID. Does not sign the request (management endpoint).
    pub async fn download_usage(&self) -> Result<Vec<UsageRecord>, ClientError> {
        let url = format!(
            "{}/v1/usage/me?agent_id={}",
            self.config.service_url.trim_end_matches('/'),
            self.config.agent_id,
        );
        let records: Vec<UsageRecord> = self.http.get(&url).send().await?.json().await?;
        Ok(records)
    }

    /// Reconcile the local request log against Service X's usage records.
    ///
    /// Downloads records from `/v1/usage/me`, then diffs them against the local
    /// `RequestLog` using `requestSignature` as the correlation key.
    pub async fn reconcile(&self) -> Result<ReconciliationReport, ClientError> {
        let service_records = self.download_usage().await?;
        let log_entries = self.log.entries();
        Ok(reconcile(&log_entries, &service_records))
    }
}
