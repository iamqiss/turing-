use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tt_core::domain::{SecretRecord, SecretValueKind};
use tt_core::ids::{OrgId, PrincipalId, RepositoryId};
use uuid::Uuid;

use crate::error::{Result, SecretsError};

pub struct SecretRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub environment: Option<String>,
    pub repository_id: Option<Uuid>,
    pub name: String,
    pub value_kind: String,
    pub description: Option<String>,
    pub wrapped_dek: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub version: i32,
    pub created_by: Uuid,
    pub updated_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_rotated_at: Option<DateTime<Utc>>,
}

impl SecretRow {
    pub fn to_record(&self) -> SecretRecord {
        SecretRecord {
            id: self.id,
            org_id: OrgId::from_uuid(self.org_id),
            environment: self.environment.clone(),
            repository_id: self.repository_id.map(RepositoryId::from_uuid),
            name: self.name.clone(),
            value_kind: SecretValueKind::parse(&self.value_kind).unwrap_or(SecretValueKind::String),
            description: self.description.clone(),
            version: self.version.max(0) as u32,
            created_by: PrincipalId::from_uuid(self.created_by),
            updated_by: PrincipalId::from_uuid(self.updated_by),
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_rotated_at: self.last_rotated_at,
        }
    }
}

#[async_trait]
pub trait SecretsStore: Send + Sync {
    async fn upsert(&self, row: &SecretRow) -> Result<()>;
    async fn get_by_id(&self, id: Uuid) -> Result<Option<SecretRow>>;
    async fn find(
        &self,
        org_id: OrgId,
        name: &str,
        environment: Option<&str>,
        repository_id: Option<RepositoryId>,
    ) -> Result<Option<SecretRow>>;
    async fn list_meta(
        &self,
        org_id: OrgId,
        environment: Option<&str>,
        repository_id: Option<RepositoryId>,
    ) -> Result<Vec<SecretRow>>;
    async fn delete(&self, id: Uuid) -> Result<()>;
    async fn audit(
        &self,
        secret_id: Uuid,
        principal_id: PrincipalId,
        action: &str,
        client_kind: &str,
    ) -> Result<()>;
}

pub struct PostgresSecretsStore {
    pool: PgPool,
}

impl PostgresSecretsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SecretsStore for PostgresSecretsStore {
    async fn upsert(&self, row: &SecretRow) -> Result<()> {
        // Delete prior row in same scope then insert (simpler than complex ON CONFLICT)
        if let Some(ref env) = row.environment {
            sqlx::query(
                "DELETE FROM scarlet_secrets WHERE org_id = $1 AND environment = $2 AND name = $3 AND repository_id IS NULL",
            )
            .bind(row.org_id)
            .bind(env)
            .bind(&row.name)
            .execute(&self.pool)
            .await
            .map_err(|e| SecretsError::Other(e.into()))?;
        } else if let Some(rid) = row.repository_id {
            sqlx::query(
                "DELETE FROM scarlet_secrets WHERE repository_id = $1 AND name = $2",
            )
            .bind(rid)
            .bind(&row.name)
            .execute(&self.pool)
            .await
            .map_err(|e| SecretsError::Other(e.into()))?;
        } else {
            sqlx::query(
                "DELETE FROM scarlet_secrets WHERE org_id = $1 AND name = $2 AND environment IS NULL AND repository_id IS NULL",
            )
            .bind(row.org_id)
            .bind(&row.name)
            .execute(&self.pool)
            .await
            .map_err(|e| SecretsError::Other(e.into()))?;
        }

        sqlx::query(
            r#"
            INSERT INTO scarlet_secrets (
                id, org_id, environment, repository_id, name, value_kind, description,
                wrapped_dek, ciphertext, nonce, version,
                created_by, updated_by, created_at, updated_at, last_rotated_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
            "#,
        )
        .bind(row.id)
        .bind(row.org_id)
        .bind(&row.environment)
        .bind(row.repository_id)
        .bind(&row.name)
        .bind(&row.value_kind)
        .bind(&row.description)
        .bind(&row.wrapped_dek)
        .bind(&row.ciphertext)
        .bind(&row.nonce)
        .bind(row.version)
        .bind(row.created_by)
        .bind(row.updated_by)
        .bind(row.created_at)
        .bind(row.updated_at)
        .bind(row.last_rotated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| SecretsError::Other(e.into()))?;
        Ok(())
    }

    async fn get_by_id(&self, id: Uuid) -> Result<Option<SecretRow>> {
        self.fetch_one("SELECT * FROM scarlet_secrets WHERE id = $1", id).await
    }

    async fn find(
        &self,
        org_id: OrgId,
        name: &str,
        environment: Option<&str>,
        repository_id: Option<RepositoryId>,
    ) -> Result<Option<SecretRow>> {
        if let Some(rid) = repository_id {
            let row = sqlx::query_as::<_, SecretSql>(
                "SELECT * FROM scarlet_secrets WHERE repository_id = $1 AND name = $2",
            )
            .bind(rid.as_uuid())
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| SecretsError::Other(e.into()))?;
            return Ok(row.map(Into::into));
        }
        if let Some(env) = environment {
            let row = sqlx::query_as::<_, SecretSql>(
                "SELECT * FROM scarlet_secrets WHERE org_id = $1 AND environment = $2 AND name = $3 AND repository_id IS NULL",
            )
            .bind(org_id.as_uuid())
            .bind(env)
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| SecretsError::Other(e.into()))?;
            return Ok(row.map(Into::into));
        }
        let row = sqlx::query_as::<_, SecretSql>(
            "SELECT * FROM scarlet_secrets WHERE org_id = $1 AND name = $2 AND environment IS NULL AND repository_id IS NULL",
        )
        .bind(org_id.as_uuid())
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SecretsError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn list_meta(
        &self,
        org_id: OrgId,
        environment: Option<&str>,
        repository_id: Option<RepositoryId>,
    ) -> Result<Vec<SecretRow>> {
        let rows = if let Some(rid) = repository_id {
            sqlx::query_as::<_, SecretSql>(
                "SELECT * FROM scarlet_secrets WHERE repository_id = $1 ORDER BY name",
            )
            .bind(rid.as_uuid())
            .fetch_all(&self.pool)
            .await
        } else if let Some(env) = environment {
            sqlx::query_as::<_, SecretSql>(
                "SELECT * FROM scarlet_secrets WHERE org_id = $1 AND (environment = $2 OR environment IS NULL) AND repository_id IS NULL ORDER BY name",
            )
            .bind(org_id.as_uuid())
            .bind(env)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, SecretSql>(
                "SELECT * FROM scarlet_secrets WHERE org_id = $1 ORDER BY name",
            )
            .bind(org_id.as_uuid())
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| SecretsError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM scarlet_secrets WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| SecretsError::Other(e.into()))?;
        Ok(())
    }

    async fn audit(
        &self,
        secret_id: Uuid,
        principal_id: PrincipalId,
        action: &str,
        client_kind: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO scarlet_access_audit (id, secret_id, principal_id, action, client_kind, created_at)
            VALUES ($1,$2,$3,$4,$5,NOW())
            "#,
        )
        .bind(Uuid::now_v7())
        .bind(secret_id)
        .bind(principal_id.as_uuid())
        .bind(action)
        .bind(client_kind)
        .execute(&self.pool)
        .await
        .map_err(|e| SecretsError::Other(e.into()))?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct SecretSql {
    id: Uuid,
    org_id: Uuid,
    environment: Option<String>,
    repository_id: Option<Uuid>,
    name: String,
    value_kind: String,
    description: Option<String>,
    wrapped_dek: Vec<u8>,
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    version: i32,
    created_by: Uuid,
    updated_by: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_rotated_at: Option<DateTime<Utc>>,
}

impl From<SecretSql> for SecretRow {
    fn from(r: SecretSql) -> Self {
        Self {
            id: r.id,
            org_id: r.org_id,
            environment: r.environment,
            repository_id: r.repository_id,
            name: r.name,
            value_kind: r.value_kind,
            description: r.description,
            wrapped_dek: r.wrapped_dek,
            ciphertext: r.ciphertext,
            nonce: r.nonce,
            version: r.version,
            created_by: r.created_by,
            updated_by: r.updated_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
            last_rotated_at: r.last_rotated_at,
        }
    }
}

impl PostgresSecretsStore {
    async fn fetch_one(&self, sql: &str, id: Uuid) -> Result<Option<SecretRow>> {
        let row = sqlx::query_as::<_, SecretSql>(sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| SecretsError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }
}
