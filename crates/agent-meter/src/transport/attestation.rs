use super::{Transport, TransportError};
use crate::signing::sign_payload;
use crate::types::{Attestation, UsageRecord};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// HMAC-SHA256 hash of a single record (leaf node for Merkle tree).
fn hash_record(record: &UsageRecord, secret: &str) -> String {
    let payload = serde_json::to_string(record).expect("UsageRecord is always serializable");
    sign_payload(&payload, secret)
}

/// SHA-256 hash of two hex strings concatenated.
fn hash_pair(left: &str, right: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(left.as_bytes());
    hasher.update(right.as_bytes());
    hex::encode(hasher.finalize())
}

/// Build a Merkle root from a slice of leaf hashes.
/// Duplicates the last leaf if the count is odd (matches TypeScript implementation).
pub fn build_merkle_root(leaves: &[String]) -> Result<String, TransportError> {
    if leaves.is_empty() {
        return Err(TransportError::Other(
            "Cannot build Merkle root from empty leaves".into(),
        ));
    }
    if leaves.len() == 1 {
        return Ok(leaves[0].clone());
    }

    let mut level = leaves.to_vec();
    while level.len() > 1 {
        let mut next = Vec::new();
        let mut i = 0;
        while i < level.len() {
            let left = &level[i];
            let right = if i + 1 < level.len() {
                &level[i + 1]
            } else {
                left // duplicate last leaf
            };
            next.push(hash_pair(left, right));
            i += 2;
        }
        level = next;
    }
    Ok(level.into_iter().next().unwrap())
}

/// Build a signed attestation over a batch of records.
pub fn build_attestation(
    records: Vec<UsageRecord>,
    service_id: &str,
    secret: &str,
) -> Result<Attestation, TransportError> {
    if records.is_empty() {
        return Err(TransportError::Other("Cannot attest empty batch".into()));
    }

    let batch_id = Uuid::new_v4().to_string();
    let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let leaves: Vec<String> = records.iter().map(|r| hash_record(r, secret)).collect();
    let merkle_root = build_merkle_root(&leaves)?;

    let sig_payload = format!("{}:{}:{}", batch_id, ts, merkle_root);
    let signature = sign_payload(&sig_payload, secret);

    Ok(Attestation {
        batch_id,
        timestamp: ts,
        service_id: service_id.to_string(),
        record_count: records.len(),
        merkle_root,
        signature,
        records,
    })
}

/// Verify a previously built attestation.
pub fn verify_attestation(attestation: &Attestation, secret: &str) -> bool {
    if attestation.records.len() != attestation.record_count {
        return false;
    }

    let leaves: Vec<String> = attestation
        .records
        .iter()
        .map(|r| hash_record(r, secret))
        .collect();

    let computed_root = match build_merkle_root(&leaves) {
        Ok(root) => root,
        Err(_) => return false,
    };

    if computed_root != attestation.merkle_root {
        return false;
    }

    let sig_payload = format!(
        "{}:{}:{}",
        attestation.batch_id, attestation.timestamp, attestation.merkle_root
    );
    let expected_sig = sign_payload(&sig_payload, secret);
    expected_sig == attestation.signature
}

pub struct AttestationTransportOptions {
    pub service_id: String,
    pub secret: String,
    /// Flush after this many records. Default: 10.
    pub batch_size: usize,
    /// Called with each completed attestation.
    pub on_attestation: Arc<dyn Fn(Attestation) + Send + Sync>,
    /// Optional delegate transport — records are also sent here.
    pub delegate: Option<Arc<dyn Transport>>,
}

/// Transport that wraps any other transport and emits Merkle-attested batches.
pub struct AttestationTransport {
    buffer: Mutex<Vec<UsageRecord>>,
    service_id: String,
    secret: String,
    batch_size: usize,
    on_attestation: Arc<dyn Fn(Attestation) + Send + Sync>,
    delegate: Option<Arc<dyn Transport>>,
}

impl AttestationTransport {
    pub fn new(options: AttestationTransportOptions) -> Self {
        Self {
            buffer: Mutex::new(Vec::new()),
            service_id: options.service_id,
            secret: options.secret,
            batch_size: options.batch_size.max(1),
            on_attestation: options.on_attestation,
            delegate: options.delegate,
        }
    }

    async fn do_flush(&self) -> Result<(), TransportError> {
        let batch = {
            let mut buf = self.buffer.lock().unwrap();
            if buf.is_empty() {
                return Ok(());
            }
            buf.drain(..).collect::<Vec<_>>()
        };

        let attestation = build_attestation(batch, &self.service_id, &self.secret)?;
        (self.on_attestation)(attestation);

        if let Some(delegate) = &self.delegate {
            delegate.flush().await?;
        }

        Ok(())
    }
}

impl Transport for AttestationTransport {
    fn send(
        &self,
        record: UsageRecord,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
        let should_flush = {
            let mut buf = self.buffer.lock().unwrap();
            buf.push(record.clone());
            buf.len() >= self.batch_size
        };

        Box::pin(async move {
            if let Some(delegate) = &self.delegate {
                delegate.send(record).await?;
            }
            if should_flush {
                self.do_flush().await?;
            }
            Ok(())
        })
    }

    fn flush(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
        Box::pin(async move { self.do_flush().await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentIdentity, PricingModel};

    fn make_record(id: &str) -> UsageRecord {
        UsageRecord {
            id: id.to_string(),
            timestamp: "2026-02-24T00:00:00.000Z".to_string(),
            service_id: "test".to_string(),
            agent: AgentIdentity {
                agent_id: "bot-1".to_string(),
                name: None,
                shepherd_id: None,
                tier: None,
            },
            operation: "GET /test".to_string(),
            units: 1.0,
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

    #[test]
    fn build_and_verify() {
        let records = vec![make_record("r1"), make_record("r2"), make_record("r3")];
        let attestation = build_attestation(records, "my-service", "secret").unwrap();
        assert!(verify_attestation(&attestation, "secret"));
        assert!(!verify_attestation(&attestation, "wrong-secret"));
    }

    #[test]
    fn merkle_root_single_leaf() {
        let leaves = vec!["abc123".to_string()];
        let root = build_merkle_root(&leaves).unwrap();
        assert_eq!(root, "abc123");
    }

    #[test]
    fn tampered_record_fails_verification() {
        let records = vec![make_record("r1"), make_record("r2")];
        let mut attestation = build_attestation(records, "my-service", "secret").unwrap();
        attestation.records[0].units = 999.0; // tamper
        assert!(!verify_attestation(&attestation, "secret"));
    }
}
