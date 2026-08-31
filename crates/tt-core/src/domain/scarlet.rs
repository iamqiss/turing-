//! Scarlet Letter ✉ — secrets that never live in .env or model context.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{OrgId, PrincipalId, RepositoryId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretScopeKind {
    Org,
    Environment,
    Repository,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretValueKind {
    String,
    Url,
    Connection,
    Token,
    Json,
    Binary,
}

impl SecretValueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Url => "url",
            Self::Connection => "connection",
            Self::Token => "token",
            Self::Json => "json",
            Self::Binary => "binary",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "string" => Some(Self::String),
            "url" => Some(Self::Url),
            "connection" => Some(Self::Connection),
            "token" => Some(Self::Token),
            "json" => Some(Self::Json),
            "binary" => Some(Self::Binary),
            _ => None,
        }
    }
}

/// Pearl-safe view — never includes plaintext or ciphertext.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretMeta {
    pub id: uuid::Uuid,
    pub org_id: OrgId,
    pub name: String,
    pub scope: SecretScopeKind,
    pub environment: Option<String>,
    pub repository_id: Option<RepositoryId>,
    pub value_kind: SecretValueKind,
    pub description: Option<String>,
    pub version: u32,
    pub exists: bool,
    pub last_rotated_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRecord {
    pub id: uuid::Uuid,
    pub org_id: OrgId,
    pub environment: Option<String>,
    pub repository_id: Option<RepositoryId>,
    pub name: String,
    pub value_kind: SecretValueKind,
    pub description: Option<String>,
    pub version: u32,
    pub created_by: PrincipalId,
    pub updated_by: PrincipalId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_rotated_at: Option<DateTime<Utc>>,
}
