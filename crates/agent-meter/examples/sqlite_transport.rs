//! SQLite persistent transport.
//!
//! Records survive process restarts. Uses WAL mode for concurrent reads.
//! Requires the `sqlite` feature flag.
//!
//! Run with:
//!   cargo run --example sqlite_transport \
//!     --manifest-path crates/agent-meter/Cargo.toml \
//!     --features sqlite

use agent_meter::{AgentMeter, IncomingRequest, MeterConfig, QueryFilter, SqliteTransport};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    // Use ":memory:" for an in-process database, or a file path for persistence.
    let transport = Arc::new(
        SqliteTransport::new(":memory:", None).expect("failed to open database"),
    );

    let meter = AgentMeter::new(MeterConfig {
        service_id: "sqlite-example".to_string(),
        transport: Some(Arc::clone(&transport) as _),
        ..Default::default()
    });

    // Ingest some records
    let requests = [
        ("bot-1", "GET",  "/api/products", 200u16, 14u64),
        ("bot-1", "POST", "/api/orders",   201,    55),
        ("bot-2", "GET",  "/api/products", 200,    11),
        ("bot-2", "GET",  "/api/prices",   200,    9),
        ("bot-1", "GET",  "/api/products", 200,    13),
    ];

    for (agent_id, method, path, status, duration_ms) in requests {
        meter.record(
            IncomingRequest {
                method:      Some(method.to_string()),
                path:        Some(path.to_string()),
                agent_id:    Some(agent_id.to_string()),
                status_code: Some(status),
                duration_ms: Some(duration_ms),
                ..Default::default()
            },
            None,
        );
    }

    // SQLiteTransport.send() is synchronous inside the async wrapper, but
    // meter.record() spawns a task. Yield so spawns complete.
    sleep(Duration::from_millis(50)).await;

    // --- Summary ---
    let summary = transport.summary(None).unwrap();
    println!("=== Usage Summary ===");
    println!("Total records : {}", summary.total_records);
    println!("Total units   : {}", summary.total_units);
    println!("Unique agents : {}", summary.unique_agents);

    println!("\nBy agent:");
    for (id, stats) in &summary.by_agent {
        println!("  {}: {} calls, {} units", id, stats.count, stats.units);
    }

    // --- Filtered query ---
    let bot1 = transport
        .query(Some(&QueryFilter {
            agent_id: Some("bot-1".to_string()),
            ..Default::default()
        }))
        .unwrap();

    println!("\nbot-1 records ({}):", bot1.len());
    for r in &bot1 {
        println!("  {} {} → {}", r.method, r.path, r.status_code);
    }

    // --- Count ---
    let product_count = transport
        .count(Some(&QueryFilter {
            operation: Some("GET /api/products".to_string()),
            ..Default::default()
        }))
        .unwrap();
    println!("\n'GET /api/products' calls: {}", product_count);

    // --- All records ---
    let all = transport.query(None).unwrap();
    println!("\nAll {} records:", all.len());
    for r in &all {
        println!(
            "  [{}] {} {}ms id={}",
            r.agent.agent_id,
            r.operation,
            r.duration_ms,
            &r.id[..8]
        );
    }
}
