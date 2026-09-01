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
            realtime_backend: "redis".into(),
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

    /// Parse session key from hex. Returns None if empty (dev ephemeral key).
    pub fn secrets_key_bytes(&self) -> anyhow::Result<[u8; 32]> {
        let s = if !self.secrets_key.trim().is_empty() {
            self.secrets_key.trim()
        } else {
            self.session_key.trim()
        };
        if s.is_empty() {
            // Dev fallback — deterministic but insecure; set TT_SECRETS_KEY in prod
            // Exactly 16 + 16 bytes for the [u8; 32] fallback (lab only)
            let mut out = [0u8; 32];
            out[..16].copy_from_slice(b"turing-tarpit!!!"); // 16
            out[16..].copy_from_slice(b"-secrets-key!!!!"); // 16
            return Ok(out);
        }
        let bytes = hex::decode(s).map_err(|e| anyhow::anyhow!("secrets_key hex: {e}"))?;
        if bytes.len() != 32 {
            anyhow::bail!("secrets_key must be 32 bytes hex-encoded");
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(out)
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
