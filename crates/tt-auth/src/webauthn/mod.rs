//! WebAuthn passkey registration & authentication (webauthn-rs).

use std::sync::Arc;

use chrono::Utc;
use tracing::info;
use tt_core::domain::{Principal, PrincipalKind};
use tt_core::ids::PrincipalId;
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::*;

use crate::ceremony::{
    AuthenticationCeremony, CeremonyId, CeremonyStore, RegistrationCeremony,
    DEFAULT_CEREMONY_TTL_SECS,
};
use crate::error::{AuthError, Result};
use crate::session::SessionService;
use crate::store::AuthStore;

/// Holds the Webauthn RP configuration and orchestrates ceremonies.
pub struct WebauthnService {
    webauthn: Webauthn,
    store: Arc<dyn AuthStore>,
    sessions: Arc<SessionService>,
    ceremonies: Arc<dyn CeremonyStore>,
    ceremony_ttl_secs: u64,
}

impl WebauthnService {
    pub fn new(
        rp_id: &str,
        rp_origin: &str,
        rp_name: &str,
        store: Arc<dyn AuthStore>,
        sessions: Arc<SessionService>,
        ceremonies: Arc<dyn CeremonyStore>,
    ) -> Result<Self> {
        let origin = Url::parse(rp_origin)
            .map_err(|e| AuthError::InvalidArgument(format!("invalid rp_origin: {e}")))?;
        let mut builder = WebauthnBuilder::new(rp_id, &origin)
            .map_err(|e| AuthError::Webauthn(e.to_string()))?;
        builder = builder.rp_name(rp_name);
        let webauthn = builder
            .build()
            .map_err(|e| AuthError::Webauthn(e.to_string()))?;

        Ok(Self {
            webauthn,
            store,
            sessions,
            ceremonies,
            ceremony_ttl_secs: DEFAULT_CEREMONY_TTL_SECS,
        })
    }

    pub fn with_ceremony_ttl(mut self, ttl_secs: u64) -> Self {
        self.ceremony_ttl_secs = ttl_secs;
        self
    }

    // ------------------------------------------------------------------
    // Registration
    // ------------------------------------------------------------------

    /// Start passkey registration for a new human.
    ///
    /// Returns the challenge (send to browser) and a `CeremonyId` the client
    /// must pass back to `finish_registration`.
    pub async fn begin_registration(
        &self,
        display_name: &str,
        handle: &str,
        email: Option<&str>,
    ) -> Result<(CreationChallengeResponse, CeremonyId)> {
        if self.store.get_principal_by_handle(handle).await?.is_some() {
            return Err(AuthError::HandleTaken);
        }
        if let Some(email) = email {
            if self.store.get_principal_by_email(email).await?.is_some() {
                return Err(AuthError::EmailTaken);
            }
        }

        let principal_id = PrincipalId::new();
        let user_unique_id: Uuid = *principal_id.as_uuid();

        let (ccr, reg_state) = self.webauthn.start_passkey_registration(
            user_unique_id,
            handle,
            display_name,
            None,
        )?;

        let ceremony_id = CeremonyId::new();
        let data = RegistrationCeremony {
            principal_id,
            display_name: display_name.to_string(),
            handle: handle.to_string(),
            email: email.map(|s| s.to_string()),
            state: reg_state,
        };
        self.ceremonies
            .put_registration(&ceremony_id, &data, self.ceremony_ttl_secs)
            .await?;

        Ok((ccr, ceremony_id))
    }

    /// Finish registration using the ceremony id from `begin_registration`.
    pub async fn finish_registration(
        &self,
        ceremony_id: &CeremonyId,
        reg: RegisterPublicKeyCredential,
        user_agent: Option<String>,
        ip_address: Option<String>,
    ) -> Result<(Principal, String)> {
        let data = self
            .ceremonies
            .take_registration(ceremony_id)
            .await?
            .ok_or_else(|| AuthError::InvalidArgument("ceremony expired or unknown".into()))?;

        let passkey = self
            .webauthn
            .finish_passkey_registration(&reg, &data.state)?;

        let now = Utc::now();
        let principal = Principal {
            id: data.principal_id,
            kind: PrincipalKind::Human,
            display_name: data.display_name,
            handle: data.handle,
            avatar_url: None,
            email: data.email,
            owner_id: None,
            is_active: true,
            created_at: now,
            updated_at: now,
        };

        self.store.create_principal(&principal).await?;
        self.store.add_passkey(data.principal_id, &passkey).await?;

        let session_token = self
            .sessions
            .create_session(data.principal_id, user_agent, ip_address)
            .await?;

        info!(principal_id = %data.principal_id, handle = %principal.handle, "human registered via WebAuthn");
        Ok((principal, session_token))
    }

    // ------------------------------------------------------------------
    // Authentication
    // ------------------------------------------------------------------

    /// Start authentication for an existing human (by handle).
    pub async fn begin_authentication(
        &self,
        handle: &str,
    ) -> Result<(RequestChallengeResponse, CeremonyId)> {
        let principal = self
            .store
            .get_principal_by_handle(handle)
            .await?
            .ok_or(AuthError::PrincipalNotFound)?;

        if !principal.is_active {
            return Err(AuthError::PrincipalInactive);
        }
        if !principal.is_human() {
            return Err(AuthError::InvalidArgument(
                "agents cannot authenticate via WebAuthn".into(),
            ));
        }

        let passkeys = self.store.list_passkeys(principal.id).await?;
        if passkeys.is_empty() {
            return Err(AuthError::InvalidArgument(
                "no passkeys registered for this user".into(),
            ));
        }

        let (rcr, auth_state) = self.webauthn.start_passkey_authentication(&passkeys)?;

        let ceremony_id = CeremonyId::new();
        let data = AuthenticationCeremony {
            principal_id: principal.id,
            state: auth_state,
        };
        self.ceremonies
            .put_authentication(&ceremony_id, &data, self.ceremony_ttl_secs)
            .await?;

        Ok((rcr, ceremony_id))
    }

    /// Finish authentication using the ceremony id from `begin_authentication`.
    pub async fn finish_authentication(
        &self,
        ceremony_id: &CeremonyId,
        auth: PublicKeyCredential,
        user_agent: Option<String>,
        ip_address: Option<String>,
    ) -> Result<(Principal, String)> {
        let data = self
            .ceremonies
            .take_authentication(ceremony_id)
            .await?
            .ok_or_else(|| AuthError::InvalidArgument("ceremony expired or unknown".into()))?;

        let _auth_result = self
            .webauthn
            .finish_passkey_authentication(&auth, &data.state)?;

        // Counter / backup state updates can be persisted here later via update_passkey.

        let principal = self
            .store
            .get_principal(data.principal_id)
            .await?
            .ok_or(AuthError::PrincipalNotFound)?;

        if !principal.is_active {
            return Err(AuthError::PrincipalInactive);
        }

        let session_token = self
            .sessions
            .create_session(data.principal_id, user_agent, ip_address)
            .await?;

        info!(principal_id = %data.principal_id, "human authenticated via WebAuthn");
        Ok((principal, session_token))
    }
}
