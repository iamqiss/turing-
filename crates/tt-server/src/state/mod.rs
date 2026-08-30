//! Shared application state.

use std::sync::Arc;

use sqlx::PgPool;
use tt_auth::{
    AgentTokenService, AuthStore, PrincipalResolver, SessionService, WebauthnService,
};
use tt_config::Config;
use tt_git::{IssueService, PullRequestService, RepoService, ReviewService};
use tt_messaging::{MessagingService, OrgService, WorkspaceService};
use tt_realtime::InMemoryRealtime;
use tt_storage::{ClickHouseClient, ProfileService};
use tt_compute::ComputeService;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: PgPool,
    pub auth_store: Arc<dyn AuthStore>,
    pub sessions: Arc<SessionService>,
    pub agent_tokens: Arc<AgentTokenService>,
    pub webauthn: Arc<WebauthnService>,
    pub principal_resolver: Arc<PrincipalResolver>,
    pub repos: Arc<RepoService>,
    pub pull_requests: Arc<PullRequestService>,
    pub issues: Arc<IssueService>,
    pub reviews: Arc<ReviewService>,
    pub messaging: Arc<MessagingService>,
    pub workspaces: Arc<WorkspaceService>,
    pub orgs: Arc<OrgService>,
    pub compute: Arc<ComputeService>,
    pub profiles: Arc<ProfileService>,
    pub clickhouse: Arc<ClickHouseClient>,
    pub realtime: Arc<InMemoryRealtime>,
}
