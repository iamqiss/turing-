//! Turing Tarpit in-house KMS.
//!
//! Runs on TT-operated bare metal (Latitude / OVH / …). Never ships KEK material
//! off-box. Control plane only ever sees wrap/unwrap of DEKs for Scarlet Letter.
//!
//! Threat model highlights:
//! - KEK lives in process memory after unseal (or HSM later)
//! - Sealed key file on disk is encrypted under operator root key
//! - API authenticated with HMAC shared secret (mTLS next)
//! - No AWS / Vault / competitor dependency for the primary path

mod engine;
mod error;
mod seal;
mod api;

pub use engine::{KeyMeta, KmsEngine};
pub use error::{KmsError, Result};
pub use seal::{
    generate_master_key, is_legacy_seal, seal_master_key, unseal_master_key, write_sealed_file,
};
pub use api::{router, AppState};
