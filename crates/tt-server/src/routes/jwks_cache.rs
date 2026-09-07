//! In-process JWKS cache with TTL (default 1 hour).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

const DEFAULT_TTL: Duration = Duration::from_secs(3600);

#[derive(Clone, Default)]
pub struct JwksCache {
    inner: Arc<RwLock<HashMap<String, Entry>>>,
}

struct Entry {
    body: serde_json::Value,
    fetched_at: Instant,
    ttl: Duration,
}

impl JwksCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_or_fetch(&self, jwks_uri: &str) -> Result<serde_json::Value, String> {
        {
            let guard = self.inner.read().await;
            if let Some(e) = guard.get(jwks_uri) {
                if e.fetched_at.elapsed() < e.ttl {
                    return Ok(e.body.clone());
                }
            }
        }
        let body: serde_json::Value = reqwest::get(jwks_uri)
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;

        let mut guard = self.inner.write().await;
        guard.insert(
            jwks_uri.to_string(),
            Entry {
                body: body.clone(),
                fetched_at: Instant::now(),
                ttl: DEFAULT_TTL,
            },
        );
        Ok(body)
    }

    pub async fn invalidate(&self, jwks_uri: &str) {
        self.inner.write().await.remove(jwks_uri);
    }
}
