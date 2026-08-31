//! OpenAI-compatible chat completions client for OLMo (vLLM / TGI / llama.cpp server).

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::debug;

use crate::error::{AgentError, Result};

#[derive(Clone)]
pub struct InferenceClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl InferenceClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, api_key: Option<String>) -> Self {
        let base = base_url.into().trim_end_matches('/').to_string();
        Self {
            http: Client::new(),
            base_url: base,
            api_key,
            model: model.into(),
            enabled: true,
        }
    }

    pub fn disabled() -> Self {
        Self {
            http: Client::new(),
            base_url: String::new(),
            api_key: None,
            model: "disabled".into(),
            enabled: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn chat(&self, messages: &[ChatMessage], tools_json: &Value) -> Result<String> {
        if !self.enabled {
            return Err(AgentError::Model("inference disabled".into()));
        }
        let url = format!("{}/v1/chat/completions", self.base_url);
        let body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": 0.2,
            "max_tokens": 1024,
            "tools": tools_json,
            "tool_choice": "auto",
        });
        let mut req = self.http.post(&url).json(&body);
        if let Some(ref k) = self.api_key {
            req = req.header("Authorization", format!("Bearer {k}"));
        }
        let res = req
            .send()
            .await
            .map_err(|e| AgentError::Model(format!("unreachable: {e}")))?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(AgentError::Model(format!("{status}: {text}")));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| AgentError::Model(format!("bad json: {e}")))?;
        debug!(raw_len = text.len(), "inference response");
        if let Some(calls) = v
            .pointer("/choices/0/message/tool_calls")
            .and_then(|c| c.as_array())
        {
            if let Some(first) = calls.first() {
                let name = first
                    .pointer("/function/name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                let args = first
                    .pointer("/function/arguments")
                    .and_then(|a| a.as_str())
                    .unwrap_or("{}");
                return Ok(format!("{{\"tool\":\"{name}\",\"arguments\":{args}}}"));
            }
        }
        let content = v
            .pointer("/choices/0/message/content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        Ok(content)
    }
}
