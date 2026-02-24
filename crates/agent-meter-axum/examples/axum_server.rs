//! Full axum server with agent-meter middleware.
//!
//! Demonstrates:
//!  - Global metering layer on all routes
//!  - Per-route options (custom operation name, unit type, skip)
//!  - /usage endpoint that reads from MemoryTransport
//!  - /signing endpoint that requires HMAC-signed requests
//!
//! Run with:
//!   cargo run --example axum_server --manifest-path crates/agent-meter-axum/Cargo.toml
//!
//! Then try:
//!   # Identified agent request
//!   curl -H "X-Agent-Id: bot-1" -H "X-Agent-Name: DemoBot" \
//!        http://localhost:3000/api/widgets
//!
//!   # Unidentified request (not metered)
//!   curl http://localhost:3000/api/widgets
//!
//!   # Health check (skipped)
//!   curl -H "X-Agent-Id: bot-1" http://localhost:3000/health
//!
//!   # View usage summary
//!   curl http://localhost:3000/usage

use agent_meter::{AgentMeter, MeterConfig, MemoryTransport, PricingModel, RouteOptions};
use agent_meter_axum::AgentMeterLayer;
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;

#[derive(Clone)]
struct AppState {
    transport: Arc<MemoryTransport>,
}

async fn get_widgets() -> Json<serde_json::Value> {
    Json(json!({ "widgets": ["sprocket", "cog", "lever"] }))
}

async fn post_order(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    Json(json!({ "orderId": "ord-001", "items": body.get("items") }))
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

async fn usage_summary(State(state): State<AppState>) -> Json<serde_json::Value> {
    let summary = state.transport.summary(None);
    Json(json!({
        "totalRecords": summary.total_records,
        "totalUnits":   summary.total_units,
        "uniqueAgents": summary.unique_agents,
        "byAgent":      summary.by_agent,
        "byOperation":  summary.by_operation,
    }))
}

#[tokio::main]
async fn main() {
    let transport = Arc::new(MemoryTransport::new());

    let meter = AgentMeter::new(MeterConfig {
        service_id: "demo-api".to_string(),
        transport: Some(Arc::clone(&transport) as _),
        // Drop health check records before they're emitted
        before_emit: Some(Box::new(|record| {
            if record.path == "/health" {
                None
            } else {
                Some(record)
            }
        })),
        ..Default::default()
    });

    // Per-route layer: orders are metered as "per-unit" with custom operation name
    let order_layer = AgentMeterLayer::new(meter.clone()).with_options(RouteOptions {
        operation: Some("place-order".to_string()),
        unit_type: Some("order".to_string()),
        pricing: Some(PricingModel::PerUnit),
        ..Default::default()
    });

    // Health check: skip metering entirely
    let health_layer = AgentMeterLayer::new(meter.clone()).with_options(RouteOptions {
        skip: true,
        ..Default::default()
    });

    let state = AppState { transport };

    let app: Router = Router::new()
        // Per-route layers first (applied innermost → outermost)
        .route("/api/orders", post(post_order).layer(order_layer))
        .route("/health", get(health).layer(health_layer))
        // Global layer covers everything else
        .route("/api/widgets", get(get_widgets))
        .route("/usage", get(usage_summary))
        .layer(AgentMeterLayer::new(meter))
        .with_state(state);

    let addr = "0.0.0.0:3000";
    println!("agent-meter demo server listening on {}", addr);
    println!("Try: curl -H 'X-Agent-Id: bot-1' http://localhost:3000/api/widgets");
    println!("     curl http://localhost:3000/usage");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
