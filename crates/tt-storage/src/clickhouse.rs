//! Minimal ClickHouse HTTP client (no heavy SDK).

use tracing::{debug, warn};

use crate::error::{Result, StorageError};

#[derive(Clone)]
pub struct ClickHouseClient {
    http: reqwest::Client,
    base_url: String,
    database: String,
    enabled: bool,
}

impl ClickHouseClient {
    pub fn new(base_url: impl Into<String>, database: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            database: database.into(),
            enabled: true,
        }
    }

    pub fn disabled() -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: String::new(),
            database: String::new(),
            enabled: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub async fn ping(&self) -> Result<()> {
        if !self.enabled {
            return Err(StorageError::ClickHouse("disabled".into()));
        }
        let url = format!("{}/ping", self.base_url);
        let res = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| StorageError::ClickHouse(e.to_string()))?;
        if res.status().is_success() {
            Ok(())
        } else {
            Err(StorageError::ClickHouse(format!("ping {}", res.status())))
        }
    }

    pub async fn execute(&self, sql: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let url = format!("{}/?database={}", self.base_url, self.database);
        let res = self
            .http
            .post(&url)
            .body(sql.to_string())
            .send()
            .await
            .map_err(|e| StorageError::ClickHouse(e.to_string()))?;
        if res.status().is_success() {
            Ok(())
        } else {
            let body = res.text().await.unwrap_or_default();
            Err(StorageError::ClickHouse(body))
        }
    }

    /// Apply init SQL files (split on semicolons carefully — we run whole file statements).
    pub async fn ensure_schema(&self, sql: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        // ClickHouse accepts multi-statement via HTTP with wait_end_of_query
        for stmt in split_sql(sql) {
            if stmt.is_empty() {
                continue;
            }
            debug!(stmt = %stmt.chars().take(80).collect::<String>(), "clickhouse exec");
            if let Err(e) = self.execute(&stmt).await {
                warn!(error = %e, "clickhouse schema statement failed");
                return Err(e);
            }
        }
        Ok(())
    }

    pub async fn insert_json_each_row(&self, table: &str, rows: &[serde_json::Value]) -> Result<()> {
        if !self.enabled || rows.is_empty() {
            return Ok(());
        }
        let body = rows
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let url = format!(
            "{}/?database={}&query=INSERT%20INTO%20{}%20FORMAT%20JSONEachRow",
            self.base_url, self.database, table
        );
        let res = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| StorageError::ClickHouse(e.to_string()))?;
        if res.status().is_success() {
            Ok(())
        } else {
            let body = res.text().await.unwrap_or_default();
            Err(StorageError::ClickHouse(body))
        }
    }
}

fn split_sql(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim_start().starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}
