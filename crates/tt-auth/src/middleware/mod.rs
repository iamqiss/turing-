//! gRPC / HTTP auth middleware helpers.
//!
//! Concrete interceptors live in `tt-api`; this module provides the shared
//! extraction logic.

use crate::principal::AuthContext;

/// Header name used for the bearer token.
pub const AUTHORIZATION_HEADER: &str = "authorization";
pub const BEARER_PREFIX: &str = "Bearer ";

/// Extract the raw token from an Authorization header value.
pub fn extract_bearer(header_value: &str) -> Option<&str> {
    let header_value = header_value.trim();
    if header_value.len() > BEARER_PREFIX.len()
        && header_value[..BEARER_PREFIX.len()].eq_ignore_ascii_case(BEARER_PREFIX)
    {
        Some(header_value[BEARER_PREFIX.len()..].trim())
    } else {
        None
    }
}

/// Small helper so API layer can attach AuthContext to request extensions.
#[derive(Clone)]
pub struct CurrentAuth(pub AuthContext);
