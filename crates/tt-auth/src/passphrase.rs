//! User-chosen passphrases for humans (CLI + clients that skip WebAuthn).
//!
//! Stored as Argon2id PHC hashes. Never log plaintext. WebAuthn remains available;
//! a principal may use either or both.

use std::sync::Arc;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Argon2, Algorithm, Params, Version};
use chrono::Utc;
use rand::rngs::OsRng;
use tracing::info;
use tt_core::domain::{Principal, PrincipalKind};
use tt_core::ids::PrincipalId;

use crate::error::{AuthError, Result};
use crate::session::SessionService;
use crate::store::AuthStore;

/// Policy for user-chosen passphrases.
pub struct PassphrasePolicy {
    pub min_len: usize,
    pub max_len: usize,
}

impl Default for PassphrasePolicy {
    fn default() -> Self {
        Self {
            min_len: 12,
            max_len: 128,
        }
    }
}

impl PassphrasePolicy {
    pub fn validate(&self, passphrase: &str) -> Result<()> {
        let len = passphrase.chars().count();
        if len < self.min_len {
            return Err(AuthError::InvalidArgument(format!(
                "passphrase must be at least {} characters",
                self.min_len
            )));
        }
        if len > self.max_len {
            return Err(AuthError::InvalidArgument(format!(
                "passphrase must be at most {} characters",
                self.max_len
            )));
        }
        // Reject all-whitespace
        if passphrase.trim().is_empty() {
            return Err(AuthError::InvalidArgument(
                "passphrase must not be blank".into(),
            ));
        }
        Ok(())
    }
}

fn argon2() -> Argon2<'static> {
    // OWASP-ish interactive defaults for Argon2id
    let params = Params::new(19_456, 2, 1, None).expect("argon2 params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

pub fn hash_passphrase(passphrase: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2()
        .hash_password(passphrase.as_bytes(), &salt)
        .map_err(|e| AuthError::Other(anyhow::anyhow!("argon2 hash: {e}")))?
        .to_string();
    Ok(hash)
}

pub fn verify_passphrase(passphrase: &str, phc_hash: &str) -> Result<bool> {
    let parsed = PasswordHash::new(phc_hash)
        .map_err(|e| AuthError::Other(anyhow::anyhow!("invalid stored hash: {e}")))?;
    Ok(argon2()
        .verify_password(passphrase.as_bytes(), &parsed)
        .is_ok())
}

pub struct PassphraseService {
    store: Arc<dyn AuthStore>,
    sessions: Arc<SessionService>,
    policy: PassphrasePolicy,
}

impl PassphraseService {
    pub fn new(store: Arc<dyn AuthStore>, sessions: Arc<SessionService>) -> Self {
        Self {
            store,
            sessions,
            policy: PassphrasePolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: PassphrasePolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Register a new human with handle + passphrase (optional email).
    /// Creates principal + passphrase hash + session token.
    pub async fn register(
        &self,
        handle: &str,
        display_name: &str,
        email: Option<&str>,
        passphrase: &str,
    ) -> Result<(Principal, String)> {
        self.policy.validate(passphrase)?;
        let handle = handle.trim();
        if handle.is_empty() || display_name.trim().is_empty() {
            return Err(AuthError::InvalidArgument(
                "handle and display_name are required".into(),
            ));
        }
        if self.store.get_principal_by_handle(handle).await?.is_some() {
            return Err(AuthError::HandleTaken);
        }
        if let Some(e) = email {
            if !e.is_empty() && self.store.get_principal_by_email(e).await?.is_some() {
                return Err(AuthError::EmailTaken);
            }
        }

        let now = Utc::now();
        let principal = Principal {
            id: PrincipalId::new(),
            kind: PrincipalKind::Human,
            display_name: display_name.trim().to_string(),
            handle: handle.to_string(),
            avatar_url: None,
            email: email
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            owner_id: None,
            is_active: true,
            created_at: now,
            updated_at: now,
        };
        self.store.create_principal(&principal).await?;

        let hash = hash_passphrase(passphrase)?;
        self.store
            .set_passphrase_hash(principal.id, &hash)
            .await?;

        let token = self
            .sessions
            .create_session(principal.id, Some("passphrase-register".into()), None)
            .await?;

        info!(principal_id = %principal.id, handle, "human registered with passphrase");
        Ok((principal, token))
    }

    /// Attach or rotate passphrase on an existing principal (must already be authenticated
    /// via another factor, or used only for admin recovery — here: requires current session
    /// is enforced by caller).
    pub async fn set_passphrase(&self, principal_id: PrincipalId, passphrase: &str) -> Result<()> {
        self.policy.validate(passphrase)?;
        let hash = hash_passphrase(passphrase)?;
        self.store.set_passphrase_hash(principal_id, &hash).await?;
        info!(%principal_id, "passphrase set/rotated");
        Ok(())
    }

    /// Login with handle + passphrase → session token.
    pub async fn login(&self, handle: &str, passphrase: &str) -> Result<(Principal, String)> {
        let principal = self
            .store
            .get_principal_by_handle(handle.trim())
            .await?
            .ok_or(AuthError::Unauthenticated)?;
        if !principal.is_active {
            return Err(AuthError::PrincipalInactive);
        }
        if principal.kind != PrincipalKind::Human {
            return Err(AuthError::InvalidArgument(
                "only human principals use passphrases".into(),
            ));
        }

        let Some(hash) = self.store.get_passphrase_hash(principal.id).await? else {
            return Err(AuthError::InvalidArgument(
                "no passphrase set for this account; use passkey or set a passphrase".into(),
            ));
        };

        if !verify_passphrase(passphrase, &hash)? {
            return Err(AuthError::Unauthenticated);
        }

        let token = self
            .sessions
            .create_session(principal.id, Some("passphrase-login".into()), None)
            .await?;

        info!(principal_id = %principal.id, "passphrase login ok");
        Ok((principal, token))
    }
}
