//! Short-lived WebAuthn ceremony state (registration / authentication).
//!
//! State MUST live server-side. Redis is the production backend; an in-memory
//! implementation is provided for tests and single-node dev.

mod redis_store;
mod memory;

pub use memory::InMemoryCeremonyStore;
pub use redis_store::RedisCeremonyStore;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tt_core::ids::PrincipalId;
use uuid::Uuid;
use webauthn_rs::prelude::{PasskeyAuthentication, PasskeyRegistration};

use crate::error::Result;

/// Opaque handle returned to the client (and used as Redis key).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CeremonyId(pub String);

impl CeremonyId {
    pub fn new() -> Self {
        Self(Uuid::now_v7().to_string())
    }
}

impl Default for CeremonyId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CeremonyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Data stashed between begin_registration and finish_registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationCeremony {
    pub principal_id: PrincipalId,
    pub display_name: String,
    pub handle: String,
    pub email: Option<String>,
    pub state: PasskeyRegistration,
}

/// Data stashed between begin_authentication and finish_authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationCeremony {
    pub principal_id: PrincipalId,
    pub state: PasskeyAuthentication,
}

#[async_trait]
pub trait CeremonyStore: Send + Sync {
    async fn put_registration(
        &self,
        id: &CeremonyId,
        data: &RegistrationCeremony,
        ttl_secs: u64,
    ) -> Result<()>;

    async fn take_registration(&self, id: &CeremonyId) -> Result<Option<RegistrationCeremony>>;

    async fn put_authentication(
        &self,
        id: &CeremonyId,
        data: &AuthenticationCeremony,
        ttl_secs: u64,
    ) -> Result<()>;

    async fn take_authentication(&self, id: &CeremonyId) -> Result<Option<AuthenticationCeremony>>;
}

/// Default TTL for ceremonies (5 minutes).
pub const DEFAULT_CEREMONY_TTL_SECS: u64 = 300;
