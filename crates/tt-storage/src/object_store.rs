//! Object storage backend — local disk now; S3/RustFS-compatible later.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::error::{Result, StorageError};

#[async_trait]
pub trait ObjectStore: Send + Sync {
    async fn put(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Vec<u8>>;
    async fn delete(&self, key: &str) -> Result<()>;
    async fn exists(&self, key: &str) -> Result<bool>;
}

/// Filesystem object store under `root/` (mirrors key path).
pub struct LocalObjectStore {
    root: PathBuf,
}

impl LocalObjectStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_for(&self, key: &str) -> Result<PathBuf> {
        // Prevent path traversal
        if key.contains("..") || key.starts_with('/') {
            return Err(StorageError::Other(anyhow::anyhow!("invalid storage key")));
        }
        Ok(self.root.join(key))
    }
}

#[async_trait]
impl ObjectStore for LocalObjectStore {
    async fn put(&self, key: &str, bytes: &[u8], _content_type: &str) -> Result<()> {
        let path = self.path_for(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| StorageError::Other(e.into()))?;
        }
        let mut f = fs::File::create(&path)
            .await
            .map_err(|e| StorageError::Other(e.into()))?;
        f.write_all(bytes)
            .await
            .map_err(|e| StorageError::Other(e.into()))?;
        f.flush()
            .await
            .map_err(|e| StorageError::Other(e.into()))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let path = self.path_for(key)?;
        fs::read(&path)
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    StorageError::NotFound
                } else {
                    StorageError::Other(e.into())
                }
            })
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let path = self.path_for(key)?;
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StorageError::Other(e.into())),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let path = self.path_for(key)?;
        Ok(Path::new(&path).exists())
    }
}

pub fn avatar_key(principal_id: Uuid, ext: &str) -> String {
    format!("avatars/{principal_id}.{}", sanitize_ext(ext))
}

pub fn attachment_key(workspace_id: Uuid, media_id: Uuid, ext: &str) -> String {
    format!(
        "attachments/{}/{}.{}",
        workspace_id,
        media_id,
        sanitize_ext(ext)
    )
}

fn sanitize_ext(ext: &str) -> &'static str {
    match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "jpg",
        "png" => "png",
        "gif" => "gif",
        "webp" => "webp",
        "pdf" => "pdf",
        "txt" => "txt",
        "md" => "md",
        "zip" => "zip",
        "webm" => "webm",
        "mp4" => "mp4",
        "mov" => "mov",
        "svg" => "svg",
        _ => "bin",
    }
}
