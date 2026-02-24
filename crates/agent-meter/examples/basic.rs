//! Basic usage: MemoryTransport + AgentMeter
//!
//! Run with:
//!   cargo run --example basic --manifest-path crates/agent-meter/Cargo.toml

use agent_meter::{AgentMeter, IncomingRequest, MeterConfig, MemoryTransport, QueryFilter};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    // Keep a typed handle to the transport so we can query it later.
    // The meter also holds an Arc<dyn Transport> pointing at the same allocation.
    let transport = Arc::new(MemoryTransport::new());

    let meter = AgentMeter::new(MeterConfig {
        service_id: "example-api".to_string(),
        transport: Some(Arc::clone(&transport) as _),
        ..Default::default()
    });

    // Simulate five requests from two agents
    let requests = vec![
        ("GET",  "/api/widgets",  "bot-1", "WidgetBot", 200u16, 11u64),
        ("POST", "/api/orders",   "bot-1", "WidgetBot", 201,    23),
        ("GET",  "/api/widgets",  "bot-2", "PriceBot",  200,    8),
        ("GET",  "/api/catalog",  "bot-2", "PriceBot",  200,    14),
        ("GET",  "/api/widgets",  "bot-1", "WidgetBot", 200,    9),
    ];

    for (method, path, agent_id, agent_name, status, duration_ms) in requests {
        meter.record(
            IncomingRequest {
                method: Some(method.to_string()),
                path: Some(path.to_string()),
                agent_id: Some(agent_id.to_string()),
                agent_name: Some(agent_name.to_string()),
                status_code: Some(status),
                duration_ms: Some(duration_ms),
                ..Default::default()
            },
            None,
        );
    }

    // meter.record() is fire-and-forget — it spawns a task to send to the transport.
    // Yield briefly so the spawned tasks complete before we read results.
    sleep(Duration::from_millis(50)).await;

    // --- Summary ---
    let summary = transport.summary(None);
    println!("=== Usage Summary ===");
    println!("Total records : {}", summary.total_records);
    println!("Total units   : {}", summary.total_units);
    println!("Unique agents : {}", summary.unique_agents);

    println!("\nBy agent:");
    for (agent_id, stats) in &summary.by_agent {
        println!("  {}: {} calls, {} units", agent_id, stats.count, stats.units);
    }

    println!("\nBy operation:");
    for (op, stats) in &summary.by_operation {
        println!("  {}: {} calls", op, stats.count);
    }

    // --- Filtered query ---
    let bot1_records = transport.query(Some(&QueryFilter {
        agent_id: Some("bot-1".to_string()),
        ..Default::default()
    }));
    println!("\nbot-1 records ({} total):", bot1_records.len());
    for r in &bot1_records {
        println!("  {} {} → {} ({}ms)", r.method, r.path, r.status_code, r.duration_ms);
    }

    // --- Raw records ---
    let all = transport.records();
    println!("\nAll {} records:", all.len());
    for r in &all {
        println!("  [{}] {} {}", r.agent.agent_id, r.operation, r.timestamp);
    }
}
