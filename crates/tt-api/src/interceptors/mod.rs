//! gRPC interceptors – auth extraction.

use std::sync::Arc;

use tt_auth::{extract_bearer, AuthContext, PrincipalResolver};
use tonic::metadata::MetadataMap;
use tonic::{Request, Status};

/// Extension type attached to authenticated requests.
#[derive(Clone)]
pub struct RequestAuth(pub AuthContext);

/// Extract optional bearer token from metadata.
pub fn bearer_from_metadata(meta: &MetadataMap) -> Option<String> {
    meta.get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(extract_bearer)
        .map(|s| s.to_string())
}

/// Require a valid human or agent principal.
pub async fn require_auth(
    resolver: &PrincipalResolver,
    meta: &MetadataMap,
) -> Result<AuthContext, Status> {
    let token = bearer_from_metadata(meta).ok_or_else(|| {
        Status::unauthenticated("missing Authorization: Bearer token")
    })?;
    resolver
        .resolve_bearer(&token)
        .await
        .map_err(|e| Status::unauthenticated(e.to_string()))
}

/// Attach auth to a request if present (does not fail if absent).
pub async fn maybe_auth(
    resolver: &Arc<PrincipalResolver>,
    req: &mut Request<()>,
) -> Result<(), Status> {
    if let Some(token) = bearer_from_metadata(req.metadata()) {
        let ctx = resolver
            .resolve_bearer(&token)
            .await
            .map_err(|e| Status::unauthenticated(e.to_string()))?;
        req.extensions_mut().insert(RequestAuth(ctx));
    }
    Ok(())
}
