//! Helpers for resolving the current principal from a request.

use std::sync::Arc;

use tt_core::domain::Principal;
use tt_core::ids::PrincipalId;

use crate::error::{AuthError, Result};
use crate::session::SessionService;
use crate::store::AuthStore;
use crate::token::AgentTokenService;

#[derive(Debug, Clone)]
pub enum AuthContext {
    Human {
        principal: Principal,
        session_id: tt_core::ids::SessionId,
    },
    Agent {
        principal: Principal,
        token_id: uuid::Uuid,
        scopes: Vec<String>,
    },
}

impl AuthContext {
    pub fn principal(&self) -> &Principal {
        match self {
            Self::Human { principal, .. } => principal,
            Self::Agent { principal, .. } => principal,
        }
    }

    pub fn principal_id(&self) -> PrincipalId {
        self.principal().id
    }

    pub fn is_human(&self) -> bool {
        matches!(self, Self::Human { .. })
    }

    pub fn is_agent(&self) -> bool {
        matches!(self, Self::Agent { .. })
    }
}

pub struct PrincipalResolver {
    store: Arc<dyn AuthStore>,
    sessions: Arc<SessionService>,
    agent_tokens: Arc<AgentTokenService>,
}

impl PrincipalResolver {
    pub fn new(
        store: Arc<dyn AuthStore>,
        sessions: Arc<SessionService>,
        agent_tokens: Arc<AgentTokenService>,
    ) -> Self {
        Self {
            store,
            sessions,
            agent_tokens,
        }
    }

    /// Resolve `Authorization: Bearer <token>`.
    pub async fn resolve_bearer(&self, token: &str) -> Result<AuthContext> {
        if token.starts_with("tt_agent_") {
            let claims = self.agent_tokens.validate(token).await?;
            let principal = self
                .store
                .get_principal(claims.agent_id)
                .await?
                .ok_or(AuthError::PrincipalNotFound)?;
            if !principal.is_active {
                return Err(AuthError::PrincipalInactive);
            }
            if !principal.is_agent() {
                return Err(AuthError::InvalidToken);
            }
            return Ok(AuthContext::Agent {
                principal,
                token_id: claims.token_id,
                scopes: claims.scopes,
            });
        }

        // Assume PASETO session
        let claims = self.sessions.validate(token).await?;
        let principal = self
            .store
            .get_principal(claims.principal_id)
            .await?
            .ok_or(AuthError::PrincipalNotFound)?;
        if !principal.is_active {
            return Err(AuthError::PrincipalInactive);
        }
        Ok(AuthContext::Human {
            principal,
            session_id: claims.session_id,
        })
    }
}
