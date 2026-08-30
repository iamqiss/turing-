//! Records contribution events from git pushes.

use std::sync::Arc;

use tracing::info;
use tt_core::domain::{ClientKind, ContributionKind};
use tt_git::analytics::PushAnalysis;
use tt_storage::events::{new_event, ContributionStore};

/// Spawn a background task that consumes push analyses and writes events.
pub fn spawn_push_consumer(
    store: Arc<dyn ContributionStore>,
) -> tokio::sync::mpsc::UnboundedSender<PushAnalysis> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PushAnalysis>();
    tokio::spawn(async move {
        while let Some(analysis) = rx.recv().await {
            record_push(&store, analysis).await;
        }
    });
    tx
}

async fn record_push(store: &Arc<dyn ContributionStore>, analysis: PushAnalysis) {
    let commit_count = analysis.commits.len();

    let mut push = new_event(
        analysis.principal_id,
        ContributionKind::Push,
        ClientKind::GitHttp,
    );
    push.workspace_id = Some(analysis.workspace_id);
    push.repository_id = Some(analysis.repository_id);
    push.files_touched = analysis.commits.iter().map(|c| c.files_touched).sum();
    push.lines_added = analysis.commits.iter().map(|c| c.lines_added).sum();
    push.lines_deleted = analysis.commits.iter().map(|c| c.lines_deleted).sum();
    let mut langs = std::collections::BTreeSet::new();
    for c in &analysis.commits {
        for l in &c.languages {
            langs.insert(l.clone());
        }
    }
    push.languages = langs.into_iter().collect();
    push.metadata = serde_json::json!({ "commit_count": commit_count });
    if let Err(e) = store.record(&push).await {
        tracing::warn!(error = %e, "record push event failed");
    }

    for c in &analysis.commits {
        let mut ev = new_event(
            analysis.principal_id,
            ContributionKind::Commit,
            ClientKind::GitHttp,
        );
        ev.workspace_id = Some(analysis.workspace_id);
        ev.repository_id = Some(analysis.repository_id);
        ev.files_touched = c.files_touched;
        ev.lines_added = c.lines_added;
        ev.lines_deleted = c.lines_deleted;
        ev.languages = c.languages.clone();
        ev.ts = c.ts;
        ev.metadata = serde_json::json!({
            "sha": c.sha,
            "message": c.message_summary,
        });
        if let Err(e) = store.record(&ev).await {
            tracing::warn!(error = %e, sha = %c.sha, "record commit event failed");
        }
    }
    info!(
        principal = %analysis.principal_id,
        repo = %analysis.repository_id,
        commits = commit_count,
        "push contribution recorded"
    );
}
