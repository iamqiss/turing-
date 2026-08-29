//! Persistence interface for auth. Implemented by the server against Postgres.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tt_core::domain::Principal;
use tt_core::ids::{PrincipalId, SessionId};
use uuid::Uuid;
use webauthn_rs::prelude::Passkey;

use crate::error::Result;

/// Stored WebAuthn passkey for a human principal.
#[derive(Debug, Clone)]
pub struct StoredPasskey {
    pub principal_id: PrincipalId,
    pub passkey: Passkey,
    pub created_at: DateTime<Utc>,
}

/// Session row (PASETO is stateless, but we keep a server-side record for revocation).
#[derive(Debug, Clone)]
pub struct StoredSession {
    pub id: SessionId,
    pub principal_id: PrincipalId,
    pub token_hash: String,       // hash of the PASETO or a session id claim
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct StoredAgentToken {
    pub id: Uuid,
    pub agent_id: PrincipalId,
    pub name: String,
    pub token_hash: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait AuthStore: Send + Sync {
    // --- Principals ---
    async fn create_principal(&self, principal: &Principal) -> Result<()>;
    async fn get_principal(&self, id: PrincipalId) -> Result<Option<Principal>>;
    async fn get_principal_by_handle(&self, handle: &str) -> Result<Option<Principal>>;
    async fn get_principal_by_email(&self, email: &str) -> Result<Option<Principal>>;

    // --- Passkeys ---
    async fn add_passkey(&self, principal_id: PrincipalId, passkey: &Passkey) -> Result<()>;
    async fn list_passkeys(&self, principal_id: PrincipalId) -> Result<Vec<Passkey>>;
    async fn update_passkey(&self, principal_id: PrincipalId, passkey: &Passkey) -> Result<()>;

    // --- Sessions ---
    async fn create_session(&self, session: &StoredSession) -> Result<()>;
    async fn get_session_by_token_hash(&self, token_hash: &str) -> Result<Option<StoredSession>>;
    async fn touch_session(&self, id: SessionId) -> Result<()>;
    async fn revoke_session(&self, id: SessionId) -> Result<()>;
    async fn revoke_all_sessions(&self, principal_id: PrincipalId) -> Result<()>;

    // --- Agent tokens ---
    async fn create_agent_token(&self, token: &StoredAgentToken) -> Result<()>;
    async fn get_agent_token_by_hash(&self, token_hash: &str) -> Result<Option<StoredAgentToken>>;
    async fn revoke_agent_token(&self, id: Uuid) -> Result<()>;
}
