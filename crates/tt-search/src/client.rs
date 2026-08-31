//! Minimal Meilisearch HTTP client (no SDK lock-in).

use reqwest::Client;
use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::error::{Result, SearchError};

#[derive(Clone)]
pub struct MeiliClient {
    http: Client,
    base: String,
    key: String,
    enabled: bool,
}

impl MeiliClient {
    pub fn new(base_url: impl Into<String>, master_key: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            base: base_url.into().trim_end_matches('/').to_string(),
            key: master_key.into(),
            enabled: true,
        }
    }

    pub fn disabled() -> Self {
        Self {
            http: Client::new(),
            base: String::new(),
            key: String::new(),
            enabled: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        if !self.enabled {
            return Err(SearchError::Disabled);
        }
        let url = format!("{}{}", self.base, path);
        let mut req = self
            .http
            .request(method, &url)
            .header("Authorization", format!("Bearer {}", self.key));
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").json(&b);
        }
        let res = req
            .send()
            .await
            .map_err(|e| SearchError::Unavailable(e.to_string()))?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(SearchError::Meili(format!("{status}: {text}")));
        }
        if text.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| SearchError::Meili(e.to_string()))
    }

    pub async fn health(&self) -> Result<()> {
        if !self.enabled {
            return Err(SearchError::Disabled);
        }
        let url = format!("{}/health", self.base);
        let res = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| SearchError::Unavailable(e.to_string()))?;
        if res.status().is_success() {
            Ok(())
        } else {
            Err(SearchError::Unavailable(format!("health {}", res.status())))
        }
    }

    pub async fn ensure_index(&self, uid: &str, primary_key: &str) -> Result<()> {
        match self
            .request(
                reqwest::Method::POST,
                "/indexes",
                Some(json!({ "uid": uid, "primaryKey": primary_key })),
            )
            .await
        {
            Ok(_) => {
                debug!(%uid, "index created");
                Ok(())
            }
            Err(SearchError::Meili(msg)) if msg.contains("index_already_exists") || msg.contains("already exists") => {
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub async fn update_settings(&self, uid: &str, settings: Value) -> Result<()> {
        self.request(
            reqwest::Method::PATCH,
            &format!("/indexes/{uid}/settings"),
            Some(settings),
        )
        .await
        .map(|_| ())
    }

    pub async fn add_documents(&self, uid: &str, docs: &[Value]) -> Result<()> {
        if docs.is_empty() {
            return Ok(());
        }
        self.request(
            reqwest::Method::POST,
            &format!("/indexes/{uid}/documents"),
            Some(Value::Array(docs.to_vec())),
        )
        .await
        .map(|_| ())
    }

    pub async fn delete_document(&self, uid: &str, id: &str) -> Result<()> {
        match self
            .request(
                reqwest::Method::DELETE,
                &format!("/indexes/{uid}/documents/{id}"),
                None,
            )
            .await
        {
            Ok(_) | Err(SearchError::Meili(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn search(
        &self,
        uid: &str,
        q: &str,
        filter: Option<&str>,
        limit: u32,
    ) -> Result<Value> {
        let mut body = json!({
            "q": q,
            "limit": limit,
        });
        if let Some(f) = filter {
            body["filter"] = json!(f);
        }
        self.request(
            reqwest::Method::POST,
            &format!("/indexes/{uid}/search"),
            Some(body),
        )
        .await
    }

    pub async fn multi_search(&self, queries: Value) -> Result<Value> {
        self.request(
            reqwest::Method::POST,
            "/multi-search",
            Some(queries),
        )
        .await
    }

    /// Best-effort ensure; logs and continues if Meili is down.
    pub async fn soft_ensure(&self, uid: &str, primary_key: &str, settings: Value) {
        if !self.enabled {
            return;
        }
        if let Err(e) = self.ensure_index(uid, primary_key).await {
            warn!(error = %e, %uid, "ensure index");
            return;
        }
        if let Err(e) = self.update_settings(uid, settings).await {
            warn!(error = %e, %uid, "update settings");
        }
    }
}
