use super::{Transport, TransportError};
use crate::types::{AgentStats, OperationStats, QueryFilter, UsageRecord, UsageSummary};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// In-memory transport. Useful for testing and development.
///
/// Records are stored in a `Vec` protected by a `Mutex`. Cheap to clone —
/// all clones share the same underlying storage.
#[derive(Clone, Default)]
pub struct MemoryTransport {
    records: Arc<Mutex<Vec<UsageRecord>>>,
}

impl MemoryTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of all stored records.
    pub fn records(&self) -> Vec<UsageRecord> {
        self.records.lock().unwrap().clone()
    }

    /// Query records by filter.
    pub fn query(&self, filter: Option<&QueryFilter>) -> Vec<UsageRecord> {
        let records = self.records.lock().unwrap();
        records
            .iter()
            .filter(|r| apply_filter(r, filter))
            .cloned()
            .collect()
    }

    /// Count records matching filter.
    pub fn count(&self, filter: Option<&QueryFilter>) -> usize {
        let records = self.records.lock().unwrap();
        records.iter().filter(|r| apply_filter(r, filter)).count()
    }

    /// Summarize usage across all matching records.
    pub fn summary(&self, filter: Option<&QueryFilter>) -> UsageSummary {
        let records = self.records.lock().unwrap();
        let mut summary = UsageSummary::default();
        let mut agents: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for record in records.iter().filter(|r| apply_filter(r, filter)) {
            summary.total_records += 1;
            summary.total_units += record.units;
            agents.insert(record.agent.agent_id.clone());

            let op = summary
                .by_operation
                .entry(record.operation.clone())
                .or_insert_with(OperationStats::default);
            op.count += 1;
            op.units += record.units;

            let ag = summary
                .by_agent
                .entry(record.agent.agent_id.clone())
                .or_insert_with(AgentStats::default);
            ag.count += 1;
            ag.units += record.units;
        }

        summary.unique_agents = agents.len();
        summary
    }

    /// Clear all stored records.
    pub fn flush_sync(&self) {
        self.records.lock().unwrap().clear();
    }
}

fn apply_filter(record: &UsageRecord, filter: Option<&QueryFilter>) -> bool {
    let Some(f) = filter else { return true };

    if let Some(ref agent_id) = f.agent_id {
        if &record.agent.agent_id != agent_id {
            return false;
        }
    }
    if let Some(ref op) = f.operation {
        if &record.operation != op {
            return false;
        }
    }
    if let Some(ref svc) = f.service_id {
        if &record.service_id != svc {
            return false;
        }
    }
    if let Some(ref from) = f.from {
        if &record.timestamp < from {
            return false;
        }
    }
    if let Some(ref to) = f.to {
        if &record.timestamp > to {
            return false;
        }
    }
    true
}

impl Transport for MemoryTransport {
    fn send(
        &self,
        record: UsageRecord,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
        self.records.lock().unwrap().push(record);
        Box::pin(async { Ok(()) })
    }

    fn flush(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentIdentity, PricingModel};

    fn make_record(agent_id: &str, operation: &str, units: f64) -> UsageRecord {
        UsageRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: "2026-02-24T00:00:00.000Z".to_string(),
            service_id: "test".to_string(),
            agent: AgentIdentity {
                agent_id: agent_id.to_string(),
                name: None,
                shepherd_id: None,
                tier: None,
            },
            operation: operation.to_string(),
            units,
            unit_type: "request".to_string(),
            pricing_model: PricingModel::PerCall,
            method: "GET".to_string(),
            path: "/test".to_string(),
            status_code: 200,
            duration_ms: 10,
            request_signature: None,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn stores_and_queries() {
        let transport = MemoryTransport::new();
        transport.send(make_record("bot-1", "GET /a", 1.0)).await.unwrap();
        transport.send(make_record("bot-2", "GET /b", 2.0)).await.unwrap();

        let all = transport.query(None);
        assert_eq!(all.len(), 2);

        let filter = QueryFilter {
            agent_id: Some("bot-1".to_string()),
            ..Default::default()
        };
        let filtered = transport.query(Some(&filter));
        assert_eq!(filtered.len(), 1);

        let summary = transport.summary(None);
        assert_eq!(summary.total_records, 2);
        assert_eq!(summary.total_units, 3.0);
        assert_eq!(summary.unique_agents, 2);
    }
}
