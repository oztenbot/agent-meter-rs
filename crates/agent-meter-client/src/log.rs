use chrono::Utc;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// A single outgoing request logged by the agent.
#[derive(Debug, Clone)]
pub struct LoggedRequest {
    /// Unique ID for this log entry (not the same as UsageRecord.id on service side).
    pub id: String,
    pub timestamp: String,
    pub method: String,
    pub url: String,
    /// HMAC signature sent as X-Agent-Signature (the correlation key).
    pub signature: Option<String>,
    /// X-Usage-Receipt header returned by Service X.
    pub receipt: Option<String>,
    pub status_code: Option<u16>,
    pub duration_ms: Option<u64>,
}

/// Thread-safe request log. Cheap to clone — all clones share the same buffer.
#[derive(Clone, Default)]
pub struct RequestLog {
    inner: Arc<Mutex<Vec<LoggedRequest>>>,
}

impl RequestLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an outgoing request. Returns the entry ID for later update.
    pub fn log(&self, method: &str, url: &str, signature: Option<String>) -> String {
        let id = Uuid::new_v4().to_string();
        let entry = LoggedRequest {
            id: id.clone(),
            timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            method: method.to_string(),
            url: url.to_string(),
            signature,
            receipt: None,
            status_code: None,
            duration_ms: None,
        };
        self.inner.lock().unwrap().push(entry);
        id
    }

    /// Update an existing log entry with response data.
    pub fn update_response(&self, id: &str, status_code: u16, duration_ms: u64, receipt: Option<String>) {
        let mut log = self.inner.lock().unwrap();
        if let Some(entry) = log.iter_mut().find(|e| e.id == id) {
            entry.status_code = Some(status_code);
            entry.duration_ms = Some(duration_ms);
            entry.receipt = receipt;
        }
    }

    /// Return all logged entries.
    pub fn entries(&self) -> Vec<LoggedRequest> {
        self.inner.lock().unwrap().clone()
    }

    /// Number of logged requests.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }
}
