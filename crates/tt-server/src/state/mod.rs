//! Shared application state.

use std::sync::Arc;

use sqlx::PgPool;
use tt_auth::{
    AgentTokenService, AuthStore, PrincipalResolver, SessionService, WebauthnService,
};
use tt_config::Config;
use tt_git::{PullRequestService, RepoService};
use tt_messaging::{MessagingService, WorkspaceService};
use tt_realtime::InMemoryRealtime;

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
    pub messaging: Arc<MessagingService>,
    pub workspaces: Arc<WorkspaceService>,
    pub realtime: Arc<InMemoryRealtime>,
}
