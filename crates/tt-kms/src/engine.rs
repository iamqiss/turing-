//! In-memory key ring after unseal.

use std::collections::HashMap;
use std::sync::Arc;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;
use tokio::sync::RwLock;

use crate::error::{KmsError, Result};

#[derive(Clone, Debug)]
pub struct KeyMeta {
    pub id: String,
    pub created_at_unix: i64,
}

struct KeySlot {
    meta: KeyMeta,
    kek: [u8; 32],
}

pub struct KmsEngine {
    inner: RwLock<Inner>,
}

struct Inner {
    locked: bool,
    keys: HashMap<String, KeySlot>,
    default_key_id: Option<String>,
}

impl KmsEngine {
    pub fn new_locked() -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(Inner {
                locked: true,
                keys: HashMap::new(),
                default_key_id: None,
            }),
        })
    }

    /// Load a single master key after operator unseal.
    pub async fn unlock(&self, key_id: &str, master: [u8; 32]) -> Result<()> {
        let mut g = self.inner.write().await;
        g.keys.insert(
            key_id.to_string(),
            KeySlot {
                meta: KeyMeta {
                    id: key_id.to_string(),
                    created_at_unix: chrono::Utc::now().timestamp(),
                },
                kek: master,
            },
        );
        g.default_key_id = Some(key_id.to_string());
        g.locked = false;
        Ok(())
    }

    pub async fn lock(&self) {
        let mut g = self.inner.write().await;
        for slot in g.keys.values_mut() {
            slot.kek.iter_mut().for_each(|b| *b = 0);
        }
        g.keys.clear();
        g.default_key_id = None;
        g.locked = true;
    }

    pub async fn is_locked(&self) -> bool {
        self.inner.read().await.locked
    }

    pub async fn default_key_id(&self) -> Option<String> {
        self.inner.read().await.default_key_id.clone()
    }

    pub async fn wrap(&self, key_id: Option<&str>, dek: &[u8]) -> Result<Vec<u8>> {
        if dek.len() != 32 {
            return Err(KmsError::Invalid("DEK must be 32 bytes".into()));
        }
        let g = self.inner.read().await;
        if g.locked {
            return Err(KmsError::Locked);
        }
        let id = key_id
            .map(|s| s.to_string())
            .or_else(|| g.default_key_id.clone())
            .ok_or_else(|| KmsError::Invalid("no key id".into()))?;
        let slot = g.keys.get(&id).ok_or_else(|| KmsError::UnknownKey(id))?;
        let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&slot.kek)
            .map_err(|e| KmsError::Crypto(e.to_string()))?;
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), dek)
            .map_err(|_| KmsError::Crypto("wrap".into()))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        Ok(out)
    }

    pub async fn unwrap(&self, key_id: Option<&str>, wrapped: &[u8]) -> Result<Vec<u8>> {
        if wrapped.len() < 12 + 16 {
            return Err(KmsError::Invalid("wrapped too short".into()));
        }
        let g = self.inner.read().await;
        if g.locked {
            return Err(KmsError::Locked);
        }
        let id = key_id
            .map(|s| s.to_string())
            .or_else(|| g.default_key_id.clone())
            .ok_or_else(|| KmsError::Invalid("no key id".into()))?;
        let slot = g.keys.get(&id).ok_or_else(|| KmsError::UnknownKey(id))?;
        let (n, ct) = wrapped.split_at(12);
        let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&slot.kek)
            .map_err(|e| KmsError::Crypto(e.to_string()))?;
        cipher
            .decrypt(Nonce::from_slice(n), ct)
            .map_err(|_| KmsError::Crypto("unwrap".into()))
    }
}
