//! Bulk reindex from Postgres into Meilisearch.

use serde_json::json;
use sqlx::PgPool;
use tracing::{info, warn};

use crate::client::MeiliClient;
use crate::documents::{IDX_ISSUES, IDX_MESSAGES, IDX_PEOPLE, IDX_PRS, IDX_REPOS};
use crate::error::Result;
use crate::service::SearchService;

#[derive(Debug, Default, serde::Serialize)]
pub struct ReindexReport {
    pub repos: u64,
    pub issues: u64,
    pub pull_requests: u64,
    pub people: u64,
    pub messages: u64,
    pub errors: Vec<String>,
}

impl SearchService {
    /// Full reindex from OLTP. Safe to run while server is up (Meili upserts by id).
    pub async fn reindex_from_postgres(&self, pool: &PgPool) -> Result<ReindexReport> {
        if !self.is_enabled() {
            return Err(crate::error::SearchError::Disabled);
        }
        self.bootstrap_indexes().await;
        let mut report = ReindexReport::default();
        let client = self.client();

        // --- repos ---
        match sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, Option<String>, bool)>(
            r#"SELECT id, workspace_id, name, description, is_private FROM repositories"#,
        )
        .fetch_all(pool)
        .await
        {
            Ok(rows) => {
                let docs: Vec<_> = rows
                    .into_iter()
                    .map(|(id, ws, name, desc, priv_)| {
                        json!({
                            "id": id.to_string(),
                            "workspace_id": ws.to_string(),
                            "name": name,
                            "description": desc.unwrap_or_default(),
                            "is_private": priv_,
                            "kind": "repository",
                        })
                    })
                    .collect();
                report.repos = docs.len() as u64;
                for chunk in docs.chunks(200) {
                    if let Err(e) = client.add_documents(IDX_REPOS, chunk).await {
                        report.errors.push(format!("repos: {e}"));
                    }
                }
            }
            Err(e) => report.errors.push(format!("repos query: {e}")),
        }

        // --- issues ---
        match sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid, i32, String, Option<String>, String)>(
            r#"
            SELECT i.id, i.repository_id, r.workspace_id, i.number, i.title, i.body, i.state
            FROM issues i
            JOIN repositories r ON r.id = i.repository_id
            "#,
        )
        .fetch_all(pool)
        .await
        {
            Ok(rows) => {
                let docs: Vec<_> = rows
                    .into_iter()
                    .map(|(id, rid, ws, num, title, body, state)| {
                        json!({
                            "id": id.to_string(),
                            "repository_id": rid.to_string(),
                            "workspace_id": ws.to_string(),
                            "number": num,
                            "title": title,
                            "body": body.unwrap_or_default(),
                            "state": state,
                            "kind": "issue",
                        })
                    })
                    .collect();
                report.issues = docs.len() as u64;
                for chunk in docs.chunks(200) {
                    if let Err(e) = client.add_documents(IDX_ISSUES, chunk).await {
                        report.errors.push(format!("issues: {e}"));
                    }
                }
            }
            Err(e) => report.errors.push(format!("issues query: {e}")),
        }

        // --- PRs ---
        match sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid, i32, String, Option<String>, String, String, String)>(
            r#"
            SELECT p.id, p.repository_id, r.workspace_id, p.number, p.title, p.body, p.status,
                   p.source_branch, p.target_branch
            FROM pull_requests p
            JOIN repositories r ON r.id = p.repository_id
            "#,
        )
        .fetch_all(pool)
        .await
        {
            Ok(rows) => {
                let docs: Vec<_> = rows
                    .into_iter()
                    .map(|(id, rid, ws, num, title, body, status, src, tgt)| {
                        json!({
                            "id": id.to_string(),
                            "repository_id": rid.to_string(),
                            "workspace_id": ws.to_string(),
                            "number": num,
                            "title": title,
                            "body": body.unwrap_or_default(),
                            "state": status,
                            "source_branch": src,
                            "target_branch": tgt,
                            "kind": "pull_request",
                        })
                    })
                    .collect();
                report.pull_requests = docs.len() as u64;
                for chunk in docs.chunks(200) {
                    if let Err(e) = client.add_documents(IDX_PRS, chunk).await {
                        report.errors.push(format!("prs: {e}"));
                    }
                }
            }
            Err(e) => report.errors.push(format!("prs query: {e}")),
        }

        // --- people ---
        match sqlx::query_as::<_, (uuid::Uuid, String, String, Option<String>)>(
            r#"
            SELECT p.id, p.handle, p.display_name, pr.bio
            FROM principals p
            LEFT JOIN profiles pr ON pr.principal_id = p.id
            "#,
        )
        .fetch_all(pool)
        .await
        {
            Ok(rows) => {
                let docs: Vec<_> = rows
                    .into_iter()
                    .map(|(id, handle, name, bio)| {
                        json!({
                            "id": id.to_string(),
                            "handle": handle,
                            "display_name": name,
                            "bio": bio.unwrap_or_default(),
                            "kind": "person",
                        })
                    })
                    .collect();
                report.people = docs.len() as u64;
                for chunk in docs.chunks(200) {
                    if let Err(e) = client.add_documents(IDX_PEOPLE, chunk).await {
                        report.errors.push(format!("people: {e}"));
                    }
                }
            }
            Err(e) => report.errors.push(format!("people query: {e}")),
        }

        // --- messages (non-deleted) ---
        match sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid, String, uuid::Uuid)>(
            r#"
            SELECT m.id, m.channel_id, c.workspace_id, m.content, m.author_id
            FROM messages m
            JOIN channels c ON c.id = m.channel_id
            WHERE m.is_deleted = FALSE
            "#,
        )
        .fetch_all(pool)
        .await
        {
            Ok(rows) => {
                let docs: Vec<_> = rows
                    .into_iter()
                    .map(|(id, ch, ws, body, author)| {
                        json!({
                            "id": id.to_string(),
                            "channel_id": ch.to_string(),
                            "workspace_id": ws.to_string(),
                            "body": body,
                            "author_id": author.to_string(),
                            "kind": "message",
                        })
                    })
                    .collect();
                report.messages = docs.len() as u64;
                for chunk in docs.chunks(200) {
                    if let Err(e) = client.add_documents(IDX_MESSAGES, chunk).await {
                        report.errors.push(format!("messages: {e}"));
                    }
                }
            }
            Err(e) => report.errors.push(format!("messages query: {e}")),
        }

        info!(
            repos = report.repos,
            issues = report.issues,
            prs = report.pull_requests,
            people = report.people,
            messages = report.messages,
            errors = report.errors.len(),
            "reindex complete"
        );
        if !report.errors.is_empty() {
            for e in &report.errors {
                warn!(error = %e, "reindex partial error");
            }
        }
        Ok(report)
    }
}

// silence unused import if client type referenced
#[allow(dead_code)]
fn _c(_: &MeiliClient) {}
