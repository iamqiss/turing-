//! Authentication & authorization for Turing Tarpit.
//!
//! - **Humans**: WebAuthn passkeys (webauthn-rs) + PASETO v4.local session tokens
//! - **Agents**: Long-lived API tokens (hashed at rest)
//! - **Ceremony state**: Redis (prod) or in-memory (dev)

pub mod error;
pub mod webauthn;
pub mod session;
pub mod token;
pub mod principal;
pub mod middleware;
pub mod store;
pub mod postgres;
pub mod ceremony;

pub use error::{AuthError, Result};
pub use session::{SessionClaims, SessionService};
pub use token::{AgentTokenClaims, AgentTokenService};
pub use webauthn::WebauthnService;
pub use store::AuthStore;
pub use postgres::PostgresAuthStore;
pub use principal::{AuthContext, PrincipalResolver};
pub use ceremony::{
    CeremonyId, CeremonyStore, InMemoryCeremonyStore, RedisCeremonyStore,
    DEFAULT_CEREMONY_TTL_SECS,
};
