//! Auth-specific errors.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, AuthError>;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("webauthn error: {0}")]
    Webauthn(String),

    #[error("invalid token")]
    InvalidToken,

    #[error("token expired")]
    TokenExpired,

    #[error("session not found or revoked")]
    SessionNotFound,

    #[error("principal not found")]
    PrincipalNotFound,

    #[error("principal inactive")]
    PrincipalInactive,

    #[error("handle already taken")]
    HandleTaken,

    #[error("email already registered")]
    EmailTaken,

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("unauthenticated")]
    Unauthenticated,

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<webauthn_rs::prelude::WebauthnError> for AuthError {
    fn from(e: webauthn_rs::prelude::WebauthnError) -> Self {
        AuthError::Webauthn(e.to_string())
    }
}
