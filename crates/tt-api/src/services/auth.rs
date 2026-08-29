//! AuthService gRPC implementation.

use std::sync::Arc;

use tt_auth::{
    AuthContext, AuthError, CeremonyId, PrincipalResolver, WebauthnService, AgentTokenService,
    SessionService,
};
use tt_core::domain::PrincipalKind;
use tonic::{Request, Response, Status};
use tracing::info;
use webauthn_rs::prelude::{PublicKeyCredential, RegisterPublicKeyCredential};

use crate::interceptors::require_auth;
use crate::pb::auth::v1::auth_service_server::AuthService;
use crate::pb::auth::v1::*;
use crate::pb::common::v1::Id;

pub struct AuthServiceImpl {
    pub webauthn: Arc<WebauthnService>,
    pub sessions: Arc<SessionService>,
    pub agent_tokens: Arc<AgentTokenService>,
    pub resolver: Arc<PrincipalResolver>,
}

impl AuthServiceImpl {
    pub fn new(
        webauthn: Arc<WebauthnService>,
        sessions: Arc<SessionService>,
        agent_tokens: Arc<AgentTokenService>,
        resolver: Arc<PrincipalResolver>,
    ) -> Self {
        Self {
            webauthn,
            sessions,
            agent_tokens,
            resolver,
        }
    }
}

fn map_auth_err(e: AuthError) -> Status {
    match e {
        AuthError::Unauthenticated | AuthError::InvalidToken | AuthError::SessionNotFound => {
            Status::unauthenticated(e.to_string())
        }
        AuthError::TokenExpired => Status::unauthenticated("token expired"),
        AuthError::PermissionDenied(msg) => Status::permission_denied(msg),
        AuthError::HandleTaken | AuthError::EmailTaken => Status::already_exists(e.to_string()),
        AuthError::PrincipalNotFound => Status::not_found(e.to_string()),
        AuthError::PrincipalInactive => Status::permission_denied("principal inactive"),
        AuthError::InvalidArgument(msg) => Status::invalid_argument(msg),
        AuthError::Webauthn(msg) => Status::invalid_argument(format!("webauthn: {msg}")),
        AuthError::Other(err) => Status::internal(err.to_string()),
    }
}

fn principal_to_pb(p: &tt_core::domain::Principal) -> Principal {
    Principal {
        id: Some(Id {
            value: p.id.to_string(),
        }),
        kind: match p.kind {
            PrincipalKind::Human => "human".into(),
            PrincipalKind::Agent => "agent".into(),
        },
        display_name: p.display_name.clone(),
        handle: p.handle.clone(),
        avatar_url: p.avatar_url.clone(),
        email: p.email.clone(),
        owner_id: p.owner_id.map(|id| Id {
            value: id.to_string(),
        }),
    }
}

#[tonic::async_trait]
impl AuthService for AuthServiceImpl {
    async fn begin_registration(
        &self,
        request: Request<BeginRegistrationRequest>,
    ) -> Result<Response<BeginRegistrationResponse>, Status> {
        let req = request.into_inner();
        if req.display_name.is_empty() || req.handle.is_empty() {
            return Err(Status::invalid_argument("display_name and handle required"));
        }
        let email = if req.email.is_empty() {
            None
        } else {
            Some(req.email.as_str())
        };

        let (ccr, ceremony_id) = self
            .webauthn
            .begin_registration(&req.display_name, &req.handle, email)
            .await
            .map_err(map_auth_err)?;

        let options = serde_json::to_string(&ccr)
            .map_err(|e| Status::internal(format!("serialize challenge: {e}")))?;

        Ok(Response::new(BeginRegistrationResponse {
            public_key_credential_creation_options: options,
            ceremony_id: ceremony_id.to_string(),
        }))
    }

    async fn finish_registration(
        &self,
        request: Request<FinishRegistrationRequest>,
    ) -> Result<Response<FinishRegistrationResponse>, Status> {
        let req = request.into_inner();
        let ceremony_id = CeremonyId(req.ceremony_id);
        let credential: RegisterPublicKeyCredential = serde_json::from_str(&req.public_key_credential)
            .map_err(|e| Status::invalid_argument(format!("invalid credential JSON: {e}")))?;

        // UA / IP can be pulled from metadata later
        let (principal, session_token) = self
            .webauthn
            .finish_registration(&ceremony_id, credential, None, None)
            .await
            .map_err(map_auth_err)?;

        info!(principal_id = %principal.id, "registration complete");

        Ok(Response::new(FinishRegistrationResponse {
            principal_id: Some(Id {
                value: principal.id.to_string(),
            }),
            session_token,
        }))
    }

    async fn begin_authentication(
        &self,
        request: Request<BeginAuthenticationRequest>,
    ) -> Result<Response<BeginAuthenticationResponse>, Status> {
        let req = request.into_inner();
        if req.handle.is_empty() {
            return Err(Status::invalid_argument("handle required"));
        }

        let (rcr, ceremony_id) = self
            .webauthn
            .begin_authentication(&req.handle)
            .await
            .map_err(map_auth_err)?;

        let options = serde_json::to_string(&rcr)
            .map_err(|e| Status::internal(format!("serialize challenge: {e}")))?;

        Ok(Response::new(BeginAuthenticationResponse {
            public_key_credential_request_options: options,
            ceremony_id: ceremony_id.to_string(),
        }))
    }

    async fn finish_authentication(
        &self,
        request: Request<FinishAuthenticationRequest>,
    ) -> Result<Response<FinishAuthenticationResponse>, Status> {
        let req = request.into_inner();
        let ceremony_id = CeremonyId(req.ceremony_id);
        let credential: PublicKeyCredential = serde_json::from_str(&req.public_key_credential)
            .map_err(|e| Status::invalid_argument(format!("invalid credential JSON: {e}")))?;

        let (principal, session_token) = self
            .webauthn
            .finish_authentication(&ceremony_id, credential, None, None)
            .await
            .map_err(map_auth_err)?;

        info!(principal_id = %principal.id, "authentication complete");

        Ok(Response::new(FinishAuthenticationResponse {
            principal_id: Some(Id {
                value: principal.id.to_string(),
            }),
            session_token,
        }))
    }

    async fn get_current_principal(
        &self,
        request: Request<GetCurrentPrincipalRequest>,
    ) -> Result<Response<GetCurrentPrincipalResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        Ok(Response::new(GetCurrentPrincipalResponse {
            principal: Some(principal_to_pb(ctx.principal())),
        }))
    }

    async fn logout(
        &self,
        request: Request<LogoutRequest>,
    ) -> Result<Response<LogoutResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        if let AuthContext::Human { session_id, .. } = ctx {
            self.sessions
                .revoke(session_id)
                .await
                .map_err(map_auth_err)?;
        }
        // Agent tokens are not revoked on "logout" – use RevokeAgentToken
        Ok(Response::new(LogoutResponse {}))
    }

    async fn create_agent_token(
        &self,
        request: Request<CreateAgentTokenRequest>,
    ) -> Result<Response<CreateAgentTokenResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        // Only humans can mint agent tokens (and only for their own agents – enforced later)
        if !ctx.is_human() {
            return Err(Status::permission_denied("only humans can create agent tokens"));
        }

        let req = request.into_inner();
        let agent_id_str = req
            .agent_id
            .as_ref()
            .map(|id| id.value.as_str())
            .ok_or_else(|| Status::invalid_argument("agent_id required"))?;
        let agent_id: tt_core::ids::PrincipalId = agent_id_str
            .parse()
            .map_err(|_| Status::invalid_argument("invalid agent_id"))?;

        if req.name.is_empty() {
            return Err(Status::invalid_argument("name required"));
        }

        let (token, token_id) = self
            .agent_tokens
            .create(agent_id, &req.name, req.scopes)
            .await
            .map_err(map_auth_err)?;

        Ok(Response::new(CreateAgentTokenResponse {
            token,
            token_id: Some(Id {
                value: token_id.to_string(),
            }),
        }))
    }

    async fn revoke_agent_token(
        &self,
        request: Request<RevokeAgentTokenRequest>,
    ) -> Result<Response<RevokeAgentTokenResponse>, Status> {
        let _ctx = require_auth(&self.resolver, request.metadata()).await?;
        let req = request.into_inner();
        let token_id_str = req
            .token_id
            .as_ref()
            .map(|id| id.value.as_str())
            .ok_or_else(|| Status::invalid_argument("token_id required"))?;
        let token_id = uuid::Uuid::parse_str(token_id_str)
            .map_err(|_| Status::invalid_argument("invalid token_id"))?;

        self.agent_tokens
            .revoke(token_id)
            .await
            .map_err(map_auth_err)?;

        Ok(Response::new(RevokeAgentTokenResponse {}))
    }
}
