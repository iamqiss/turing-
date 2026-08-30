//! Application bootstrap: config, pools, services.

use std::path::PathBuf;
use std::sync::Arc;

use sqlx::postgres::PgPoolOptions;
use tracing::{info, warn};
use tt_auth::{
    AgentTokenService, CeremonyStore, InMemoryCeremonyStore, PostgresAuthStore, PrincipalResolver,
    RedisCeremonyStore, SessionService, WebauthnService,
};
use tt_git::{IssueService, PostgresIssueStore, PostgresLabelStore, PostgresPullRequestStore, PostgresReviewStore, ReviewService, PostgresRepoStore, PostgresSettingsStore, PullRequestService, RepoService};
use tt_messaging::{MessagingService, OrgService, PostgresMessageStore, PostgresOrgStore, PostgresWorkspaceStore, WorkspaceService};
use tt_realtime::InMemoryRealtime;
use tt_config::Config;
use tt_storage::{ClickHouseClient, PostgresContributionStore, PostgresProfileStore, ProfileService};
use tt_compute::{ComputeService, HttpProvider, PostgresComputeStore};
use tt_net::PostgresIpAllocator;

use crate::state::AppState;

pub async fn build_state(config: Config) -> anyhow::Result<AppState> {
    let config = Arc::new(config);

    // --- Postgres ---
    info!(url = %mask_url(&config.database_url), "connecting to postgres");
    let db = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&config.database_url)
        .await?;

    // Run migrations if the migrations folder is present
    run_migrations(&db).await?;

    // --- Auth store ---
    let auth_store: Arc<dyn tt_auth::AuthStore> = Arc::new(PostgresAuthStore::new(db.clone()));

    // --- Session key ---
    let session_key = config.session_key_bytes()?;
    if session_key.is_none() {
        warn!("SESSION_KEY not set – using ephemeral key (sessions will not survive restart)");
    }
    let sessions = Arc::new(SessionService::new(
        auth_store.clone(),
        session_key,
        config.session_ttl_hours,
    )?);

    // --- Agent tokens ---
    let agent_tokens = Arc::new(AgentTokenService::new(
        auth_store.clone(),
        config.agent_token_ttl_days,
    ));

    // --- Ceremony store (Redis, fallback in-memory) ---
    let ceremonies: Arc<dyn CeremonyStore> = match RedisCeremonyStore::new(&config.redis_url).await
    {
        Ok(store) => {
            info!("ceremony store: redis");
            Arc::new(store)
        }
        Err(e) => {
            warn!(error = %e, "redis unavailable for ceremonies – using in-memory store");
            Arc::new(InMemoryCeremonyStore::new())
        }
    };

    // --- WebAuthn ---
    let webauthn = Arc::new(
        WebauthnService::new(
            &config.webauthn_rp_id,
            &config.webauthn_rp_origin,
            &config.webauthn_rp_name,
            auth_store.clone(),
            sessions.clone(),
            ceremonies,
        )?
        .with_ceremony_ttl(config.ceremony_ttl_secs),
    );

    // --- Principal resolver ---
    let principal_resolver = Arc::new(PrincipalResolver::new(
        auth_store.clone(),
        sessions.clone(),
        agent_tokens.clone(),
    ));

    info!("auth stack ready");

    // --- Git hosting ---
    let repos_root = std::path::PathBuf::from(&config.git_repos_root);
    std::fs::create_dir_all(&repos_root)?;
    let repo_store = Arc::new(PostgresRepoStore::new(db.clone()));
    let repos = Arc::new(RepoService::new(repo_store, Arc::new(PostgresSettingsStore::new(db.clone())), PathBuf::from(&config.git_repos_root)));
    info!(root = %config.git_repos_root, "git hosting ready");

    let pr_store = Arc::new(PostgresPullRequestStore::new(db.clone()));
    let pull_requests = Arc::new(PullRequestService::new(pr_store.clone(), repos.clone(), Arc::new(PostgresReviewStore::new(db.clone()))));
    info!("pull requests ready");

    let issue_store = Arc::new(PostgresIssueStore::new(db.clone()));
    let issues = Arc::new(IssueService::new(issue_store, Arc::new(PostgresLabelStore::new(db.clone())), repos.clone()));
    info!("issues ready");

    let review_store = Arc::new(PostgresReviewStore::new(db.clone()));
    let reviews = Arc::new(ReviewService::new(review_store, pr_store as Arc<dyn tt_git::PullRequestStore>));
    info!("reviews ready");

    // --- Messaging + realtime ---
    let realtime = Arc::new(InMemoryRealtime::new());
    let msg_store = Arc::new(PostgresMessageStore::new(db.clone()));
    let events: Arc<dyn tt_realtime::EventPublisher> = realtime.clone();
    let messaging = Arc::new(MessagingService::new(msg_store, events));
    info!("messaging ready");

    // --- Workspaces ---
    let ws_store = Arc::new(PostgresWorkspaceStore::new(db.clone()));
    let workspaces = Arc::new(WorkspaceService::new(ws_store));
    info!("workspaces ready");

    let org_store = Arc::new(PostgresOrgStore::new(db.clone()));
    let orgs = Arc::new(OrgService::new(org_store, db.clone()));
    info!("orgs ready");

    let compute_store = Arc::new(PostgresComputeStore::new(db.clone()));
    let ip_alloc = Arc::new(PostgresIpAllocator::new(db.clone()));
    let compute = Arc::new(ComputeService::new(
        compute_store,
        ip_alloc,
        Arc::new(HttpProvider::new()),
    ));
    info!("compute ready (HTTP → tt-node)");

    // --- Profiles + contribution analytics ---
    let ch = {
        let client = ClickHouseClient::new(
            config.clickhouse_url.clone(),
            config.clickhouse_database.clone(),
        );
        match tokio::time::timeout(std::time::Duration::from_secs(2), client.ping()).await {
            Ok(Ok(())) => {
                info!("clickhouse reachable");
                if let Ok(sql) = std::fs::read_to_string("migrations/clickhouse/001_contribution_events.sql") {
                    if let Err(e) = client.ensure_schema(&sql).await {
                        warn!(error = %e, "clickhouse schema apply failed");
                    }
                }
                Arc::new(client)
            }
            Ok(Err(e)) => {
                warn!(error = %e, "clickhouse unavailable – dual-write disabled");
                Arc::new(ClickHouseClient::disabled())
            }
            Err(_) => {
                warn!("clickhouse ping timed out – dual-write disabled");
                Arc::new(ClickHouseClient::disabled())
            }
        }
    };
    let contrib = Arc::new(PostgresContributionStore::new(db.clone(), ch.clone()));
    let profile_store = Arc::new(PostgresProfileStore::new(db.clone()));
    let profiles = Arc::new(ProfileService::new(profile_store, contrib, db.clone()));
    info!("profiles ready");


    Ok(AppState {
        config,
        db,
        auth_store,
        sessions,
        agent_tokens,
        webauthn,
        principal_resolver,
        repos,
        pull_requests,
        issues,
        reviews,
        messaging,
        workspaces,
        orgs,
        compute,
        profiles,
        clickhouse: ch,
        realtime,
    })
}

async fn run_migrations(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    let dir = std::path::Path::new("migrations/postgres");
    if !dir.is_dir() {
        warn!("no migrations/postgres directory – skipping");
        return Ok(());
    }

    // Track applied files
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS _tt_migrations (
            filename TEXT PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"#,
    )
    .execute(pool)
    .await?;

    // Bootstrap: if legacy schema already present, mark 001 as applied
    let has_principals: (bool,) = sqlx::query_as(
        r#"SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_schema = 'public' AND table_name = 'principals'
        )"#,
    )
    .fetch_one(pool)
    .await?;
    if has_principals.0 {
        sqlx::query(
            r#"INSERT INTO _tt_migrations (filename) VALUES ('001_init.sql')
               ON CONFLICT DO NOTHING"#,
        )
        .execute(pool)
        .await?;
    }

    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().map(|e| e == "sql").unwrap_or(false)
        })
        .collect();
    files.sort();

    for path in files {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let applied: (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM _tt_migrations WHERE filename = $1)",
        )
        .bind(&name)
        .fetch_one(pool)
        .await?;
        if applied.0 {
            info!(%name, "migration already applied");
            continue;
        }
        info!(%name, "applying migration");
        let sql = std::fs::read_to_string(&path)?;
        // Postgres + sqlx: run statement-by-statement so a multi-statement
        // file cannot partially apply and still look "successful".
        for stmt in split_sql_statements(&sql) {
            if stmt.is_empty() {
                continue;
            }
            sqlx::Executor::execute(pool, stmt.as_str())
                .await
                .map_err(|e| anyhow::anyhow!("migration {name} failed on: {stmt}\n{e}"))?;
        }
        sqlx::query("INSERT INTO _tt_migrations (filename) VALUES ($1)")
            .bind(&name)
            .execute(pool)
            .await?;
        info!(%name, "migration applied");
    }
    Ok(())
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") {
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
        if trimmed.ends_with(';') {
            let s = cur.trim().trim_end_matches(';').trim().to_string();
            if !s.is_empty() {
                out.push(s);
            }
            cur.clear();
        }
    }
    let s = cur.trim().trim_end_matches(';').trim().to_string();
    if !s.is_empty() {
        out.push(s);
    }
    out
}

fn mask_url(url: &str) -> String {
    // Hide password in logs: postgres://user:pass@host → postgres://user:***@host
    if let Some(idx) = url.find("://") {
        let rest = &url[idx + 3..];
        if let Some(at) = rest.find('@') {
            let creds = &rest[..at];
            if let Some(colon) = creds.find(':') {
                let user = &creds[..colon];
                return format!("{}://{}:***@{}", &url[..idx], user, &rest[at + 1..]);
            }
        }
    }
    url.to_string()
}
