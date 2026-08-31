//! Multi-tenant isolation helpers for Turing Tarpit Cloud.
//!
//! Principles:
//! - Never trust client-supplied org_id without membership proof
//! - Workload/instance IDs are opaque; always re-load and check org
//! - Node agents authenticate with hashed tokens (not bearer user sessions)
//! - Cross-tenant data must not leak in error messages (uniform NotFound)

use sha2::{Digest, Sha256};
use tt_core::domain::Workload;
use tt_core::ids::{OrgId, WorkloadId};

use crate::error::{ComputeError, Result};

/// Constant-time-ish compare of token hash (length check + equality).
pub fn hash_agent_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

pub fn verify_agent_token(presented: &str, stored_hash: &str) -> bool {
    let h = hash_agent_token(presented);
    // Avoid early return on length in public API surface; still not full CT.
    h.len() == stored_hash.len() && h.as_bytes() == stored_hash.as_bytes()
}

pub fn mint_agent_token() -> (String, String, String) {
    // (plaintext, hash, prefix)
    let secret = uuid::Uuid::new_v4().to_string().replace('-', "")
        + &uuid::Uuid::new_v4().to_string().replace('-', "");
    let hash = hash_agent_token(&secret);
    let prefix = secret.chars().take(12).collect::<String>();
    (format!("tt_node_{secret}"), hash, prefix)
}

/// Ensure workload belongs to expected org — prevents IDOR across tenants.
pub fn assert_workload_org(wl: &Workload, org_id: OrgId) -> Result<()> {
    if wl.org_id != org_id {
        // Uniform not-found: do not reveal that the ID exists in another org.
        return Err(ComputeError::NotFound);
    }
    Ok(())
}

pub fn assert_same_org(a: OrgId, b: OrgId) -> Result<()> {
    if a != b {
        return Err(ComputeError::NotFound);
    }
    Ok(())
}

/// Reject path-param workload id that does not match org scope.
pub fn scope_workload_id(wl: &Workload, org_id: OrgId) -> Result<WorkloadId> {
    assert_workload_org(wl, org_id)?;
    Ok(wl.id)
}
