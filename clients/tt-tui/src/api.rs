//! Thin HTTP client against tt-server REST surfaces.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Repo {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub default_branch: String,
    pub is_private: bool,
    pub clone_url_path: String,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    pub base: String,
    pub token: Option<String>,
}

impl Client {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: base.into().trim_end_matches('/').to_string(),
            token: None,
        }
    }

    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token;
        self
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => req.header("Authorization", format!("Bearer {t}")),
            None => req,
        }
    }

    async fn read_error(res: reqwest::Response) -> String {
        res.text().await.unwrap_or_default()
    }

    pub async fn health(&self) -> Result<String> {
        let url = format!("{}/healthz", self.base);
        let res = self.http.get(&url).send().await?;
        let status = res.status();
        Ok(format!("{} {}", status, res.text().await.unwrap_or_default()))
    }

    pub async fn login(&self, handle: &str, passphrase: &str) -> Result<LoginResponse> {
        let url = format!("{}/api/auth/login", self.base);
        let body = serde_json::json!({ "handle": handle, "passphrase": passphrase });
        let res = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("login request")?;
        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!("login failed: {}", Self::read_error(res).await));
        }
        res.json().await.context("login json")
    }

    pub async fn register(
        &self,
        handle: &str,
        display_name: &str,
        passphrase: &str,
    ) -> Result<LoginResponse> {
        let url = format!("{}/api/auth/register", self.base);
        let body = serde_json::json!({
            "handle": handle,
            "display_name": display_name,
            "passphrase": passphrase,
        });
        let res = self.http.post(&url).json(&body).send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!("register failed: {}", Self::read_error(res).await));
        }
        res.json().await.context("register json")
    }

    pub async fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        let url = format!("{}/api/workspaces", self.base);
        let res = self.auth(self.http.get(&url)).send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!("{status}: {}", Self::read_error(res).await));
        }
        Ok(res.json().await?)
    }

    pub async fn create_workspace(
        &self,
        name: &str,
        slug: &str,
        description: Option<&str>,
    ) -> Result<Workspace> {
        let url = format!("{}/api/workspaces", self.base);
        let body = serde_json::json!({
            "name": name,
            "slug": slug,
            "description": description,
        });
        let res = self.auth(self.http.post(&url).json(&body)).send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!("{status}: {}", Self::read_error(res).await));
        }
        Ok(res.json().await?)
    }

    pub async fn list_repos(&self, workspace_id: &str) -> Result<Vec<Repo>> {
        let url = format!("{}/api/workspaces/{workspace_id}/repos", self.base);
        let res = self.auth(self.http.get(&url)).send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!("{status}: {}", Self::read_error(res).await));
        }
        Ok(res.json().await?)
    }

    pub async fn create_repo(
        &self,
        workspace_id: &str,
        name: &str,
        description: Option<&str>,
        is_private: bool,
    ) -> Result<Repo> {
        let url = format!("{}/api/workspaces/{workspace_id}/repos", self.base);
        let body = serde_json::json!({
            "name": name,
            "description": description,
            "is_private": is_private,
            "default_branch": "main",
        });
        let res = self.auth(self.http.post(&url).json(&body)).send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!("{status}: {}", Self::read_error(res).await));
        }
        Ok(res.json().await?)
    }

    pub async fn delete_repo(&self, workspace_id: &str, name: &str) -> Result<()> {
        let url = format!("{}/api/workspaces/{workspace_id}/repos/{name}", self.base);
        let res = self.auth(self.http.delete(&url)).send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!("{status}: {}", Self::read_error(res).await));
        }
        Ok(())
    }

    pub async fn get_json(&self, path: &str) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        let res = self.auth(self.http.get(&url)).send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!("{status}: {}", Self::read_error(res).await));
        }
        Ok(res.json().await?)
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        let res = self.auth(self.http.post(&url).json(body)).send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!("{status}: {}", Self::read_error(res).await));
        }
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        let text = res.text().await?;
        if text.is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&text)?)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LoginResponse {
    pub principal_id: String,
    pub handle: String,
    pub display_name: String,
    pub session_token: String,
}
