//! # agent-meter-axum
//!
//! Tower/axum middleware layer for agent-meter.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use agent_meter::{AgentMeter, MeterConfig, MemoryTransport};
//! use agent_meter_axum::AgentMeterLayer;
//! use axum::{Router, routing::get};
//! use std::sync::Arc;
//!
//! let meter = AgentMeter::new(MeterConfig {
//!     service_id: "my-api".to_string(),
//!     transport: Some(Arc::new(MemoryTransport::new())),
//!     ..Default::default()
//! });
//!
//! let app: Router = Router::new()
//!     .route("/api/widgets", get(|| async { "[]" }))
//!     .layer(AgentMeterLayer::new(meter));
//! ```

use agent_meter::{AgentMeter, IncomingRequest, RouteOptions};
use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use futures_util::future::BoxFuture;
use http::HeaderMap;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tower::{Layer, Service};

/// Tower layer that wraps a service with agent-meter metering.
#[derive(Clone)]
pub struct AgentMeterLayer {
    meter: AgentMeter,
    options: Option<Arc<RouteOptions>>,
}

impl AgentMeterLayer {
    pub fn new(meter: AgentMeter) -> Self {
        Self {
            meter,
            options: None,
        }
    }

    /// Attach per-route options (operation name, unit count, pricing, etc.)
    pub fn with_options(mut self, options: RouteOptions) -> Self {
        self.options = Some(Arc::new(options));
        self
    }
}

impl<S> Layer<S> for AgentMeterLayer {
    type Service = AgentMeterService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AgentMeterService {
            inner,
            meter: self.meter.clone(),
            options: self.options.clone(),
        }
    }
}

/// Tower service that meters each request.
#[derive(Clone)]
pub struct AgentMeterService<S> {
    inner: S,
    meter: AgentMeter,
    options: Option<Arc<RouteOptions>>,
}

impl<S> Service<Request<Body>> for AgentMeterService<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let start = Instant::now();
        let meter = self.meter.clone();
        let options = self.options.clone();

        // Extract request metadata before consuming the request
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
        let headers = req.headers().clone();

        let agent_id = header_str(&headers, "x-agent-id");
        let agent_name = header_str(&headers, "x-agent-name");
        let request_signature = header_str(&headers, "x-agent-signature");

        let mut inner = self.inner.clone();
        Box::pin(async move {
            let response = inner.call(req).await?;
            let status_code = response.status().as_u16();
            let duration_ms = start.elapsed().as_millis() as u64;

            let incoming = IncomingRequest {
                method: Some(method),
                path: Some(path),
                agent_id,
                agent_name,
                request_signature,
                body: None, // body is consumed; sign body before middleware if needed
                status_code: Some(status_code),
                duration_ms: Some(duration_ms),
                headers: headers
                    .iter()
                    .filter_map(|(k, v)| {
                        v.to_str().ok().map(|s| (k.to_string(), s.to_string()))
                    })
                    .collect(),
            };

            meter.record(incoming, options.as_ref().map(|o| (**o).clone()));

            Ok(response)
        })
    }
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}
