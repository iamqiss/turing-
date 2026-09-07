//! Authentication & authorization for Turing Tarpit.
//!
//! - **Humans**: WebAuthn passkeys and/or user-chosen passphrases + PASETO sessions
//! - **Agents**: Long-lived API tokens (hashed at rest)
//! - **Ceremony state**: Redis (prod) or in-memory (dev)

pub mod error;
pub mod webauthn;
pub mod passphrase;
pub mod break_glass;
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
pub use passphrase::PassphraseService;
pub use break_glass::BreakGlassService;
pub use store::AuthStore;
pub use postgres::PostgresAuthStore;
pub use principal::{AuthContext, PrincipalResolver};
pub use middleware::extract_bearer;
pub use ceremony::{
    CeremonyId, CeremonyStore, InMemoryCeremonyStore, RedisCeremonyStore,
    DEFAULT_CEREMONY_TTL_SECS,
};
