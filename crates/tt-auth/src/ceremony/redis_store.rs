//! Redis-backed ceremony store.

use async_trait::async_trait;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use tracing::debug;

use crate::error::{AuthError, Result};

use super::{
    AuthenticationCeremony, CeremonyId, CeremonyStore, RegistrationCeremony,
};

const REG_PREFIX: &str = "tt:ceremony:reg:";
const AUTH_PREFIX: &str = "tt:ceremony:auth:";

#[derive(Clone)]
pub struct RedisCeremonyStore {
    conn: ConnectionManager,
}

impl RedisCeremonyStore {
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url)
            .map_err(|e| AuthError::Other(anyhow::anyhow!("redis client: {e}")))?;
        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| AuthError::Other(anyhow::anyhow!("redis connect: {e}")))?;
        Ok(Self { conn })
    }

    pub fn from_manager(conn: ConnectionManager) -> Self {
        Self { conn }
    }

    /// GET then DEL (one-time use). Acceptable race window for short-lived ceremonies.
    async fn take_string(&self, key: &str) -> Result<Option<String>> {
        let mut conn = self.conn.clone();
        let value: Option<String> = conn
            .get(key)
            .await
            .map_err(|e| AuthError::Other(anyhow::anyhow!("redis get: {e}")))?;
        if value.is_some() {
            let _: () = conn
                .del(key)
                .await
                .map_err(|e| AuthError::Other(anyhow::anyhow!("redis del: {e}")))?;
        }
        Ok(value)
    }
}

#[async_trait]
impl CeremonyStore for RedisCeremonyStore {
    async fn put_registration(
        &self,
        id: &CeremonyId,
        data: &RegistrationCeremony,
        ttl_secs: u64,
    ) -> Result<()> {
        let key = format!("{REG_PREFIX}{id}");
        let value = serde_json::to_string(data)
            .map_err(|e| AuthError::Other(anyhow::anyhow!("serialize reg ceremony: {e}")))?;

        let mut conn = self.conn.clone();
        conn.set_ex::<_, _, ()>(&key, value, ttl_secs)
            .await
            .map_err(|e| AuthError::Other(anyhow::anyhow!("redis set_ex: {e}")))?;

        debug!(%id, ttl_secs, "stored registration ceremony");
        Ok(())
    }

    async fn take_registration(&self, id: &CeremonyId) -> Result<Option<RegistrationCeremony>> {
        let key = format!("{REG_PREFIX}{id}");
        match self.take_string(&key).await? {
            None => Ok(None),
            Some(s) => {
                let data: RegistrationCeremony = serde_json::from_str(&s).map_err(|e| {
                    AuthError::Other(anyhow::anyhow!("deserialize reg ceremony: {e}"))
                })?;
                debug!(%id, "took registration ceremony");
                Ok(Some(data))
            }
        }
    }

    async fn put_authentication(
        &self,
        id: &CeremonyId,
        data: &AuthenticationCeremony,
        ttl_secs: u64,
    ) -> Result<()> {
        let key = format!("{AUTH_PREFIX}{id}");
        let value = serde_json::to_string(data)
            .map_err(|e| AuthError::Other(anyhow::anyhow!("serialize auth ceremony: {e}")))?;

        let mut conn = self.conn.clone();
        conn.set_ex::<_, _, ()>(&key, value, ttl_secs)
            .await
            .map_err(|e| AuthError::Other(anyhow::anyhow!("redis set_ex: {e}")))?;

        debug!(%id, ttl_secs, "stored authentication ceremony");
        Ok(())
    }

    async fn take_authentication(&self, id: &CeremonyId) -> Result<Option<AuthenticationCeremony>> {
        let key = format!("{AUTH_PREFIX}{id}");
        match self.take_string(&key).await? {
            None => Ok(None),
            Some(s) => {
                let data: AuthenticationCeremony = serde_json::from_str(&s).map_err(|e| {
                    AuthError::Other(anyhow::anyhow!("deserialize auth ceremony: {e}"))
                })?;
                debug!(%id, "took authentication ceremony");
                Ok(Some(data))
            }
        }
    }
}
