//! End-to-end demo: signing, metering, receipts, and reconciliation.
//!
//! Starts an in-process axum server with agent-meter middleware, runs an agent
//! making 25 requests (mix of success and 4xx), then reconciles.
//!
//! Run:
//!   cargo run --example e2e --manifest-path crates/agent-meter-client/Cargo.toml

use agent_meter::{AgentMeter, MeterConfig, MemoryTransport, QueryFilter, UsageRecord};
use agent_meter_axum::AgentMeterLayer;
use agent_meter_client::{AgentClient, ClientConfig};
use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

// ── Server setup ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    transport: Arc<MemoryTransport>,
}

async fn generate_handler() -> Json<serde_json::Value> {
    // Simulate a short computation
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    Json(serde_json::json!({"result": "text generated"}))
}

#[derive(Deserialize)]
struct UsageQuery {
    agent_id: Option<String>,
}

async fn usage_handler(
    State(state): State<AppState>,
    Query(params): Query<UsageQuery>,
) -> Json<Vec<UsageRecord>> {
    let filter = params.agent_id.map(|id| QueryFilter {
        agent_id: Some(id),
        ..Default::default()
    });
    Json(state.transport.query(filter.as_ref()))
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let transport = Arc::new(MemoryTransport::new());

    let meter = AgentMeter::new(MeterConfig {
        service_id: "demo-api".to_string(),
        transport: Some(Arc::clone(&transport) as _),
        // Don't meter 4xx/5xx (default). Makes agent_only meaningful in reconciliation.
        meter_errors: false,
        ..Default::default()
    });

    let state = AppState {
        transport: Arc::clone(&transport),
    };

    // Apply AgentMeterLayer per-route so only /api/generate is metered.
    // Requests to unknown routes get 404 from axum's default handler — the
    // metering middleware never runs, so no receipt is issued for them.
    let meter_layer = AgentMeterLayer::new(meter).with_receipt_secret("svc-receipt-secret");
    let app = Router::new()
        .route("/api/generate", post(generate_handler).layer(meter_layer))
        .route("/v1/usage/me", get(usage_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    println!("Service X running at http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // ── Agent Y ───────────────────────────────────────────────────────────────

    let client = AgentClient::new(ClientConfig {
        agent_id: "agent-y".to_string(),
        agent_name: Some("DemoAgent".to_string()),
        signing_secret: Some("shared-secret".to_string()),
        service_url: format!("http://{}", addr),
    });

    println!("\nAgent Y making 25 requests (20 success, 5 to a missing route)...\n");

    let mut receipts_received = 0;
    for i in 0..25 {
        let body = serde_json::json!({"prompt": format!("request {}", i + 1)}).to_string();

        // Requests 21-25 go to a non-existent route → 404, not metered
        let path = if i < 20 {
            "/api/generate"
        } else {
            "/api/unknown"
        };

        let resp = client.call("POST", path, Some(&body)).await.unwrap();
        if resp.receipt.is_some() {
            receipts_received += 1;
        }
        print!(
            "  [{:02}] {} {}",
            i + 1,
            resp.status_code,
            path
        );
        if resp.receipt.is_some() {
            print!("  ✓ receipt");
        }
        println!();
    }

    println!("\nReceipts received: {}/20 (issued only for metered routes)", receipts_received);

    // ── Reconciliation ────────────────────────────────────────────────────────

    println!("\nReconciling agent log against service records...");
    let report = client.reconcile().await.unwrap();

    println!("\n╔══════════════════════════════╗");
    println!("║   Reconciliation Report      ║");
    println!("╠══════════════════════════════╣");
    println!("║  matched:          {:>8}  ║", report.summary.matched);
    println!("║  agent_only:       {:>8}  ║", report.summary.agent_only_count);
    println!("║  service_only:     {:>8}  ║", report.summary.service_only_count);
    println!("║  unit_discrepancy: {:>8.2}  ║", report.summary.unit_discrepancy);
    println!("╚══════════════════════════════╝");

    println!("\nAgent-only breakdown (not metered by service):");
    for entry in &report.agent_only {
        println!(
            "  {} {} → status {:?}",
            entry.method,
            entry.url,
            entry.status_code
        );
    }

    if report.summary.service_only_count == 0 {
        println!("\nservice_only: 0 — clean. No unexpected charges.");
    } else {
        println!("\nWARNING: {} service-only records (investigate!)", report.summary.service_only_count);
    }
}
