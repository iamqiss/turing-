//! Envelope encryption: random DEK per secret, KEK owned by a KeyProvider (KMS).
//!
//! Turing Tarpit stores only packed wrapped_dek + data nonce/ciphertext.
//! Plaintext exists only ephemerally in processes allowed to decrypt.
//!
//! Every wrapped DEK records the provider and key id it was wrapped under.
//! New secrets are always sealed with the **active** provider; opening picks the
//! provider the secret was sealed under, so the KEK can be rotated without losing
//! old secrets (keep the old key as a *retired* provider until everything has been
//! re-sealed).

use std::sync::Arc;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;
use tracing::warn;

use crate::error::{Result, SecretsError};
use crate::kms::{pack_wrapped, unpack_wrapped, KeyProvider};

pub struct Envelope {
    active: Arc<dyn KeyProvider>,
    retired: Vec<Arc<dyn KeyProvider>>,
}

impl Envelope {
    pub fn new(provider: Arc<dyn KeyProvider>) -> Self {
        Self {
            active: provider,
            retired: Vec::new(),
        }
    }

    /// Providers that can still *open* old secrets but are never used to seal.
    pub fn with_retired(mut self, retired: Vec<Arc<dyn KeyProvider>>) -> Self {
        self.retired = retired;
        self
    }

    pub fn provider_name(&self) -> &str {
        self.active.provider_name()
    }

    /// True when `packed_wrapped_dek` was not sealed under the active provider/key,
    /// i.e. the secret should be re-sealed before its old key is removed.
    pub fn needs_rewrap(&self, packed_wrapped_dek: &[u8]) -> bool {
        match unpack_wrapped(packed_wrapped_dek) {
            Ok((provider, key_id, _)) => {
                provider != self.active.provider_name() || key_id != self.active.key_id()
            }
            Err(_) => true,
        }
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

        let wrapped_ct = self.active.wrap_dek(&dek).await?;
        let packed = pack_wrapped(
            self.active.provider_name(),
            self.active.key_id(),
            &wrapped_ct,
        );

        dek.iter_mut().for_each(|b| *b = 0);

        Ok((packed, nonce.to_vec(), ciphertext))
    }

    /// Unwrap the DEK using the provider/key the secret was sealed under.
    async fn unwrap_dek(&self, packed_wrapped_dek: &[u8]) -> Result<Vec<u8>> {
        let (provider, key_id, wrapped_ct) = unpack_wrapped(packed_wrapped_dek)?;

        // Only providers of the same kind can possibly unwrap this blob.
        // Pre-metadata blobs unpack as ("local", "legacy").
        let candidates: Vec<&Arc<dyn KeyProvider>> = std::iter::once(&self.active)
            .chain(self.retired.iter())
            .filter(|p| p.provider_name() == provider)
            .collect();

        if candidates.is_empty() {
            return Err(SecretsError::Crypto(format!(
                "secret was sealed under provider {provider:?} (key {key_id:?}) but only {} is configured; \
                 add the old key as a retired key to read it",
                self.configured()
            )));
        }

        // Pass 1: the provider that holds exactly the recorded key id.
        let mut last_err: Option<SecretsError> = None;
        for p in &candidates {
            match p.unwrap_dek_for(&key_id, &wrapped_ct).await {
                Ok(dek) => return Ok(dek),
                Err(e) => last_err = Some(e),
            }
        }

        // Pass 2: tolerate a renamed key id when the key material is unchanged.
        // Safe because unwrapping is authenticated: a wrong key fails, it never
        // yields a wrong DEK.
        for p in candidates.iter().filter(|p| p.key_id() != key_id) {
            if let Ok(dek) = p.unwrap_dek(&wrapped_ct).await {
                warn!(
                    sealed_key_id = %key_id,
                    provider_key_id = %p.key_id(),
                    "secret opened under a different key id label than it was sealed with"
                );
                return Ok(dek);
            }
        }

        Err(SecretsError::Crypto(format!(
            "cannot unwrap DEK sealed under {provider}/{key_id}; configured: {}; last error: {}",
            self.configured(),
            last_err.map(|e| e.to_string()).unwrap_or_else(|| "none".into())
        )))
    }

    fn configured(&self) -> String {
        std::iter::once(&self.active)
            .map(|p| format!("{}/{} (active)", p.provider_name(), p.key_id()))
            .chain(
                self.retired
                    .iter()
                    .map(|p| format!("{}/{} (retired)", p.provider_name(), p.key_id())),
            )
            .collect::<Vec<_>>()
            .join(", ")
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
        let dek = self.unwrap_dek(packed_wrapped_dek).await?;
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;

    use super::*;
    use crate::kms::LocalKeyProvider;

    fn local(byte: u8, id: &str) -> Arc<dyn KeyProvider> {
        Arc::new(LocalKeyProvider {
            key: [byte; 32],
            key_id: id.into(),
        })
    }

    /// Stands in for tt-kms: one endpoint, several keys, one of them active.
    struct FakeTt {
        active: String,
        keys: HashMap<String, LocalKeyProvider>,
    }

    impl FakeTt {
        fn new(active: &str, keys: &[(&str, u8)]) -> Arc<dyn KeyProvider> {
            Arc::new(Self {
                active: active.into(),
                keys: keys
                    .iter()
                    .map(|(id, b)| {
                        (
                            id.to_string(),
                            LocalKeyProvider {
                                key: [*b; 32],
                                key_id: id.to_string(),
                            },
                        )
                    })
                    .collect(),
            })
        }
    }

    #[async_trait]
    impl KeyProvider for FakeTt {
        fn provider_name(&self) -> &str {
            "tt"
        }
        fn key_id(&self) -> &str {
            &self.active
        }
        async fn wrap_dek(&self, dek: &[u8]) -> Result<Vec<u8>> {
            self.keys[&self.active].wrap_dek(dek).await
        }
        async fn unwrap_dek(&self, ct: &[u8]) -> Result<Vec<u8>> {
            self.keys[&self.active].unwrap_dek(ct).await
        }
        async fn unwrap_dek_for(&self, key_id: &str, ct: &[u8]) -> Result<Vec<u8>> {
            match self.keys.get(key_id) {
                Some(k) => k.unwrap_dek(ct).await,
                None => Err(SecretsError::Crypto(format!("unknown key {key_id}"))),
            }
        }
    }

    /// A provider that must never be consulted.
    struct MustNotBeCalled(&'static str);

    #[async_trait]
    impl KeyProvider for MustNotBeCalled {
        fn provider_name(&self) -> &str {
            self.0
        }
        fn key_id(&self) -> &str {
            "x"
        }
        async fn wrap_dek(&self, _: &[u8]) -> Result<Vec<u8>> {
            panic!("wrap must not be called")
        }
        async fn unwrap_dek(&self, _: &[u8]) -> Result<Vec<u8>> {
            panic!("unwrap must not be called")
        }
    }

    async fn sealed_under(p: Arc<dyn KeyProvider>, pt: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        Envelope::new(p).seal(pt).await.unwrap()
    }

    #[tokio::test]
    async fn roundtrip_with_a_single_provider() {
        let env = Envelope::new(local(1, "k1"));
        let (w, n, c) = env.seal(b"s3cret").await.unwrap();
        assert_eq!(env.open(&w, &n, &c).await.unwrap(), b"s3cret");
        assert!(!env.needs_rewrap(&w));
    }

    #[tokio::test]
    async fn rotation_without_the_old_key_fails_loudly_and_says_why() {
        let (w, n, c) = sealed_under(local(1, "k1"), b"old").await;
        let env = Envelope::new(local(2, "k2"));
        let err = env.open(&w, &n, &c).await.unwrap_err().to_string();
        assert!(err.contains("local/k1"), "{err}");
        assert!(err.contains("local/k2 (active)"), "{err}");
    }

    #[tokio::test]
    async fn retired_key_keeps_old_secrets_readable_and_new_seals_use_active() {
        let (w_old, n_old, c_old) = sealed_under(local(1, "k1"), b"old").await;
        let env = Envelope::new(local(2, "k2")).with_retired(vec![local(1, "k1")]);

        assert_eq!(env.open(&w_old, &n_old, &c_old).await.unwrap(), b"old");
        assert!(env.needs_rewrap(&w_old));

        let (w_new, n_new, c_new) = env.seal(b"new").await.unwrap();
        assert_eq!(env.open(&w_new, &n_new, &c_new).await.unwrap(), b"new");
        assert!(!env.needs_rewrap(&w_new));
        let (prov, kid, _) = unpack_wrapped(&w_new).unwrap();
        assert_eq!((prov.as_str(), kid.as_str()), ("local", "k2"));
    }

    #[tokio::test]
    async fn two_retired_keys_each_open_their_own_secrets() {
        let a = sealed_under(local(1, "k1"), b"a").await;
        let b = sealed_under(local(2, "k2"), b"b").await;
        let env = Envelope::new(local(3, "k3")).with_retired(vec![local(2, "k2"), local(1, "k1")]);
        assert_eq!(env.open(&a.0, &a.1, &a.2).await.unwrap(), b"a");
        assert_eq!(env.open(&b.0, &b.1, &b.2).await.unwrap(), b"b");
    }

    #[tokio::test]
    async fn renamed_key_id_with_same_key_material_still_opens() {
        let (w, n, c) = sealed_under(local(7, "old-label"), b"v").await;
        let env = Envelope::new(local(7, "new-label"));
        assert_eq!(env.open(&w, &n, &c).await.unwrap(), b"v");
        assert!(env.needs_rewrap(&w)); // label differs, so it will be re-sealed
    }

    #[tokio::test]
    async fn legacy_blobs_without_metadata_open_via_the_local_provider() {
        // pre-metadata format: raw wrapped DEK, no magic header
        let kek = local(5, "whatever");
        let dek = [9u8; 32];
        let wrapped = kek.wrap_dek(&dek).await.unwrap();
        let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&dek).unwrap();
        let nonce = [3u8; 12];
        let ct = cipher.encrypt(Nonce::from_slice(&nonce), b"legacy".as_slice()).unwrap();

        let env = Envelope::new(kek);
        assert_eq!(env.open(&wrapped, &nonce, &ct).await.unwrap(), b"legacy");
        assert!(env.needs_rewrap(&wrapped));
    }

    #[tokio::test]
    async fn provider_kind_mismatch_is_refused_without_calling_the_wrong_backend() {
        let (w, n, c) = sealed_under(local(1, "k1"), b"x").await;
        // active is a "tt" backend that panics if touched
        let env = Envelope::new(Arc::new(MustNotBeCalled("tt")));
        let err = env.open(&w, &n, &c).await.unwrap_err().to_string();
        assert!(err.contains("provider \"local\""), "{err}");
        assert!(err.contains("retired"), "{err}");
    }

    #[tokio::test]
    async fn migrating_from_local_to_tt_keeps_local_secrets_readable() {
        let (w, n, c) = sealed_under(local(1, "k1"), b"from-local").await;
        let env = Envelope::new(FakeTt::new("t1", &[("t1", 9)])).with_retired(vec![local(1, "k1")]);
        assert_eq!(env.open(&w, &n, &c).await.unwrap(), b"from-local");
        // and new seals go to tt
        let (w2, n2, c2) = env.seal(b"to-tt").await.unwrap();
        assert_eq!(unpack_wrapped(&w2).unwrap().0, "tt");
        assert_eq!(env.open(&w2, &n2, &c2).await.unwrap(), b"to-tt");
    }

    #[tokio::test]
    async fn tt_kms_rotation_uses_the_recorded_key_id() {
        // Server holds both keys; the control plane is switched to v2.
        let before = FakeTt::new("v1", &[("v1", 1), ("v2", 2)]);
        let (w, n, c) = sealed_under(before, b"sealed-under-v1").await;

        let after = FakeTt::new("v2", &[("v1", 1), ("v2", 2)]);
        let env = Envelope::new(after);
        assert_eq!(env.open(&w, &n, &c).await.unwrap(), b"sealed-under-v1");
        assert!(env.needs_rewrap(&w));
    }

    #[tokio::test]
    async fn tt_kms_rotation_fails_clearly_when_server_lacks_the_old_key() {
        let (w, n, c) = sealed_under(FakeTt::new("v1", &[("v1", 1)]), b"x").await;
        let env = Envelope::new(FakeTt::new("v2", &[("v2", 2)]));
        let err = env.open(&w, &n, &c).await.unwrap_err().to_string();
        assert!(err.contains("tt/v1"), "{err}");
        assert!(err.contains("unknown key v1"), "{err}");
    }

    #[tokio::test]
    async fn tampered_ciphertext_nonce_and_wrapped_dek_are_rejected() {
        let env = Envelope::new(local(1, "k1"));
        let (w, n, c) = env.seal(b"value").await.unwrap();

        let mut bad_c = c.clone();
        bad_c[0] ^= 1;
        assert!(env.open(&w, &n, &bad_c).await.is_err());

        let mut bad_n = n.clone();
        bad_n[0] ^= 1;
        assert!(env.open(&w, &bad_n, &c).await.is_err());

        let mut bad_w = w.clone();
        let last = bad_w.len() - 1;
        bad_w[last] ^= 1;
        assert!(env.open(&bad_w, &n, &c).await.is_err());

        assert!(env.open(&w, &n[..11], &c).await.is_err());
        assert!(env.open(&[], &n, &c).await.is_err());
    }

    #[tokio::test]
    async fn needs_rewrap_is_true_for_garbage() {
        let env = Envelope::new(local(1, "k1"));
        assert!(env.needs_rewrap(b"TTK1")); // magic but truncated
    }
}
