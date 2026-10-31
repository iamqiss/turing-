//! Configuration loading for Turing Tarpit.
//!
//! Sources (later overrides earlier):
//! 1. `config/default.toml`
//! 2. `config/{TT_ENV}.toml` (default: development)
//! 3. Environment variables (prefixed `TT_` where applicable, plus common names)

use std::path::Path;

use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub grpc_port: u16,

    pub database_url: String,
    pub redis_url: String,

    pub meili_url: String,
    pub meili_master_key: String,

    pub realtime_backend: String,
    /// Iggy HTTP base (e.g. http://127.0.0.1:3000). Used when realtime_backend=iggy.
    pub iggy_url: String,
    pub iggy_username: String,
    pub iggy_password: String,
    pub iggy_stream: String,
    pub iggy_topic: String,

    // Auth
    pub webauthn_rp_id: String,
    pub webauthn_rp_origin: String,
    pub webauthn_rp_name: String,
    /// 32-byte key, hex-encoded. Empty = generate ephemeral (dev only).
    pub session_key: String,
    pub session_ttl_hours: i64,
    pub ceremony_ttl_secs: u64,
    pub agent_token_ttl_days: Option<i64>,

    // Object storage — local | s3 | rustfs | minio
    pub object_store: String,
    pub s3_endpoint: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_bucket: String,
    pub s3_region: String,
    /// ClamAV clamd address host:port — empty skips virus scan
    pub clamd_addr: String,

    // Git
    pub git_repos_root: String,
    pub media_root: String,

    /// ClickHouse HTTP interface, e.g. http://localhost:8123
    pub clickhouse_url: String,
    pub clickhouse_database: String,

    /// Stripe secret key (sk_...) — empty disables live API calls
    pub stripe_secret_key: String,
    /// Stripe webhook signing secret (whsec_...)
    pub stripe_webhook_secret: String,
    /// Price id that maps to seat quantity
    pub stripe_price_id: String,

    /// Comma-separated principal UUIDs allowed to register compute nodes (empty = deny all except DB table)
    pub platform_operator_ids: String,
    /// Platform superadmins (comma UUIDs). Defaults to platform_operator_ids if empty.
    pub superadmin_ids: String,

    /// 32-byte hex key for encrypting secrets at rest (agent tokens, deploy tokens).
    /// Falls back to session_key if empty. Also used as local KMS KEK when kms_provider=local.
    pub secrets_key: String,

    /// Scarlet Letter KMS: local | http | aws
    pub kms_provider: String,
    /// AWS key ARN/alias or Vault transit key name
    pub kms_key_id: String,
    /// Vault/HTTP KMS base URL
    pub kms_endpoint: String,
    /// Vault token / bearer
    pub kms_token: String,
    /// AWS region when kms_provider=aws
    pub kms_region: String,

    /// OpenAI-compatible base URL for OLMo (e.g. http://gpu-host:8000)
    pub inference_url: String,
    pub inference_model: String,
    pub inference_api_key: String,
}

/// Deterministic key for throwaway labs. It is published in this source tree, so
/// anything encrypted with it is readable by anyone. Only used when
/// `TT_ALLOW_INSECURE_DEV_KEY=1`.
pub const INSECURE_DEV_KEY: [u8; 32] = *b"turing-tarpit!!!-secrets-key!!!!";

/// `TT_ALLOW_INSECURE_DEV_KEY=1` opts in to [`INSECURE_DEV_KEY`].
pub fn insecure_dev_key_allowed() -> bool {
    std::env::var("TT_ALLOW_INSECURE_DEV_KEY").is_ok_and(|v| v == "1")
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 8080,
            grpc_port: 50051,
            database_url: "postgres://turing:turing@localhost:5432/turing_tarpit".into(),
            redis_url: "redis://localhost:6379".into(),
            meili_url: "http://localhost:7700".into(),
            meili_master_key: "turing-dev-master-key-change-me".into(),
            realtime_backend: "memory".into(),
            iggy_url: "http://127.0.0.1:3000".into(),
            iggy_username: "iggy".into(),
            iggy_password: "iggy".into(),
            iggy_stream: "tt".into(),
            iggy_topic: "events".into(),
            webauthn_rp_id: "localhost".into(),
            webauthn_rp_origin: "http://localhost:8080".into(),
            webauthn_rp_name: "Turing Tarpit".into(),
            session_key: String::new(),
            session_ttl_hours: 168,
            ceremony_ttl_secs: 300,
            agent_token_ttl_days: Some(90),
            object_store: "local".into(),
            s3_endpoint: "http://localhost:9000".into(),
            s3_access_key: "turing".into(),
            s3_secret_key: "turingsecret".into(),
            s3_bucket: "turing-tarpit".into(),
            s3_region: "us-east-1".into(),
            clamd_addr: String::new(),
            git_repos_root: "/var/lib/turing-tarpit/repos".into(),
            media_root: "/var/lib/turing-tarpit/media".into(),
            clickhouse_url: "http://localhost:8123".into(),
            clickhouse_database: "turing_tarpit".into(),
            stripe_secret_key: String::new(),
            stripe_webhook_secret: String::new(),
            stripe_price_id: String::new(),
            platform_operator_ids: String::new(),
            superadmin_ids: String::new(),
            secrets_key: String::new(),
            kms_provider: "local".into(), // lab; production: "tt"
            kms_key_id: "scarlet".into(),
            kms_endpoint: String::new(),
            kms_token: String::new(),
            kms_region: "eu-central-1".into(),
            inference_url: String::new(),
            inference_model: "olmo-3-7b".into(),
            inference_api_key: String::new(),
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let env_name = std::env::var("TT_ENV").unwrap_or_else(|_| "development".into());

        let mut figment = Figment::new().merge(Serialized::defaults(Config::default()));

        if Path::new("config/default.toml").exists() {
            figment = figment.merge(Toml::file("config/default.toml"));
        }
        let env_file = format!("config/{env_name}.toml");
        if Path::new(&env_file).exists() {
            figment = figment.merge(Toml::file(&env_file));
        }

        // Env: both TT_-prefixed and common unprefixed names used in .env.example
        figment = figment
            .merge(Env::prefixed("TT_").split("_"))
            .merge(
                Env::raw()
                    .only(&[
                        "DATABASE_URL",
                        "REDIS_URL",
                        "MEILI_URL",
                        "MEILI_MASTER_KEY",
                        "WEBAUTHN_RP_ID",
                        "WEBAUTHN_RP_ORIGIN",
                        "WEBAUTHN_RP_NAME",
                        "SESSION_KEY",
                        "SESSION_TTL_HOURS",
                        "S3_ENDPOINT",
                        "S3_ACCESS_KEY",
                        "S3_SECRET_KEY",
                        "S3_BUCKET",
                        "S3_REGION",
                        "GIT_REPOS_ROOT",
                        "REALTIME_BACKEND",
                    ])
                    .map(|k| match k.as_str() {
                        "DATABASE_URL" => "database_url".into(),
                        "REDIS_URL" => "redis_url".into(),
                        "MEILI_URL" => "meili_url".into(),
                        "MEILI_MASTER_KEY" => "meili_master_key".into(),
                        "WEBAUTHN_RP_ID" => "webauthn_rp_id".into(),
                        "WEBAUTHN_RP_ORIGIN" => "webauthn_rp_origin".into(),
                        "WEBAUTHN_RP_NAME" => "webauthn_rp_name".into(),
                        "SESSION_KEY" => "session_key".into(),
                        "SESSION_TTL_HOURS" => "session_ttl_hours".into(),
                        "OBJECT_STORE" => "object_store".into(),
                        "S3_ENDPOINT" => "s3_endpoint".into(),
                        "S3_ACCESS_KEY" => "s3_access_key".into(),
                        "S3_SECRET_KEY" => "s3_secret_key".into(),
                        "S3_BUCKET" => "s3_bucket".into(),
                        "S3_REGION" => "s3_region".into(),
                        "CLAMD_ADDR" => "clamd_addr".into(),
                        "GIT_REPOS_ROOT" => "git_repos_root".into(),
                        "MEDIA_ROOT" => "media_root".into(),
                        "CLICKHOUSE_URL" => "clickhouse_url".into(),
                        "CLICKHOUSE_DATABASE" => "clickhouse_database".into(),
                        "STRIPE_SECRET_KEY" => "stripe_secret_key".into(),
                        "STRIPE_WEBHOOK_SECRET" => "stripe_webhook_secret".into(),
                        "STRIPE_PRICE_ID" => "stripe_price_id".into(),
                        "PLATFORM_OPERATOR_IDS" => "platform_operator_ids".into(),
                        "SUPERADMIN_IDS" => "superadmin_ids".into(),
                        "SECRETS_KEY" => "secrets_key".into(),
                        "KMS_PROVIDER" => "kms_provider".into(),
                        "KMS_KEY_ID" => "kms_key_id".into(),
                        "KMS_ENDPOINT" => "kms_endpoint".into(),
                        "KMS_TOKEN" => "kms_token".into(),
                        "KMS_REGION" => "kms_region".into(),
                        "INFERENCE_URL" => "inference_url".into(),
                        "INFERENCE_MODEL" => "inference_model".into(),
                        "INFERENCE_API_KEY" => "inference_api_key".into(),
                        "REALTIME_BACKEND" => "realtime_backend".into(),
                        "IGGY_URL" => "iggy_url".into(),
                        "IGGY_USERNAME" => "iggy_username".into(),
                        "IGGY_PASSWORD" => "iggy_password".into(),
                        "IGGY_STREAM" => "iggy_stream".into(),
                        "IGGY_TOPIC" => "iggy_topic".into(),
                        other => other.into(),
                    }),
            );

        // Also map TT_HOST / TT_PORT style if prefixed split is awkward
        if let Ok(v) = std::env::var("TT_HOST") {
            figment = figment.merge(("host", v));
        }
        if let Ok(v) = std::env::var("TT_PORT") {
            if let Ok(p) = v.parse::<u16>() {
                figment = figment.merge(("port", p));
            }
        }
        if let Ok(v) = std::env::var("TT_GRPC_PORT") {
            if let Ok(p) = v.parse::<u16>() {
                figment = figment.merge(("grpc_port", p));
            }
        }

        let config: Config = figment.extract()?;
        Ok(config)
    }

    /// 32-byte secrets key from `TT_SECRETS_KEY` (falls back to `TT_SESSION_KEY`).
    ///
    /// With neither set this is an **error**, unless `TT_ALLOW_INSECURE_DEV_KEY=1`,
    /// in which case the publicly known [`INSECURE_DEV_KEY`] is returned. Never
    /// returns a guessable key silently.
    pub fn secrets_key_bytes(&self) -> anyhow::Result<[u8; 32]> {
        self.secrets_key_bytes_with(insecure_dev_key_allowed())
    }

    /// Like [`Self::secrets_key_bytes`] with the dev-key opt-in passed explicitly.
    pub fn secrets_key_bytes_with(&self, allow_insecure_dev_key: bool) -> anyhow::Result<[u8; 32]> {
        let s = if !self.secrets_key.trim().is_empty() {
            self.secrets_key.trim()
        } else {
            self.session_key.trim()
        };
        if s.is_empty() {
            if allow_insecure_dev_key {
                return Ok(INSECURE_DEV_KEY);
            }
            anyhow::bail!(
                "TT_SECRETS_KEY is not set. Generate one with `openssl rand -hex 32`. \
                 For a throwaway lab only, set TT_ALLOW_INSECURE_DEV_KEY=1 to use a \
                 publicly known key."
            );
        }
        let bytes = hex::decode(s).map_err(|e| anyhow::anyhow!("secrets_key hex: {e}"))?;
        if bytes.len() != 32 {
            anyhow::bail!("secrets_key must be 32 bytes hex-encoded");
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(out)
    }

    
    pub fn superadmin_id_list(&self) -> Vec<String> {
        let own = self
            .superadmin_ids
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if !own.is_empty() {
            own
        } else {
            self.platform_operator_id_list()
        }
    }

    pub fn is_superadmin(&self, principal_id: &str) -> bool {
        self.superadmin_id_list().iter().any(|id| id == principal_id)
    }

    pub fn platform_operator_id_list(&self) -> Vec<String> {
        self.platform_operator_ids
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn session_key_bytes(&self) -> anyhow::Result<Option<[u8; 32]>> {
        let s = self.session_key.trim();
        if s.is_empty() {
            return Ok(None);
        }
        let bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            anyhow::bail!("SESSION_KEY must be 32 bytes (64 hex chars), got {}", bytes.len());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Some(arr))
    }
}

#[cfg(test)]
mod secrets_key_tests {
    use super::*;

    fn cfg(secrets: &str, session: &str) -> Config {
        let mut c = Config::default();
        c.secrets_key = secrets.into();
        c.session_key = session.into();
        c
    }

    const HEX_A: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn dev_key_constant_is_32_bytes_and_unchanged() {
        // Existing lab databases were encrypted with exactly these bytes.
        assert_eq!(&INSECURE_DEV_KEY[..16], b"turing-tarpit!!!");
        assert_eq!(&INSECURE_DEV_KEY[16..], b"-secrets-key!!!!");
    }

    #[test]
    fn empty_key_is_an_error_by_default() {
        let err = cfg("", "").secrets_key_bytes_with(false).unwrap_err().to_string();
        assert!(err.contains("TT_SECRETS_KEY"), "{err}");
        let err = cfg("   ", "  ").secrets_key_bytes_with(false).unwrap_err().to_string();
        assert!(err.contains("TT_SECRETS_KEY"), "{err}");
    }

    #[test]
    fn dev_key_only_with_explicit_opt_in() {
        assert_eq!(cfg("", "").secrets_key_bytes_with(true).unwrap(), INSECURE_DEV_KEY);
    }

    #[test]
    fn explicit_key_wins_and_opt_in_does_not_override_it() {
        let k = cfg(HEX_A, "").secrets_key_bytes_with(true).unwrap();
        assert_eq!(k[0], 0x00);
        assert_eq!(k[31], 0x1f);
        assert_ne!(k, INSECURE_DEV_KEY);
    }

    #[test]
    fn session_key_is_used_when_secrets_key_is_empty() {
        let k = cfg("", HEX_A).secrets_key_bytes_with(false).unwrap();
        assert_eq!(k[31], 0x1f);
    }

    #[test]
    fn malformed_keys_are_errors_not_zero_keys() {
        assert!(cfg("zz", "").secrets_key_bytes_with(true).is_err());
        assert!(cfg("00ff", "").secrets_key_bytes_with(true).is_err());
        assert!(cfg(&format!("{HEX_A}00"), "").secrets_key_bytes_with(true).is_err());
    }
}
