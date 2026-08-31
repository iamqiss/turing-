//! Media service — avatars + channel attachments.

use std::sync::Arc;

use chrono::Utc;
use sqlx::PgPool;
use tt_core::ids::PrincipalId;
use uuid::Uuid;

use crate::error::{Result, StorageError};
use crate::object_store::{attachment_key, avatar_key, ObjectStore};

const MAX_AVATAR_BYTES: u64 = 2 * 1024 * 1024; // 2 MiB
const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024; // 25 MiB

const AVATAR_TYPES: &[&str] = &["image/jpeg", "image/png", "image/webp", "image/gif"];
const ATTACHMENT_TYPES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/webp",
    "image/gif",
    "image/svg+xml",
    "application/pdf",
    "text/plain",
    "text/markdown",
    "application/zip",
    "video/mp4",
    "video/webm",
];

#[derive(Debug, Clone, serde::Serialize)]
pub struct MediaObject {
    pub id: Uuid,
    pub owner_id: PrincipalId,
    pub kind: String,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub storage_key: String,
    pub created_at: chrono::DateTime<Utc>,
}

pub struct MediaService {
    pool: PgPool,
    store: Arc<dyn ObjectStore>,
    /// Public base for avatar URLs served by API, e.g. https://tt.example/api/media
    public_base: String,
}

impl MediaService {
    pub fn new(pool: PgPool, store: Arc<dyn ObjectStore>, public_base: impl Into<String>) -> Self {
        Self {
            pool,
            store,
            public_base: public_base.into().trim_end_matches('/').to_string(),
        }
    }

    pub fn public_url(&self, media_id: Uuid) -> String {
        format!("{}/{}", self.public_base, media_id)
    }

    fn ext_from_name_or_type(filename: &str, content_type: &str) -> &'static str {
        if let Some(ext) = filename.rsplit('.').next() {
            if ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                return match ext.to_ascii_lowercase().as_str() {
                    "jpg" | "jpeg" => "jpg",
                    "png" => "png",
                    "gif" => "gif",
                    "webp" => "webp",
                    "pdf" => "pdf",
                    "txt" => "txt",
                    "md" => "md",
                    "zip" => "zip",
                    "mp4" => "mp4",
                    "webm" => "webm",
                    "svg" => "svg",
                    _ => "bin",
                };
            }
        }
        match content_type {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "application/pdf" => "pdf",
            "video/mp4" => "mp4",
            "video/webm" => "webm",
            _ => "bin",
        }
    }

    pub async fn upload_avatar(
        &self,
        owner_id: PrincipalId,
        filename: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<MediaObject> {
        if bytes.len() as u64 > MAX_AVATAR_BYTES {
            return Err(StorageError::Other(anyhow::anyhow!("avatar too large (max 2MiB)")));
        }
        if !AVATAR_TYPES.contains(&content_type) {
            return Err(StorageError::Other(anyhow::anyhow!(
                "avatar must be jpeg/png/webp/gif"
            )));
        }
        let id = Uuid::now_v7();
        let ext = Self::ext_from_name_or_type(filename, content_type);
        let key = avatar_key(*owner_id.as_uuid(), ext);
        self.store.put(&key, bytes, content_type).await?;

        // Upsert media row + set principals.avatar_url
        sqlx::query("DELETE FROM media_objects WHERE owner_id = $1 AND kind = 'avatar'")
            .bind(owner_id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Other(e.into()))?;

        let now = Utc::now();
        sqlx::query(
            r#"
            INSERT INTO media_objects (id, owner_id, kind, filename, content_type, size_bytes, storage_key, created_at)
            VALUES ($1,$2,'avatar',$3,$4,$5,$6,$7)
            "#,
        )
        .bind(id)
        .bind(owner_id.as_uuid())
        .bind(filename)
        .bind(content_type)
        .bind(bytes.len() as i64)
        .bind(&key)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.into()))?;

        let url = self.public_url(id);
        sqlx::query("UPDATE principals SET avatar_url = $2, updated_at = NOW() WHERE id = $1")
            .bind(owner_id.as_uuid())
            .bind(&url)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Other(e.into()))?;

        Ok(MediaObject {
            id,
            owner_id,
            kind: "avatar".into(),
            filename: filename.into(),
            content_type: content_type.into(),
            size_bytes: bytes.len() as u64,
            storage_key: key,
            created_at: now,
        })
    }

    pub async fn upload_attachment(
        &self,
        owner_id: PrincipalId,
        workspace_id: Uuid,
        filename: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<MediaObject> {
        if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
            return Err(StorageError::Other(anyhow::anyhow!(
                "attachment too large (max 25MiB)"
            )));
        }
        if !ATTACHMENT_TYPES.iter().any(|t| *t == content_type) {
            return Err(StorageError::Other(anyhow::anyhow!(
                "content type not allowed"
            )));
        }
        let id = Uuid::now_v7();
        let ext = Self::ext_from_name_or_type(filename, content_type);
        let key = attachment_key(workspace_id, id, ext);
        self.store.put(&key, bytes, content_type).await?;
        let now = Utc::now();
        sqlx::query(
            r#"
            INSERT INTO media_objects (id, owner_id, kind, filename, content_type, size_bytes, storage_key, created_at)
            VALUES ($1,$2,'attachment',$3,$4,$5,$6,$7)
            "#,
        )
        .bind(id)
        .bind(owner_id.as_uuid())
        .bind(filename)
        .bind(content_type)
        .bind(bytes.len() as i64)
        .bind(&key)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.into()))?;

        Ok(MediaObject {
            id,
            owner_id,
            kind: "attachment".into(),
            filename: filename.into(),
            content_type: content_type.into(),
            size_bytes: bytes.len() as u64,
            storage_key: key,
            created_at: now,
        })
    }

    pub async fn get_meta(&self, id: Uuid) -> Result<MediaObject> {
        let row: Option<(Uuid, Uuid, String, String, String, i64, String, chrono::DateTime<Utc>)> =
            sqlx::query_as(
                r#"
                SELECT id, owner_id, kind, filename, content_type, size_bytes, storage_key, created_at
                FROM media_objects WHERE id = $1
                "#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StorageError::Other(e.into()))?;
        let (id, owner, kind, filename, ct, size, key, created) =
            row.ok_or(StorageError::NotFound)?;
        Ok(MediaObject {
            id,
            owner_id: PrincipalId::from_uuid(owner),
            kind,
            filename,
            content_type: ct,
            size_bytes: size.max(0) as u64,
            storage_key: key,
            created_at: created,
        })
    }

    pub async fn get_bytes(&self, id: Uuid) -> Result<(MediaObject, Vec<u8>)> {
        let meta = self.get_meta(id).await?;
        let bytes = self.store.get(&meta.storage_key).await?;
        Ok((meta, bytes))
    }

    pub async fn link_to_message(&self, message_id: Uuid, media_ids: &[Uuid]) -> Result<()> {
        for mid in media_ids {
            sqlx::query(
                "INSERT INTO message_attachments (message_id, media_id) VALUES ($1,$2) ON CONFLICT DO NOTHING",
            )
            .bind(message_id)
            .bind(mid)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Other(e.into()))?;
        }
        Ok(())
    }

    pub async fn list_for_message(&self, message_id: Uuid) -> Result<Vec<MediaObject>> {
        let rows: Vec<(Uuid, Uuid, String, String, String, i64, String, chrono::DateTime<Utc>)> =
            sqlx::query_as(
                r#"
                SELECT m.id, m.owner_id, m.kind, m.filename, m.content_type, m.size_bytes, m.storage_key, m.created_at
                FROM media_objects m
                INNER JOIN message_attachments a ON a.media_id = m.id
                WHERE a.message_id = $1
                ORDER BY m.created_at
                "#,
            )
            .bind(message_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::Other(e.into()))?;
        Ok(rows
            .into_iter()
            .map(|(id, owner, kind, filename, ct, size, key, created)| MediaObject {
                id,
                owner_id: PrincipalId::from_uuid(owner),
                kind,
                filename,
                content_type: ct,
                size_bytes: size.max(0) as u64,
                storage_key: key,
                created_at: created,
            })
            .collect())
    }
}
