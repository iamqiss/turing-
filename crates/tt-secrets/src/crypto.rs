//! Envelope encryption: random DEK per secret, KEK owned by a KeyProvider (KMS).
//!
//! Turing Tarpit stores only packed wrapped_dek + data nonce/ciphertext.
//! Plaintext exists only ephemerally in processes allowed to decrypt.

use std::sync::Arc;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;

use crate::error::{Result, SecretsError};
use crate::kms::{pack_wrapped, unpack_wrapped, KeyProvider};

pub struct Envelope {
    provider: Arc<dyn KeyProvider>,
}

impl Envelope {
    pub fn new(provider: Arc<dyn KeyProvider>) -> Self {
        Self { provider }
    }

    pub fn provider_name(&self) -> &str {
        self.provider.provider_name()
    }

    /// Encrypt plaintext → (packed_wrapped_dek, data_nonce, ciphertext)
    pub async fn seal(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
        let mut dek = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut dek);

        let data_cipher = <Aes256Gcm as KeyInit>::new_from_slice(&dek)
            .map_err(|e| SecretsError::Crypto(e.to_string()))?;
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ciphertext = data_cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| SecretsError::Crypto("seal data".into()))?;

        let wrapped_ct = self.provider.wrap_dek(&dek).await?;
        let packed = pack_wrapped(
            self.provider.provider_name(),
            self.provider.key_id(),
            &wrapped_ct,
        );

        dek.iter_mut().for_each(|b| *b = 0);

        Ok((packed, nonce.to_vec(), ciphertext))
    }

    pub async fn open(
        &self,
        packed_wrapped_dek: &[u8],
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>> {
        if nonce.len() != 12 {
            return Err(SecretsError::Crypto("bad nonce".into()));
        }
        let (_provider, _key_id, wrapped_ct) = unpack_wrapped(packed_wrapped_dek)?;
        // Future: select provider by _provider / _key_id for multi-key rotation.
        // Today the process is configured with the active provider.
        let dek = self.provider.unwrap_dek(&wrapped_ct).await?;
        if dek.len() != 32 {
            return Err(SecretsError::Crypto("bad dek length".into()));
        }
        let data_cipher = <Aes256Gcm as KeyInit>::new_from_slice(&dek)
            .map_err(|e| SecretsError::Crypto(e.to_string()))?;
        data_cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| SecretsError::Crypto("open data".into()))
    }
}
