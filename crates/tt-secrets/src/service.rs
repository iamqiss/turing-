//! Scarlet Letter service — put / pull / meta / rotate / audit.

use std::sync::Arc;

use chrono::Utc;
use tt_core::domain::{SecretMeta, SecretScopeKind, SecretValueKind};
use tt_core::ids::{OrgId, PrincipalId, RepositoryId};
use uuid::Uuid;

use crate::crypto::Envelope;
use crate::error::{Result, SecretsError};
use crate::kms::{build_providers, KmsConfig, KeyProvider};
use crate::store::{SecretRow, SecretsStore};

pub struct SecretsService {
    store: Arc<dyn SecretsStore>,
    envelope: Envelope,
}

impl SecretsService {
    pub fn new(store: Arc<dyn SecretsStore>, kek: [u8; 32]) -> Self {
        // Back-compat: local provider from raw KEK bytes
        let provider: Arc<dyn KeyProvider> = Arc::new(crate::kms::LocalKeyProvider {
            // re-export via constructor helper
            key: kek,
            key_id: "local".into(),
        });
        Self {
            store,
            envelope: Envelope::new(provider),
        }
    }

    pub fn with_kms(store: Arc<dyn SecretsStore>, kms: &KmsConfig) -> Result<Self> {
        let (active, retired) = build_providers(kms)?;
        Ok(Self {
            store,
            envelope: Envelope::new(active).with_retired(retired),
        })
    }

    fn validate_name(name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name.len() > 128 {
            return Err(SecretsError::Invalid("name length".into()));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(SecretsError::Invalid(
                "name must be [A-Za-z0-9_-]".into(),
            ));
        }
        Ok(())
    }

    fn scope_of(environment: Option<&str>, repository_id: Option<RepositoryId>) -> SecretScopeKind {
        if repository_id.is_some() {
            SecretScopeKind::Repository
        } else if environment.is_some() {
            SecretScopeKind::Environment
        } else {
            SecretScopeKind::Org
        }
    }

    fn row_to_meta(row: &SecretRow) -> SecretMeta {
        SecretMeta {
            id: row.id,
            org_id: OrgId::from_uuid(row.org_id),
            name: row.name.clone(),
            scope: Self::scope_of(row.environment.as_deref(), row.repository_id.map(RepositoryId::from_uuid)),
            environment: row.environment.clone(),
            repository_id: row.repository_id.map(RepositoryId::from_uuid),
            value_kind: SecretValueKind::parse(&row.value_kind).unwrap_or(SecretValueKind::String),
            description: row.description.clone(),
            version: row.version.max(0) as u32,
            exists: true,
            last_rotated_at: row.last_rotated_at,
            updated_at: row.updated_at,
        }
    }

    /// Put or rotate a secret. Plaintext never leaves this stack frame unencrypted.
    pub async fn put(
        &self,
        org_id: OrgId,
        actor: PrincipalId,
        name: &str,
        value: &str,
        environment: Option<&str>,
        repository_id: Option<RepositoryId>,
        value_kind: SecretValueKind,
        description: Option<String>,
        client_kind: &str,
    ) -> Result<SecretMeta> {
        Self::validate_name(name)?;
        if value.is_empty() {
            return Err(SecretsError::Invalid("value required".into()));
        }
        if environment.is_some() && repository_id.is_some() {
            return Err(SecretsError::Invalid(
                "set environment OR repository, not both".into(),
            ));
        }

        let prior = self
            .store
            .find(org_id, name, environment, repository_id)
            .await?;
        let version = prior.as_ref().map(|p| p.version + 1).unwrap_or(1);
        let (wrapped_dek, nonce, ciphertext) = self.envelope.seal(value.as_bytes()).await?;
        let now = Utc::now();
        let id = prior.as_ref().map(|p| p.id).unwrap_or_else(Uuid::now_v7);
        let created_by = prior
            .as_ref()
            .map(|p| p.created_by)
            .unwrap_or_else(|| *actor.as_uuid());
        let created_at = prior
            .as_ref()
            .map(|p| p.created_at)
            .unwrap_or(now);

        let row = SecretRow {
            id,
            org_id: *org_id.as_uuid(),
            environment: environment.map(|s| s.to_string()),
            repository_id: repository_id.map(|id| *id.as_uuid()),
            name: name.trim().to_string(),
            value_kind: value_kind.as_str().to_string(),
            description,
            wrapped_dek,
            ciphertext,
            nonce,
            version,
            created_by,
            updated_by: *actor.as_uuid(),
            created_at,
            updated_at: now,
            last_rotated_at: if prior.is_some() { Some(now) } else { None },
        };
        self.store.upsert(&row).await?;
        let action = if prior.is_some() { "rotate" } else { "put" };
        let _ = self.store.audit(id, actor, action, client_kind).await;
        Ok(Self::row_to_meta(&row))
    }

    /// Decrypt for authorized pull (CLI / CI). Audited.
    pub async fn pull_value(
        &self,
        org_id: OrgId,
        actor: PrincipalId,
        name: &str,
        environment: Option<&str>,
        repository_id: Option<RepositoryId>,
        client_kind: &str,
    ) -> Result<String> {
        // Resolution order when env set: repo (if provided) already explicit;
        // for env scope try env then fall back to org-level same name
        let row = if let Some(rid) = repository_id {
            self.store
                .find(org_id, name, None, Some(rid))
                .await?
                .ok_or(SecretsError::NotFound)?
        } else if let Some(env) = environment {
            match self.store.find(org_id, name, Some(env), None).await? {
                Some(r) => r,
                None => self
                    .store
                    .find(org_id, name, None, None)
                    .await?
                    .ok_or(SecretsError::NotFound)?,
            }
        } else {
            self.store
                .find(org_id, name, None, None)
                .await?
                .ok_or(SecretsError::NotFound)?
        };

        let plain = self
            .envelope
            .open(&row.wrapped_dek, &row.nonce, &row.ciphertext)
            .await?;
        let value = String::from_utf8(plain).map_err(|_| SecretsError::Crypto("utf8".into()))?;
        let _ = self
            .store
            .audit(row.id, actor, "pull", client_kind)
            .await;
        Ok(value)
    }

    /// Pearl / IDE: metadata only — never values.
    pub async fn list_meta(
        &self,
        org_id: OrgId,
        actor: PrincipalId,
        environment: Option<&str>,
        repository_id: Option<RepositoryId>,
        client_kind: &str,
    ) -> Result<Vec<SecretMeta>> {
        let rows = self
            .store
            .list_meta(org_id, environment, repository_id)
            .await?;
        for r in &rows {
            let _ = self
                .store
                .audit(r.id, actor, "pearl_meta", client_kind)
                .await;
        }
        Ok(rows.iter().map(Self::row_to_meta).collect())
    }

    /// Pearl: does this secret exist + shape? No value.
    pub async fn pearl_probe(
        &self,
        org_id: OrgId,
        actor: PrincipalId,
        name: &str,
        environment: Option<&str>,
        repository_id: Option<RepositoryId>,
    ) -> Result<SecretMeta> {
        let row = if let Some(rid) = repository_id {
            self.store.find(org_id, name, None, Some(rid)).await?
        } else if let Some(env) = environment {
            match self.store.find(org_id, name, Some(env), None).await? {
                Some(r) => Some(r),
                None => self.store.find(org_id, name, None, None).await?,
            }
        } else {
            self.store.find(org_id, name, None, None).await?
        };
        match row {
            Some(r) => {
                let _ = self.store.audit(r.id, actor, "pearl_meta", "pearl").await;
                Ok(Self::row_to_meta(&r))
            }
            None => Ok(SecretMeta {
                id: Uuid::nil(),
                org_id,
                name: name.to_string(),
                scope: Self::scope_of(environment, repository_id),
                environment: environment.map(|s| s.to_string()),
                repository_id,
                value_kind: SecretValueKind::String,
                description: None,
                version: 0,
                exists: false,
                last_rotated_at: None,
                updated_at: Utc::now(),
            }),
        }
    }

    /// Pull many names for `tt secrets run` — map name → value (ephemeral).
    pub async fn pull_map(
        &self,
        org_id: OrgId,
        actor: PrincipalId,
        names: &[String],
        environment: Option<&str>,
        repository_id: Option<RepositoryId>,
        client_kind: &str,
    ) -> Result<std::collections::BTreeMap<String, String>> {
        let mut out = std::collections::BTreeMap::new();
        for name in names {
            match self
                .pull_value(org_id, actor, name, environment, repository_id, client_kind)
                .await
            {
                Ok(v) => {
                    out.insert(name.clone(), v);
                }
                Err(SecretsError::NotFound) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    pub async fn delete(
        &self,
        org_id: OrgId,
        actor: PrincipalId,
        name: &str,
        environment: Option<&str>,
        repository_id: Option<RepositoryId>,
    ) -> Result<()> {
        let row = self
            .store
            .find(org_id, name, environment, repository_id)
            .await?
            .ok_or(SecretsError::NotFound)?;
        self.store.delete(row.id).await?;
        let _ = self.store.audit(row.id, actor, "delete", "api").await;
        Ok(())
    }
}
