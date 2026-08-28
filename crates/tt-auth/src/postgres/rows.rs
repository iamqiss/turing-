//! sqlx row types and conversions.

use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use tt_core::domain::{Principal, PrincipalKind};
use tt_core::ids::{PrincipalId, SessionId};
use uuid::Uuid;

use crate::store::{StoredAgentToken, StoredSession};

#[derive(Debug, FromRow)]
pub(crate) struct PrincipalRow {
    pub id: Uuid,
    pub kind: String,
    pub display_name: String,
    pub handle: String,
    pub avatar_url: Option<String>,
    pub email: Option<String>,
    pub owner_id: Option<Uuid>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<PrincipalRow> for Principal {
    fn from(r: PrincipalRow) -> Self {
        let kind = match r.kind.as_str() {
            "agent" => PrincipalKind::Agent,
            _ => PrincipalKind::Human,
        };
        Principal {
            id: PrincipalId::from_uuid(r.id),
            kind,
            display_name: r.display_name,
            handle: r.handle,
            avatar_url: r.avatar_url,
            email: r.email,
            owner_id: r.owner_id.map(PrincipalId::from_uuid),
            is_active: r.is_active,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct PasskeyRow {
    pub credential_data: JsonValue,
}

#[derive(Debug, FromRow)]
pub(crate) struct SessionRow {
    pub id: Uuid,
    pub principal_id: Uuid,
    pub token_hash: String,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

impl From<SessionRow> for StoredSession {
    fn from(r: SessionRow) -> Self {
        StoredSession {
            id: SessionId::from_uuid(r.id),
            principal_id: PrincipalId::from_uuid(r.principal_id),
            token_hash: r.token_hash,
            user_agent: r.user_agent,
            ip_address: r.ip_address,
            expires_at: r.expires_at,
            created_at: r.created_at,
            last_used_at: r.last_used_at,
        }
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct AgentTokenRow {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub name: String,
    pub token_hash: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<AgentTokenRow> for StoredAgentToken {
    fn from(r: AgentTokenRow) -> Self {
        StoredAgentToken {
            id: r.id,
            agent_id: PrincipalId::from_uuid(r.agent_id),
            name: r.name,
            token_hash: r.token_hash,
            scopes: r.scopes,
            expires_at: r.expires_at,
            last_used_at: r.last_used_at,
            created_at: r.created_at,
            revoked_at: r.revoked_at,
        }
    }
}
