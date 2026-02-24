use super::{Transport, TransportError};
use crate::types::UsageRecord;
use reqwest::Client;
use serde_json::json;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

pub struct HttpTransportOptions {
    pub url: String,
    pub headers: HashMap<String, String>,
    /// Flush after this many records. Default: 10.
    pub batch_size: usize,
    /// Flush every N milliseconds. Default: None (only flush on batch size).
    pub flush_interval_ms: Option<u64>,
    /// Max retries per batch. Default: 3.
    pub max_retries: usize,
    /// Called when a batch permanently fails (after all retries).
    pub on_error: Option<Arc<dyn Fn(TransportError, Vec<UsageRecord>) + Send + Sync>>,
}

impl Default for HttpTransportOptions {
    fn default() -> Self {
        Self {
            url: String::new(),
            headers: HashMap::new(),
            batch_size: 10,
            flush_interval_ms: None,
            max_retries: 3,
            on_error: None,
        }
    }
}

struct Inner {
    buffer: Mutex<Vec<UsageRecord>>,
    client: Client,
    url: String,
    headers: HashMap<String, String>,
    batch_size: usize,
    max_retries: usize,
    on_error: Option<Arc<dyn Fn(TransportError, Vec<UsageRecord>) + Send + Sync>>,
}

/// HTTP transport. Batches records and POSTs them to a backend with
/// exponential backoff retry. Handles 429 rate limit responses.
#[derive(Clone)]
pub struct HttpTransport {
    inner: Arc<Inner>,
}

impl HttpTransport {
    pub fn new(options: HttpTransportOptions) -> Self {
        let inner = Arc::new(Inner {
            buffer: Mutex::new(Vec::new()),
            client: Client::new(),
            url: options.url,
            headers: options.headers,
            batch_size: options.batch_size,
            max_retries: options.max_retries,
            on_error: options.on_error,
        });

        let transport = Self { inner };

        if let Some(interval_ms) = options.flush_interval_ms {
            let t = transport.clone();
            tokio::spawn(async move {
                let interval = Duration::from_millis(interval_ms);
                loop {
                    sleep(interval).await;
                    let _ = t.do_flush().await;
                }
            });
        }

        transport
    }

    async fn do_flush(&self) -> Result<(), TransportError> {
        let batch = {
            let mut buf = self.inner.buffer.lock().unwrap();
            if buf.is_empty() {
                return Ok(());
            }
            buf.drain(..).collect::<Vec<_>>()
        };

        let mut last_error: Option<TransportError> = None;

        for attempt in 0..self.inner.max_retries {
            let body = json!({ "records": &batch });

            let mut request = self
                .inner
                .client
                .post(&self.inner.url)
                .json(&body);

            for (key, value) in &self.inner.headers {
                request = request.header(key, value);
            }

            match request.send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        return Ok(());
                    }

                    if resp.status().as_u16() == 429 {
                        // Parse Retry-After header
                        let retry_after = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(60);

                        last_error = Some(TransportError::RateLimited {
                            retry_after_secs: retry_after,
                        });

                        if attempt < self.inner.max_retries - 1 {
                            sleep(Duration::from_secs(retry_after)).await;
                        }
                        continue;
                    }

                    last_error = Some(TransportError::Other(format!(
                        "HTTP {}: {}",
                        resp.status().as_u16(),
                        resp.status().canonical_reason().unwrap_or("Unknown")
                    )));
                }
                Err(e) => {
                    last_error = Some(TransportError::Http(e));
                }
            }

            // Exponential backoff: 100ms, 400ms, 900ms...
            if attempt < self.inner.max_retries - 1 {
                let delay = 100 * (attempt as u64 + 1).pow(2);
                sleep(Duration::from_millis(delay)).await;
            }
        }

        if let (Some(err), Some(on_error)) = (last_error, &self.inner.on_error) {
            on_error(err, batch);
        }

        Ok(())
    }
}

impl Transport for HttpTransport {
    fn send(
        &self,
        record: UsageRecord,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
        let should_flush = {
            let mut buf = self.inner.buffer.lock().unwrap();
            buf.push(record);
            buf.len() >= self.inner.batch_size
        };

        Box::pin(async move {
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
