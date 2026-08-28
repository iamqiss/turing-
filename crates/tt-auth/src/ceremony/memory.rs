//! In-memory ceremony store (dev / tests).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::error::Result;

use super::{
    AuthenticationCeremony, CeremonyId, CeremonyStore, RegistrationCeremony,
};

struct Entry<T> {
    data: T,
    expires_at: Instant,
}

#[derive(Default)]
struct Inner {
    registrations: HashMap<String, Entry<RegistrationCeremony>>,
    authentications: HashMap<String, Entry<AuthenticationCeremony>>,
}

#[derive(Clone, Default)]
pub struct InMemoryCeremonyStore {
    inner: Arc<Mutex<Inner>>,
}

impl InMemoryCeremonyStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn purge_expired(inner: &mut Inner) {
        let now = Instant::now();
        inner.registrations.retain(|_, e| e.expires_at > now);
        inner.authentications.retain(|_, e| e.expires_at > now);
    }
}

#[async_trait]
impl CeremonyStore for InMemoryCeremonyStore {
    async fn put_registration(
        &self,
        id: &CeremonyId,
        data: &RegistrationCeremony,
        ttl_secs: u64,
    ) -> Result<()> {
        let mut inner = self.inner.lock().await;
        Self::purge_expired(&mut inner);
        inner.registrations.insert(
            id.0.clone(),
            Entry {
                data: data.clone(),
                expires_at: Instant::now() + Duration::from_secs(ttl_secs),
            },
        );
        Ok(())
    }

    async fn take_registration(&self, id: &CeremonyId) -> Result<Option<RegistrationCeremony>> {
        let mut inner = self.inner.lock().await;
        Self::purge_expired(&mut inner);
        Ok(inner.registrations.remove(&id.0).map(|e| e.data))
    }

    async fn put_authentication(
        &self,
        id: &CeremonyId,
        data: &AuthenticationCeremony,
        ttl_secs: u64,
    ) -> Result<()> {
        let mut inner = self.inner.lock().await;
        Self::purge_expired(&mut inner);
        inner.authentications.insert(
            id.0.clone(),
            Entry {
                data: data.clone(),
                expires_at: Instant::now() + Duration::from_secs(ttl_secs),
            },
        );
        Ok(())
    }

    async fn take_authentication(&self, id: &CeremonyId) -> Result<Option<AuthenticationCeremony>> {
        let mut inner = self.inner.lock().await;
        Self::purge_expired(&mut inner);
        Ok(inner.authentications.remove(&id.0).map(|e| e.data))
    }
}
