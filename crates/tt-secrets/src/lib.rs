//! Scarlet Letter ✉ — org / env / repo secrets with envelope encryption.
//!
//! Plaintext never touches disk. Pearl (agent) only sees metadata: name, kind, exists.
//! CLI: `tt secrets pull` / `tt secrets run -- <cmd>` materializes into process env only.

mod crypto;
mod kms;
mod error;
mod service;
mod store;
mod scan;

pub use crypto::Envelope;
pub use kms::{build_provider, KmsConfig, KeyProvider, LocalKeyProvider};
pub use error::{Result, SecretsError};
pub use service::SecretsService;
pub use store::{PostgresSecretsStore, SecretRow, SecretsStore};
pub use scan::{apply_allowlist, contains_secret_shaped, redact_for_model, scan_text, Finding, ScanReport};
