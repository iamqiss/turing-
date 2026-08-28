//! Agent API tokens.
//!
//! Agents authenticate with long-lived tokens. We store only a hash.
//! The token format is: `tt_agent_<base64url(random 32 bytes)>`
//! Optionally we can also wrap claims in PASETO; for agents an opaque
//! high-entropy token + server-side hash is simpler and sufficient.

use std::sync::Arc;

use chrono::{Duration, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use tracing::info;
use tt_core::ids::PrincipalId;
use uuid::Uuid;

use crate::error::{AuthError, Result};
use crate::store::{AuthStore, StoredAgentToken};

#[derive(Debug, Clone)]
pub struct AgentTokenClaims {
    pub token_id: Uuid,
    pub agent_id: PrincipalId,
    pub scopes: Vec<String>,
}

pub struct AgentTokenService {
    store: Arc<dyn AuthStore>,
    default_ttl: Option<Duration>, // None = never expires
}

impl AgentTokenService {
    pub fn new(store: Arc<dyn AuthStore>, default_ttl_days: Option<i64>) -> Self {
        Self {
            store,
            default_ttl: default_ttl_days.map(Duration::days),
        }
    }

    /// Create a new agent token. Returns the plaintext token **once**.
    pub async fn create(
        &self,
        agent_id: PrincipalId,
        name: &str,
        scopes: Vec<String>,
    ) -> Result<(String, Uuid)> {
        // Verify the principal exists and is an agent
        // (caller should also enforce ownership)

        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let raw = format!("tt_agent_{}", base64_url_encode(&bytes));
        let token_hash = hash_token(&raw);

        let id = Uuid::now_v7();
        let now = Utc::now();
        let expires_at = self.default_ttl.map(|d| now + d);

        let stored = StoredAgentToken {
            id,
            agent_id,
            name: name.to_string(),
            token_hash,
            scopes: scopes.clone(),
            expires_at,
            last_used_at: None,
            created_at: now,
            revoked_at: None,
        };
        self.store.create_agent_token(&stored).await?;

        info!(%agent_id, token_id = %id, name, "agent token created");
        Ok((raw, id))
    }

    /// Validate an agent bearer token.
    pub async fn validate(&self, token: &str) -> Result<AgentTokenClaims> {
        if !token.starts_with("tt_agent_") {
            return Err(AuthError::InvalidToken);
        }
        let token_hash = hash_token(token);
        let stored = self
            .store
            .get_agent_token_by_hash(&token_hash)
            .await?
            .ok_or(AuthError::InvalidToken)?;

        if stored.revoked_at.is_some() {
            return Err(AuthError::InvalidToken);
        }
        if let Some(exp) = stored.expires_at {
            if exp < Utc::now() {
                return Err(AuthError::TokenExpired);
            }
        }

        Ok(AgentTokenClaims {
            token_id: stored.id,
            agent_id: stored.agent_id,
            scopes: stored.scopes,
        })
    }

    pub async fn revoke(&self, token_id: Uuid) -> Result<()> {
        self.store.revoke_agent_token(token_id).await
    }
}

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn base64_url_encode(data: &[u8]) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    URL_SAFE_NO_PAD.encode(data)
}
