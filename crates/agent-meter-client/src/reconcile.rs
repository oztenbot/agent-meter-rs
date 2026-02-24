use crate::log::LoggedRequest;
use agent_meter::UsageRecord;
use std::collections::{HashMap, HashSet};

/// A request logged by the agent that was also recorded by the service.
#[derive(Debug, Clone)]
pub struct MatchedRecord {
    pub log_entry: LoggedRequest,
    pub service_record: UsageRecord,
}

/// Summary counts from reconciliation.
#[derive(Debug, Clone, Default)]
pub struct ReconciliationSummary {
    /// Requests present in both the agent log and service records.
    pub matched: usize,
    /// Requests in the agent log with no matching service record.
    /// Common cause: 4xx/5xx responses when `meter_errors: false` (expected).
    pub agent_only_count: usize,
    /// Service records with no matching agent log entry.
    /// Warrants investigation — possible billing error or replay attack.
    pub service_only_count: usize,
    /// Sum of units on service_only records (unexpected charges).
    pub unit_discrepancy: f64,
}

/// Full reconciliation report.
#[derive(Debug, Clone)]
pub struct ReconciliationReport {
    pub matched: Vec<MatchedRecord>,
    pub agent_only: Vec<LoggedRequest>,
    pub service_only: Vec<UsageRecord>,
    pub summary: ReconciliationSummary,
}

/// Diff agent log against service records using `requestSignature` as the correlation key.
///
/// `requestSignature` is the HMAC the agent computed before sending, stored by Service X
/// in `UsageRecord.requestSignature`. It's the unforgeable link between both parties' logs.
pub fn reconcile(agent_log: &[LoggedRequest], service_records: &[UsageRecord]) -> ReconciliationReport {
    // Index service records by requestSignature
    let mut service_by_sig: HashMap<String, &UsageRecord> = HashMap::new();
    for record in service_records {
        if let Some(ref sig) = record.request_signature {
            service_by_sig.insert(sig.clone(), record);
        }
    }

    let mut matched = Vec::new();
    let mut agent_only = Vec::new();
    let mut matched_sigs: HashSet<String> = HashSet::new();

    for entry in agent_log {
        match &entry.signature {
            Some(sig) => {
                if let Some(&service_record) = service_by_sig.get(sig) {
                    matched_sigs.insert(sig.clone());
                    matched.push(MatchedRecord {
                        log_entry: entry.clone(),
                        service_record: service_record.clone(),
                    });
                } else {
                    agent_only.push(entry.clone());
                }
            }
            None => {
                // Unsigned request — no correlation key, can't match
                agent_only.push(entry.clone());
            }
        }
    }

    // Service records with no matching agent entry
    let mut service_only = Vec::new();
    let mut unit_discrepancy = 0.0_f64;

    for record in service_records {
        let has_match = record
            .request_signature
            .as_ref()
            .map_or(false, |sig| matched_sigs.contains(sig));

        if !has_match {
            unit_discrepancy += record.units;
            service_only.push(record.clone());
        }
    }

    let summary = ReconciliationSummary {
        matched: matched.len(),
        agent_only_count: agent_only.len(),
        service_only_count: service_only.len(),
        unit_discrepancy,
    };

    ReconciliationReport {
        matched,
        agent_only,
        service_only,
        summary,
    }
}
