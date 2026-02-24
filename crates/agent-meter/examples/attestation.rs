//! Merkle attestation: cryptographically signed batch receipts.
//!
//! AttestationTransport wraps any other transport. When a batch reaches
//! `batch_size`, it builds a Merkle tree over all records, signs the root,
//! and calls `on_attestation` with the result.
//!
//! Run with:
//!   cargo run --example attestation --manifest-path crates/agent-meter/Cargo.toml

use agent_meter::{
    transport::attestation::{verify_attestation, AttestationTransportOptions},
    AgentMeter, Attestation, AttestationTransport, IncomingRequest, MeterConfig, MemoryTransport,
};
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    const SECRET: &str = "my-signing-secret";
    const BATCH_SIZE: usize = 3;

    // Collect attested batches so we can inspect them after
    let attested: Arc<Mutex<Vec<Attestation>>> = Arc::new(Mutex::new(Vec::new()));
    let attested_clone = Arc::clone(&attested);

    // Delegate transport: also keep records in memory for cross-checking
    let delegate = Arc::new(MemoryTransport::new());

    let attestation_transport = Arc::new(AttestationTransport::new(
        AttestationTransportOptions {
            service_id: "example-api".to_string(),
            secret: SECRET.to_string(),
            batch_size: BATCH_SIZE,
            on_attestation: Arc::new(move |attestation: Attestation| {
                println!(
                    "\n✓ Batch attested: id={} records={} root={}...",
                    &attestation.batch_id[..8],
                    attestation.record_count,
                    &attestation.merkle_root[..16],
                );

                // Verify on the spot
                let valid = verify_attestation(&attestation, SECRET);
                println!("  Signature valid: {}", valid);
                assert!(valid, "attestation signature must be valid");

                attested_clone.lock().unwrap().push(attestation);
            }),
            delegate: Some(delegate),
        },
    ));

    let meter = AgentMeter::new(MeterConfig {
        service_id: "example-api".to_string(),
        transport: Some(Arc::clone(&attestation_transport) as _),
        ..Default::default()
    });

    println!("Sending 7 requests (batch size = {})...", BATCH_SIZE);
    println!("Expect 2 auto-flushes (at 3 and 6 records), 1 manual flush for the remainder.\n");

    let agents = ["alpha", "beta", "gamma"];
    for i in 0..7usize {
        let agent = agents[i % agents.len()];
        meter.record(
            IncomingRequest {
                method: Some("GET".to_string()),
                path: Some(format!("/api/item/{}", i)),
                agent_id: Some(agent.to_string()),
                status_code: Some(200),
                duration_ms: Some(10 + i as u64),
                ..Default::default()
            },
            None,
        );
    }

    // Let spawned tasks run so the auto-flushes fire
    sleep(Duration::from_millis(100)).await;

    // Flush the remaining partial batch (7 % 3 = 1 record)
    println!("Flushing remainder...");
    meter.flush().await.unwrap();

    sleep(Duration::from_millis(50)).await;

    // --- Results ---
    let batches = attested.lock().unwrap();
    println!("\n=== Attestation Summary ===");
    println!("Total batches attested: {}", batches.len());
    println!(
        "Total records covered: {}",
        batches.iter().map(|b| b.record_count).sum::<usize>()
    );

    for (i, batch) in batches.iter().enumerate() {
        println!(
            "\nBatch {}: {} records, root {}...",
            i + 1,
            batch.record_count,
            &batch.merkle_root[..16]
        );
        for r in &batch.records {
            println!("  - [{}] {}", r.agent.agent_id, r.operation);
        }

        // Demonstrate tamper detection
        if batch.record_count > 0 {
            let mut tampered = batch.clone();
            tampered.records[0].units = 9999.0;
            let still_valid = verify_attestation(&tampered, SECRET);
            println!("  Tampered record still verifies: {} (should be false)", still_valid);
            assert!(!still_valid);
        }
    }
}
