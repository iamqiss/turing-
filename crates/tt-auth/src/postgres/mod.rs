//! Postgres implementation of AuthStore (sqlx).

mod rows;

use async_trait::async_trait;
use sqlx::PgPool;
use tt_core::domain::{Principal, PrincipalKind};
use tt_core::ids::{PrincipalId, SessionId};
use uuid::Uuid;
use webauthn_rs::prelude::Passkey;

use crate::error::{AuthError, Result};
use crate::store::{AuthStore, StoredAgentToken, StoredSession};

use rows::*;

#[derive(Clone)]
pub struct PostgresAuthStore {
    pool: PgPool,
}

impl PostgresAuthStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AuthStore for PostgresAuthStore {
    // -----------------------------------------------------------------------
    // Principals
    // -----------------------------------------------------------------------

    async fn create_principal(&self, principal: &Principal) -> Result<()> {
        let kind = match principal.kind {
            PrincipalKind::Human => "human",
            PrincipalKind::Agent => "agent",
        };
        sqlx::query(
            r#"
            INSERT INTO principals (
                id, kind, display_name, handle, avatar_url, email,
                owner_id, is_active, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(principal.id.as_uuid())
        .bind(kind)
        .bind(&principal.display_name)
        .bind(&principal.handle)
        .bind(&principal.avatar_url)
        .bind(&principal.email)
        .bind(principal.owner_id.as_ref().map(|id| id.as_uuid()))
        .bind(principal.is_active)
        .bind(principal.created_at)
        .bind(principal.updated_at)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(())
    }

    async fn get_principal(&self, id: PrincipalId) -> Result<Option<Principal>> {
        let row = sqlx::query_as::<_, PrincipalRow>(
            r#"
            SELECT id, kind, display_name, handle, avatar_url, email,
                   owner_id, is_active, created_at, updated_at
            FROM principals
            WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(row.map(Into::into))
    }

    async fn get_principal_by_handle(&self, handle: &str) -> Result<Option<Principal>> {
        let row = sqlx::query_as::<_, PrincipalRow>(
            r#"
            SELECT id, kind, display_name, handle, avatar_url, email,
                   owner_id, is_active, created_at, updated_at
            FROM principals
            WHERE lower(handle) = lower($1)
            "#,
        )
        .bind(handle)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(row.map(Into::into))
    }

    async fn get_principal_by_email(&self, email: &str) -> Result<Option<Principal>> {
        let row = sqlx::query_as::<_, PrincipalRow>(
            r#"
            SELECT id, kind, display_name, handle, avatar_url, email,
                   owner_id, is_active, created_at, updated_at
            FROM principals
            WHERE email IS NOT NULL AND lower(email) = lower($1)
            "#,
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(row.map(Into::into))
    }

    // -----------------------------------------------------------------------
    // Passkeys
    // -----------------------------------------------------------------------

    async fn add_passkey(&self, principal_id: PrincipalId, passkey: &Passkey) -> Result<()> {
        let data = serde_json::to_value(passkey)
            .map_err(|e| AuthError::Other(anyhow::anyhow!("serialize passkey: {e}")))?;

        // Extract cred_id if available for the index (best-effort)
        let cred_id: Option<Vec<u8>> = data
            .get("cred")
            .and_then(|c| c.get("cred_id"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_u64().map(|n| n as u8))
                    .collect()
            });

        sqlx::query(
            r#"
            INSERT INTO passkeys (principal_id, credential_data, cred_id, created_at, updated_at)
            VALUES ($1, $2, $3, NOW(), NOW())
            "#,
        )
        .bind(principal_id.as_uuid())
        .bind(data)
        .bind(cred_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(())
    }

    async fn list_passkeys(&self, principal_id: PrincipalId) -> Result<Vec<Passkey>> {
        let rows = sqlx::query_as::<_, PasskeyRow>(
            r#"
            SELECT credential_data
            FROM passkeys
            WHERE principal_id = $1
            "#,
        )
        .bind(principal_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let pk: Passkey = serde_json::from_value(row.credential_data)
                .map_err(|e| AuthError::Other(anyhow::anyhow!("deserialize passkey: {e}")))?;
            out.push(pk);
        }
        Ok(out)
    }

    async fn update_passkey(&self, principal_id: PrincipalId, passkey: &Passkey) -> Result<()> {
        let data = serde_json::to_value(passkey)
            .map_err(|e| AuthError::Other(anyhow::anyhow!("serialize passkey: {e}")))?;

        // Prefer matching on cred_id when we can extract it
        let cred_id: Option<Vec<u8>> = data
            .get("cred")
            .and_then(|c| c.get("cred_id"))
            .and_then(|v| {
                // webauthn-rs may serialize as byte array or base64 string
                if let Some(arr) = v.as_array() {
                    Some(arr.iter().filter_map(|x| x.as_u64().map(|n| n as u8)).collect())
                } else if let Some(s) = v.as_str() {
                    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
                    URL_SAFE_NO_PAD.decode(s).ok()
                } else {
                    None
                }
            });

        let result = if let Some(ref cid) = cred_id {
            sqlx::query(
                r#"
                UPDATE passkeys
                SET credential_data = $3, updated_at = NOW()
                WHERE principal_id = $1 AND cred_id = $2
                "#,
            )
            .bind(principal_id.as_uuid())
            .bind(cid)
            .bind(&data)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?
        } else {
            // Fallback: update the most recently created passkey for this principal
            sqlx::query(
                r#"
                UPDATE passkeys
                SET credential_data = $2, updated_at = NOW()
                WHERE id = (
                    SELECT id FROM passkeys
                    WHERE principal_id = $1
                    ORDER BY created_at DESC
                    LIMIT 1
                )
                "#,
            )
            .bind(principal_id.as_uuid())
            .bind(&data)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?
        };

        if result.rows_affected() == 0 {
            return Err(AuthError::Other(anyhow::anyhow!("no passkey to update")));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Sessions
    // -----------------------------------------------------------------------

    async fn create_session(&self, session: &StoredSession) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO sessions (
                id, principal_id, token_hash, user_agent, ip_address,
                expires_at, created_at, last_used_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(session.id.as_uuid())
        .bind(session.principal_id.as_uuid())
        .bind(&session.token_hash)
        .bind(&session.user_agent)
        .bind(&session.ip_address)
        .bind(session.expires_at)
        .bind(session.created_at)
        .bind(session.last_used_at)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(())
    }

    async fn get_session_by_token_hash(&self, token_hash: &str) -> Result<Option<StoredSession>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, principal_id, token_hash, user_agent, ip_address,
                   expires_at, created_at, last_used_at
            FROM sessions
            WHERE token_hash = $1
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(row.map(Into::into))
    }

    async fn touch_session(&self, id: SessionId) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE sessions SET last_used_at = NOW() WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn revoke_session(&self, id: SessionId) -> Result<()> {
        // Hard delete for v1; soft-delete can be added later
        sqlx::query(r#"DELETE FROM sessions WHERE id = $1"#)
            .bind(id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn revoke_all_sessions(&self, principal_id: PrincipalId) -> Result<()> {
        sqlx::query(r#"DELETE FROM sessions WHERE principal_id = $1"#)
            .bind(principal_id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Agent tokens
    // -----------------------------------------------------------------------

    async fn create_agent_token(&self, token: &StoredAgentToken) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO agent_tokens (
                id, agent_id, name, token_hash, scopes,
                expires_at, last_used_at, created_at, revoked_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(token.id)
        .bind(token.agent_id.as_uuid())
        .bind(&token.name)
        .bind(&token.token_hash)
        .bind(&token.scopes)
        .bind(token.expires_at)
        .bind(token.last_used_at)
        .bind(token.created_at)
        .bind(token.revoked_at)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(())
    }

    async fn get_agent_token_by_hash(&self, token_hash: &str) -> Result<Option<StoredAgentToken>> {
        let row = sqlx::query_as::<_, AgentTokenRow>(
            r#"
            SELECT id, agent_id, name, token_hash, scopes,
                   expires_at, last_used_at, created_at, revoked_at
            FROM agent_tokens
            WHERE token_hash = $1
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(row.map(Into::into))
    }

    async fn revoke_agent_token(&self, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE agent_tokens
            SET revoked_at = NOW()
            WHERE id = $1 AND revoked_at IS NULL
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }
}

fn map_sqlx(e: sqlx::Error) -> AuthError {
    match &e {
        sqlx::Error::Database(db) if db.constraint().is_some() => {
            let c = db.constraint().unwrap_or("");
            if c.contains("handle") {
                AuthError::HandleTaken
            } else if c.contains("email") {
                AuthError::EmailTaken
            } else {
                AuthError::Other(e.into())
            }
        }
        _ => AuthError::Other(e.into()),
    }
}
