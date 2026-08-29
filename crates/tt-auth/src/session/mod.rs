//! PASETO v4.local session tokens.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use pasetors::claims::{Claims, ClaimsValidationRules};
use pasetors::keys::{Generate, SymmetricKey};
use pasetors::local;
use pasetors::token::UntrustedToken;
use pasetors::version4::V4;
use pasetors::Local;
use sha2::{Digest, Sha256};
use tracing::debug;
use tt_core::ids::{PrincipalId, SessionId};

use crate::error::{AuthError, Result};
use crate::store::{AuthStore, StoredSession};

/// Claims embedded in a session PASETO.
#[derive(Debug, Clone)]
pub struct SessionClaims {
    pub session_id: SessionId,
    pub principal_id: PrincipalId,
    pub exp: chrono::DateTime<Utc>,
}

pub struct SessionService {
    key: SymmetricKey<V4>,
    store: Arc<dyn AuthStore>,
    ttl: Duration,
}

impl SessionService {
    pub fn new(
        store: Arc<dyn AuthStore>,
        key_bytes: Option<[u8; 32]>,
        ttl_hours: i64,
    ) -> Result<Self> {
        let key = match key_bytes {
            Some(bytes) => SymmetricKey::<V4>::from(bytes.as_slice())
                .map_err(|e| AuthError::Other(anyhow::anyhow!("invalid session key: {e}")))?,
            None => SymmetricKey::<V4>::generate()
                .map_err(|e| AuthError::Other(anyhow::anyhow!("keygen failed: {e}")))?,
        };
        Ok(Self {
            key,
            store,
            ttl: Duration::hours(ttl_hours.max(1)),
        })
    }

    pub async fn create_session(
        &self,
        principal_id: PrincipalId,
        user_agent: Option<String>,
        ip_address: Option<String>,
    ) -> Result<String> {
        let session_id = SessionId::new();
        let now = Utc::now();
        let expires_at = now + self.ttl;

        // Registered claims must use the dedicated setters — not add_additional.
        let mut claims = Claims::new().map_err(|e| AuthError::Other(e.into()))?;
        claims
            .subject(&principal_id.to_string())
            .map_err(|e| AuthError::Other(anyhow::anyhow!("paseto sub: {e}")))?;
        claims
            .set_expires_in(&StdDuration::from_secs(self.ttl.num_seconds() as u64))
            .map_err(|e| AuthError::Other(anyhow::anyhow!("paseto exp: {e}")))?;
        claims
            .add_additional("sid", session_id.to_string())
            .map_err(|e| AuthError::Other(anyhow::anyhow!("paseto sid: {e}")))?;

        let token = local::encrypt(&self.key, &claims, None, None)
            .map_err(|e| AuthError::Other(anyhow::anyhow!("paseto encrypt: {e}")))?;

        let token_hash = hash_token(&token);
        let stored = StoredSession {
            id: session_id,
            principal_id,
            token_hash,
            user_agent,
            ip_address,
            expires_at,
            created_at: now,
            last_used_at: now,
        };
        self.store.create_session(&stored).await?;

        debug!(%session_id, %principal_id, "session created");
        Ok(token)
    }

    pub async fn validate(&self, token: &str) -> Result<SessionClaims> {
        let validation = ClaimsValidationRules::new();

        let untrusted =
            UntrustedToken::<Local, V4>::try_from(token).map_err(|_| AuthError::InvalidToken)?;

        let trusted = local::decrypt(&self.key, &untrusted, &validation, None, None)
            .map_err(|_| AuthError::InvalidToken)?;

        let claims = trusted.payload_claims().ok_or(AuthError::InvalidToken)?;

        let principal_id_str = claims
            .get_claim("sub")
            .and_then(|v| v.as_str())
            .ok_or(AuthError::InvalidToken)?;
        let principal_id: PrincipalId = principal_id_str
            .parse()
            .map_err(|_| AuthError::InvalidToken)?;

        let session_id_str = claims
            .get_claim("sid")
            .and_then(|v| v.as_str())
            .ok_or(AuthError::InvalidToken)?;
        let session_id: SessionId = session_id_str
            .parse()
            .map_err(|_| AuthError::InvalidToken)?;

        // exp is ISO-8601 string when set via set_expires_in / expiration
        let exp = if let Some(v) = claims.get_claim("exp") {
            if let Some(s) = v.as_str() {
                chrono::DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|_| AuthError::InvalidToken)?
            } else if let Some(n) = v.as_i64() {
                chrono::DateTime::from_timestamp(n, 0).ok_or(AuthError::InvalidToken)?
            } else {
                return Err(AuthError::InvalidToken);
            }
        } else {
            return Err(AuthError::InvalidToken);
        };

        if exp < Utc::now() {
            return Err(AuthError::TokenExpired);
        }

        let token_hash = hash_token(token);
        let stored = self
            .store
            .get_session_by_token_hash(&token_hash)
            .await?
            .ok_or(AuthError::SessionNotFound)?;

        if stored.expires_at < Utc::now() {
            return Err(AuthError::TokenExpired);
        }

        let _ = self.store.touch_session(session_id).await;

        Ok(SessionClaims {
            session_id,
            principal_id,
            exp,
        })
    }

    pub async fn revoke(&self, session_id: SessionId) -> Result<()> {
        self.store.revoke_session(session_id).await
    }

    pub async fn revoke_all(&self, principal_id: PrincipalId) -> Result<()> {
        self.store.revoke_all_sessions(principal_id).await
    }
}

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}
