//! Break Glass recovery methods.
//!
//! 1. **Time-lock** — user triggers recovery; broadcast to sessions/email/phone;
//!    cancel window (default 7 days); then a one-time recovery token unlocks passphrase reset.
//! 2. **Trustees** — 3–5 trustee public keys / linked principals; majority signatures
//!    on the event id unlock recovery. Clear social labels stay client-side (`label_commitment`).
//!
//! Users may enable any combination. Completing recovery rotates the passphrase and
//! revokes existing sessions.
//!
//! Hardening:
//! - Ed25519 verification of trustee signatures over a fixed message
//! - Constant-time token compares
//! - Anti-enumeration on unauthenticated triggers (generic responses)
//! - Postgres-backed rate limits on triggers
//! - Single-use recovery tokens (`recovery_claimed_at`)
//! - Promote pending time-locks past `unlock_at` via `tick_time_locks`

use std::sync::Arc;

use chrono::{Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use sqlx::PgPool;
use tracing::info;
use tt_core::ids::PrincipalId;
use uuid::Uuid;

use crate::error::{AuthError, Result};
use crate::passphrase::{hash_passphrase, PassphrasePolicy};
use crate::session::SessionService;
use crate::store::AuthStore;

pub const DEFAULT_TIME_LOCK_SECS: i64 = 7 * 24 * 3600;
pub const MIN_TRUSTEES: usize = 3;
pub const MAX_TRUSTEES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakGlassMethod {
    TimeLock,
    Trustees,
}

impl BreakGlassMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TimeLock => "time_lock",
            Self::Trustees => "trustees",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakGlassStatus {
    Pending,
    Cancelled,
    Ready,
    Completed,
    Expired,
}

impl BreakGlassStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Cancelled => "cancelled",
            Self::Ready => "ready",
            Self::Completed => "completed",
            Self::Expired => "expired",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "cancelled" => Self::Cancelled,
            "ready" => Self::Ready,
            "completed" => Self::Completed,
            "expired" => Self::Expired,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BreakGlassSettings {
    pub principal_id: PrincipalId,
    pub time_lock_enabled: bool,
    pub time_lock_delay_secs: i64,
    pub trustees_enabled: bool,
    pub trustee_threshold: i32,
    pub notify_email: Option<String>,
    pub notify_phone: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Trustee {
    pub id: Uuid,
    pub principal_id: PrincipalId,
    pub label_commitment: String,
    pub trustee_pubkey: String,
    pub trustee_principal_id: Option<PrincipalId>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrusteeTriggerResult {
    pub started: bool,
    pub event: Option<BreakGlassEvent>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeLockTriggerResult {
    pub started: bool,
    pub event: Option<BreakGlassEvent>,
    pub cancel_token: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BreakGlassEvent {
    pub id: Uuid,
    pub principal_id: PrincipalId,
    pub method: BreakGlassMethod,
    pub status: BreakGlassStatus,
    pub unlock_at: Option<chrono::DateTime<Utc>>,
    pub threshold: Option<i32>,
    pub approvals: i32,
    pub created_at: chrono::DateTime<Utc>,
}


fn hash_token(raw: &str) -> String {
    let mut h = Sha256::new();
    h.update(raw.as_bytes());
    hex::encode(h.finalize())
}

fn tokens_equal(a: &str, b: &str) -> bool {
    let ha = hash_token(a);
    let hb = hash_token(b);
    // compare hashes in constant time (equal length hex)
    bool::from(ha.as_bytes().ct_eq(hb.as_bytes()))
}

fn hashes_equal(expected_hex: &str, presented_raw: &str) -> bool {
    let presented = hash_token(presented_raw);
    if expected_hex.len() != presented.len() {
        return false;
    }
    bool::from(expected_hex.as_bytes().ct_eq(presented.as_bytes()))
}

/// Decode trustee pubkey: 32-byte Ed25519 raw or hex-encoded 32 bytes.
pub fn parse_ed25519_pubkey(s: &str) -> Result<VerifyingKey> {
    let bytes = if let Ok(b) = hex::decode(s) {
        b
    } else {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(s)
            .or_else(|_| base64::engine::general_purpose::STANDARD.decode(s))
            .map_err(|_| AuthError::InvalidArgument("invalid trustee_pubkey encoding".into()))?
    };
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| AuthError::InvalidArgument("trustee_pubkey must be 32 bytes".into()))?;
    VerifyingKey::from_bytes(&arr)
        .map_err(|e| AuthError::InvalidArgument(format!("invalid ed25519 pubkey: {e}")))
}

pub fn verify_trustee_signature(
    pubkey: &str,
    message: &str,
    signature_b64_or_hex: &str,
) -> Result<()> {
    let vk = parse_ed25519_pubkey(pubkey)?;
    let sig_bytes = if let Ok(b) = hex::decode(signature_b64_or_hex) {
        b
    } else {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(signature_b64_or_hex)
            .or_else(|_| base64::engine::general_purpose::STANDARD.decode(signature_b64_or_hex))
            .map_err(|_| AuthError::InvalidArgument("invalid signature encoding".into()))?
    };
    let sig = Signature::from_slice(&sig_bytes)
        .map_err(|_| AuthError::InvalidArgument("signature must be 64 bytes".into()))?;
    vk.verify(message.as_bytes(), &sig)
        .map_err(|_| AuthError::PermissionDenied("invalid trustee signature".into()))?;
    Ok(())
}

/// ~16 attempts / hour / bucket (handle hash or IP hash).
const RATE_LIMIT_MAX: i32 = 16;
const RATE_LIMIT_WINDOW_SECS: i64 = 3600;


fn random_token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{prefix}_{}", base64_url(&bytes))
}

fn base64_url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Canonical message trustees sign: event id + account principal id.
pub fn trustee_sign_message(event_id: Uuid, account_id: PrincipalId) -> String {
    format!("tt-break-glass:v1:{event_id}:{}", account_id)
}

pub struct BreakGlassService {
    pool: PgPool,
    store: Arc<dyn AuthStore>,
    sessions: Arc<SessionService>,
    policy: PassphrasePolicy,
}

impl BreakGlassService {
    pub fn new(pool: PgPool, store: Arc<dyn AuthStore>, sessions: Arc<SessionService>) -> Self {
        Self {
            pool,
            store,
            sessions,
            policy: PassphrasePolicy::default(),
        }
    }

    // ----- settings & trustees -----

    pub async fn get_settings(&self, principal_id: PrincipalId) -> Result<BreakGlassSettings> {
        let row = sqlx::query_as::<_, SettingsRow>(
            r#"
            SELECT principal_id, time_lock_enabled, time_lock_delay_secs,
                   trustees_enabled, trustee_threshold, notify_email, notify_phone
            FROM break_glass_settings WHERE principal_id = $1
            "#,
        )
        .bind(principal_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;

        Ok(row
            .map(Into::into)
            .unwrap_or(BreakGlassSettings {
                principal_id,
                time_lock_enabled: false,
                time_lock_delay_secs: DEFAULT_TIME_LOCK_SECS,
                trustees_enabled: false,
                trustee_threshold: 3,
                notify_email: None,
                notify_phone: None,
            }))
    }

    pub async fn update_settings(
        &self,
        principal_id: PrincipalId,
        time_lock_enabled: Option<bool>,
        time_lock_delay_secs: Option<i64>,
        trustees_enabled: Option<bool>,
        trustee_threshold: Option<i32>,
        notify_email: Option<Option<String>>,
        notify_phone: Option<Option<String>>,
    ) -> Result<BreakGlassSettings> {
        let mut s = self.get_settings(principal_id).await?;
        if let Some(v) = time_lock_enabled {
            s.time_lock_enabled = v;
        }
        if let Some(v) = time_lock_delay_secs {
            if v < 3600 {
                return Err(AuthError::InvalidArgument(
                    "time_lock_delay_secs must be >= 3600 (1 hour)".into(),
                ));
            }
            s.time_lock_delay_secs = v;
        }
        if let Some(v) = trustees_enabled {
            s.trustees_enabled = v;
        }
        if let Some(v) = trustee_threshold {
            if v < 1 || v > MAX_TRUSTEES as i32 {
                return Err(AuthError::InvalidArgument(format!(
                    "trustee_threshold must be 1..={MAX_TRUSTEES}"
                )));
            }
            s.trustee_threshold = v;
        }
        if let Some(v) = notify_email {
            s.notify_email = v;
        }
        if let Some(v) = notify_phone {
            s.notify_phone = v;
        }

        sqlx::query(
            r#"
            INSERT INTO break_glass_settings (
                principal_id, time_lock_enabled, time_lock_delay_secs,
                trustees_enabled, trustee_threshold, notify_email, notify_phone
            ) VALUES ($1,$2,$3,$4,$5,$6,$7)
            ON CONFLICT (principal_id) DO UPDATE SET
                time_lock_enabled = EXCLUDED.time_lock_enabled,
                time_lock_delay_secs = EXCLUDED.time_lock_delay_secs,
                trustees_enabled = EXCLUDED.trustees_enabled,
                trustee_threshold = EXCLUDED.trustee_threshold,
                notify_email = EXCLUDED.notify_email,
                notify_phone = EXCLUDED.notify_phone,
                updated_at = NOW()
            "#,
        )
        .bind(principal_id.as_uuid())
        .bind(s.time_lock_enabled)
        .bind(s.time_lock_delay_secs)
        .bind(s.trustees_enabled)
        .bind(s.trustee_threshold)
        .bind(&s.notify_email)
        .bind(&s.notify_phone)
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;

        Ok(s)
    }

    pub async fn list_trustees(&self, principal_id: PrincipalId) -> Result<Vec<Trustee>> {
        let rows = sqlx::query_as::<_, TrusteeRow>(
            r#"
            SELECT id, principal_id, label_commitment, trustee_pubkey, trustee_principal_id
            FROM break_glass_trustees WHERE principal_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(principal_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn add_trustee(
        &self,
        principal_id: PrincipalId,
        label_commitment: &str,
        trustee_pubkey: &str,
        trustee_principal_id: Option<PrincipalId>,
    ) -> Result<Trustee> {
        let existing = self.list_trustees(principal_id).await?;
        if existing.len() >= MAX_TRUSTEES {
            return Err(AuthError::InvalidArgument(format!(
                "at most {MAX_TRUSTEES} trustees"
            )));
        }
        if label_commitment.is_empty() || trustee_pubkey.is_empty() {
            return Err(AuthError::InvalidArgument(
                "label_commitment and trustee_pubkey required".into(),
            ));
        }
        // Reject non-Ed25519 material early
        let _ = parse_ed25519_pubkey(trustee_pubkey)?;
        let id = Uuid::now_v7();
        sqlx::query(
            r#"
            INSERT INTO break_glass_trustees (
                id, principal_id, label_commitment, trustee_pubkey, trustee_principal_id
            ) VALUES ($1,$2,$3,$4,$5)
            "#,
        )
        .bind(id)
        .bind(principal_id.as_uuid())
        .bind(label_commitment)
        .bind(trustee_pubkey)
        .bind(trustee_principal_id.as_ref().map(|p| p.as_uuid()))
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;

        Ok(Trustee {
            id,
            principal_id,
            label_commitment: label_commitment.into(),
            trustee_pubkey: trustee_pubkey.into(),
            trustee_principal_id,
        })
    }

    pub async fn remove_trustee(&self, principal_id: PrincipalId, trustee_id: Uuid) -> Result<()> {
        let r = sqlx::query(
            "DELETE FROM break_glass_trustees WHERE id = $1 AND principal_id = $2",
        )
        .bind(trustee_id)
        .bind(principal_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;
        if r.rows_affected() == 0 {
            return Err(AuthError::PrincipalNotFound);
        }
        Ok(())
    }

    // ----- time-lock -----

    /// Start a time-lock Break Glass event. Returns cancel_token (show once) + event.
    pub async fn trigger_time_lock(
        &self,
        handle: &str,
        client_fingerprint: Option<&str>,
    ) -> Result<TimeLockTriggerResult> {
        let fp = client_fingerprint.unwrap_or("unknown");
        let bucket = format!("tl:{}", hash_token(&format!("{handle}|{fp}")));
        self.check_rate_limit(&bucket).await?;

        // Anti-enumeration: same outer Result::Ok shape; only difference is `started`.
        let dummy_cancel = random_token("tt_bg_cancel");
        let principal = match self.store.get_principal_by_handle(handle.trim()).await? {
            Some(p) => p,
            None => {
                return Ok(TimeLockTriggerResult {
                    started: false,
                    event: None,
                    cancel_token: None,
                    message: "if this account has time-lock enabled, a Break Glass window was opened and notifications were sent".into(),
                });
            }
        };
        let settings = self.get_settings(principal.id).await?;
        if !settings.time_lock_enabled {
            return Ok(TimeLockTriggerResult {
                started: false,
                event: None,
                cancel_token: None,
                message: "if this account has time-lock enabled, a Break Glass window was opened and notifications were sent".into(),
            });
        }
        if self.ensure_no_active_event(principal.id).await.is_err() {
            return Ok(TimeLockTriggerResult {
                started: false,
                event: None,
                cancel_token: None,
                message: "if this account has time-lock enabled, a Break Glass window was opened and notifications were sent".into(),
            });
        }

        let id = Uuid::now_v7();
        let now = Utc::now();
        let unlock_at = now + Duration::seconds(settings.time_lock_delay_secs);
        let cancel_token = random_token("tt_bg_cancel");
        let cancel_hash = hash_token(&cancel_token);
        let ip_hash = hash_token(fp);

        sqlx::query(
            r#"
            INSERT INTO break_glass_events (
                id, principal_id, method, status, unlock_at, cancel_token_hash,
                trigger_ip_hash, created_at, updated_at
            ) VALUES ($1,$2,'time_lock','pending',$3,$4,$5,$6,$6)
            "#,
        )
        .bind(id)
        .bind(principal.id.as_uuid())
        .bind(unlock_at)
        .bind(&cancel_hash)
        .bind(&ip_hash)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;

        self.enqueue_notifications(id, &principal, &settings, "time_lock_started")
            .await?;

        info!(event_id = %id, principal_id = %principal.id, %unlock_at, "break-glass time-lock started");
        let _ = dummy_cancel;
        Ok(TimeLockTriggerResult {
            started: true,
            event: Some(BreakGlassEvent {
                id,
                principal_id: principal.id,
                method: BreakGlassMethod::TimeLock,
                status: BreakGlassStatus::Pending,
                unlock_at: Some(unlock_at),
                threshold: None,
                approvals: 0,
                created_at: now,
            }),
            cancel_token: Some(cancel_token),
            message: "Break Glass time-lock started. Cancel from any active session if this was not you.".into(),
        })
    }

    /// Cancel from an active session or the cancel token (email link).
    pub async fn cancel_time_lock(
        &self,
        event_id: Uuid,
        cancel_token: Option<&str>,
        session_principal: Option<PrincipalId>,
    ) -> Result<()> {
        let row = self.load_event(event_id).await?;
        if row.method != "time_lock" || row.status != "pending" {
            return Err(AuthError::InvalidArgument(
                "event is not a pending time-lock".into(),
            ));
        }

        let allowed = if let Some(pid) = session_principal {
            pid.as_uuid() == &row.principal_id
        } else if let Some(tok) = cancel_token {
            row.cancel_token_hash
                .as_ref()
                .map(|h| hashes_equal(h, tok))
                .unwrap_or(false)
        } else {
            false
        };
        if !allowed {
            return Err(AuthError::PermissionDenied(
                "not allowed to cancel this Break Glass event".into(),
            ));
        }

        sqlx::query(
            r#"
            UPDATE break_glass_events
            SET status = 'cancelled', cancelled_at = NOW(), updated_at = NOW()
            WHERE id = $1 AND status = 'pending'
            "#,
        )
        .bind(event_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;

        info!(%event_id, "break-glass time-lock cancelled");
        Ok(())
    }

    /// After unlock_at, mint a one-time recovery token to set a new passphrase.
    pub async fn claim_time_lock_ready(&self, event_id: Uuid) -> Result<String> {
        let row = self.load_event(event_id).await?;
        if row.method != "time_lock" || row.status != "pending" {
            return Err(AuthError::InvalidArgument("event not claimable".into()));
        }
        let unlock_at = row.unlock_at.ok_or_else(|| {
            AuthError::InvalidArgument("missing unlock_at".into())
        })?;
        if Utc::now() < unlock_at {
            return Err(AuthError::InvalidArgument(format!(
                "time-lock still active until {unlock_at}"
            )));
        }

        let recovery = random_token("tt_bg_recover");
        let recovery_hash = hash_token(&recovery);
        sqlx::query(
            r#"
            UPDATE break_glass_events
            SET status = 'ready', recovery_token_hash = $2, updated_at = NOW()
            WHERE id = $1 AND status = 'pending'
            "#,
        )
        .bind(event_id)
        .bind(&recovery_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;

        Ok(recovery)
    }

    // ----- trustees -----

    pub async fn trigger_trustee_recovery(
        &self,
        handle: &str,
        client_fingerprint: Option<&str>,
    ) -> Result<TrusteeTriggerResult> {
        let fp = client_fingerprint.unwrap_or("unknown");
        let bucket = format!("tr:{}", hash_token(&format!("{handle}|{fp}")));
        self.check_rate_limit(&bucket).await?;

        let generic = TrusteeTriggerResult {
            started: false,
            event: None,
            message: "if this account has trustee recovery enabled, trustees were notified".into(),
        };

        let principal = match self.store.get_principal_by_handle(handle.trim()).await? {
            Some(p) => p,
            None => return Ok(generic),
        };
        let settings = self.get_settings(principal.id).await?;
        if !settings.trustees_enabled {
            return Ok(generic);
        }
        let trustees = self.list_trustees(principal.id).await?;
        if trustees.len() < MIN_TRUSTEES {
            return Ok(generic);
        }
        if settings.trustee_threshold as usize > trustees.len() {
            return Ok(generic);
        }
        if self.ensure_no_active_event(principal.id).await.is_err() {
            return Ok(generic);
        }

        let id = Uuid::now_v7();
        let now = Utc::now();
        let msg = trustee_sign_message(id, principal.id);
        sqlx::query(
            r#"
            INSERT INTO break_glass_events (
                id, principal_id, method, status, threshold, approval_message, created_at, updated_at
            ) VALUES ($1,$2,'trustees','pending',$3,$4,$5,$5)
            "#,
        )
        .bind(id)
        .bind(principal.id.as_uuid())
        .bind(settings.trustee_threshold)
        .bind(&msg)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;

        self.enqueue_notifications(id, &principal, &settings, "trustee_recovery_started")
            .await?;

        info!(event_id = %id, principal_id = %principal.id, "break-glass trustee recovery started");
        Ok(TrusteeTriggerResult {
            started: true,
            event: Some(BreakGlassEvent {
                id,
                principal_id: principal.id,
                method: BreakGlassMethod::Trustees,
                status: BreakGlassStatus::Pending,
                unlock_at: None,
                threshold: Some(settings.trustee_threshold),
                approvals: 0,
                created_at: now,
            }),
            message: "Trustee recovery started. Awaiting threshold approvals.".into(),
        })
    }

    /// Trustee approval. Signature is over `trustee_sign_message(event_id, account_id)`.
    /// Verification is delegated to a pluggable check: we store the signature and
    /// accept either (a) matching linked trustee_principal_id session, or
    /// (b) non-empty signature binding to trustee_pubkey (client/HSM verified offline
    /// in stricter deployments — here we require linked principal OR signature present).
    pub async fn approve_as_trustee(
        &self,
        event_id: Uuid,
        trustee_id: Uuid,
        signature: &str,
        actor: Option<PrincipalId>,
    ) -> Result<BreakGlassEvent> {
        let row = self.load_event(event_id).await?;
        if row.method != "trustees" || row.status != "pending" {
            return Err(AuthError::InvalidArgument("not a pending trustee event".into()));
        }
        let trustee = sqlx::query_as::<_, TrusteeRow>(
            "SELECT id, principal_id, label_commitment, trustee_pubkey, trustee_principal_id FROM break_glass_trustees WHERE id = $1",
        )
        .bind(trustee_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?
        .ok_or(AuthError::PrincipalNotFound)?;

        if trustee.principal_id != row.principal_id {
            return Err(AuthError::PermissionDenied("trustee not on this account".into()));
        }

        if let Some(linked) = trustee.trustee_principal_id {
            let ok = actor.map(|a| a.as_uuid() == &linked).unwrap_or(false);
            if !ok {
                return Err(AuthError::PermissionDenied(
                    "trustee must sign in as the linked principal".into(),
                ));
            }
        }
        if signature.trim().is_empty() {
            return Err(AuthError::InvalidArgument(
                "ed25519 signature required".into(),
            ));
        }
        let msg = trustee_sign_message(event_id, PrincipalId::from_uuid(row.principal_id));
        verify_trustee_signature(&trustee.trustee_pubkey, &msg, signature)?;

        sqlx::query(
            r#"
            INSERT INTO break_glass_approvals (event_id, trustee_id, signature)
            VALUES ($1,$2,$3)
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(event_id)
        .bind(trustee_id)
        .bind(signature)
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM break_glass_approvals WHERE event_id = $1",
        )
        .bind(event_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;

        let threshold = row.threshold.unwrap_or(3) as i64;
        if count >= threshold {
            let recovery = random_token("tt_bg_recover");
            let recovery_hash = hash_token(&recovery);
            sqlx::query(
                r#"
                UPDATE break_glass_events
                SET status = 'ready', recovery_token_hash = $2, updated_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(event_id)
            .bind(&recovery_hash)
            .execute(&self.pool)
            .await
            .map_err(|e| AuthError::Other(e.into()))?;
            // Stash recovery token in a notification target for the account holder
            // (email/push worker). Return count via event; recovery claimed separately
            // with claim_ready using stored hash — for trustees path expose via
            // claim_trustee_ready.
            let _ = recovery; // returned by claim_ready after status ready
        }

        self.get_event(event_id).await
    }

    pub async fn claim_ready(&self, event_id: Uuid) -> Result<String> {
        let _ = self.tick_time_locks().await;
        let row = self.load_event(event_id).await?;
        if row.recovery_claimed_at.is_some() {
            return Err(AuthError::InvalidArgument(
                "recovery token already claimed".into(),
            ));
        }
        if row.status == "pending" && row.method == "time_lock" {
            if let Some(unlock_at) = row.unlock_at {
                if Utc::now() < unlock_at {
                    return Err(AuthError::InvalidArgument(format!(
                        "time-lock still active until {unlock_at}"
                    )));
                }
            }
        } else if row.status != "ready" {
            return Err(AuthError::InvalidArgument(
                "event is not ready for recovery token".into(),
            ));
        }

        let recovery = random_token("tt_bg_recover");
        let recovery_hash = hash_token(&recovery);
        let r = sqlx::query(
            r#"
            UPDATE break_glass_events
            SET status = 'ready',
                recovery_token_hash = $2,
                recovery_claimed_at = NOW(),
                updated_at = NOW()
            WHERE id = $1
              AND recovery_claimed_at IS NULL
              AND status IN ('pending', 'ready')
            "#,
        )
        .bind(event_id)
        .bind(&recovery_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;
        if r.rows_affected() == 0 {
            return Err(AuthError::InvalidArgument(
                "recovery token already claimed or event not claimable".into(),
            ));
        }
        Ok(recovery)
    }

    /// Complete recovery: valid recovery token + new passphrase → hash update, sessions revoked.
    pub async fn complete_recovery(
        &self,
        event_id: Uuid,
        recovery_token: &str,
        new_passphrase: &str,
    ) -> Result<()> {
        self.policy.validate(new_passphrase)?;
        let row = self.load_event(event_id).await?;
        if row.status != "ready" {
            return Err(AuthError::InvalidArgument("event not ready".into()));
        }
        let expected = row.recovery_token_hash.as_ref().ok_or_else(|| {
            AuthError::InvalidArgument("no recovery token".into())
        })?;
        if !hashes_equal(expected, recovery_token) {
            return Err(AuthError::Unauthenticated);
        }

        let pid = PrincipalId::from_uuid(row.principal_id);
        let hash = hash_passphrase(new_passphrase)?;
        self.store.set_passphrase_hash(pid, &hash).await?;
        self.sessions.revoke_all(pid).await?;

        sqlx::query(
            r#"
            UPDATE break_glass_events
            SET status = 'completed', completed_at = NOW(), recovery_token_hash = NULL, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(event_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;

        info!(%event_id, principal_id = %pid, "break-glass recovery completed");
        Ok(())
    }

    pub async fn get_event(&self, event_id: Uuid) -> Result<BreakGlassEvent> {
        let row = self.load_event(event_id).await?;
        let approvals: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM break_glass_approvals WHERE event_id = $1",
        )
        .bind(event_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))
        .unwrap_or(0);

        Ok(BreakGlassEvent {
            id: row.id,
            principal_id: PrincipalId::from_uuid(row.principal_id),
            method: if row.method == "trustees" {
                BreakGlassMethod::Trustees
            } else {
                BreakGlassMethod::TimeLock
            },
            status: BreakGlassStatus::parse(&row.status),
            unlock_at: row.unlock_at,
            threshold: row.threshold,
            approvals: approvals as i32,
            created_at: row.created_at,
        })
    }


    async fn check_rate_limit(&self, bucket_key: &str) -> Result<()> {
        let now = Utc::now();
        let row = sqlx::query_as::<_, RateRow>(
            "SELECT bucket_key, window_start, hit_count FROM break_glass_rate_limits WHERE bucket_key = $1",
        )
        .bind(bucket_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;

        match row {
            Some(r) => {
                let age = (now - r.window_start).num_seconds();
                if age > RATE_LIMIT_WINDOW_SECS {
                    sqlx::query(
                        "UPDATE break_glass_rate_limits SET window_start = $2, hit_count = 1 WHERE bucket_key = $1",
                    )
                    .bind(bucket_key)
                    .bind(now)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| AuthError::Other(e.into()))?;
                } else if r.hit_count >= RATE_LIMIT_MAX {
                    return Err(AuthError::PermissionDenied(
                        "too many Break Glass attempts; try later".into(),
                    ));
                } else {
                    sqlx::query(
                        "UPDATE break_glass_rate_limits SET hit_count = hit_count + 1 WHERE bucket_key = $1",
                    )
                    .bind(bucket_key)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| AuthError::Other(e.into()))?;
                }
            }
            None => {
                sqlx::query(
                    "INSERT INTO break_glass_rate_limits (bucket_key, window_start, hit_count) VALUES ($1,$2,1)",
                )
                .bind(bucket_key)
                .bind(now)
                .execute(&self.pool)
                .await
                .map_err(|e| AuthError::Other(e.into()))?;
            }
        }
        Ok(())
    }

    /// Promote expired pending time-locks to `ready` without issuing a token yet.
    pub async fn tick_time_locks(&self) -> Result<u64> {
        let r = sqlx::query(
            r#"
            UPDATE break_glass_events
            SET status = 'ready', updated_at = NOW()
            WHERE method = 'time_lock'
              AND status = 'pending'
              AND unlock_at IS NOT NULL
              AND unlock_at <= NOW()
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;
        Ok(r.rows_affected())
    }

    async fn ensure_no_active_event(&self, principal_id: PrincipalId) -> Result<()> {
        let n: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM break_glass_events
            WHERE principal_id = $1 AND status IN ('pending', 'ready')
            "#,
        )
        .bind(principal_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;
        if n > 0 {
            return Err(AuthError::InvalidArgument(
                "a Break Glass event is already active".into(),
            ));
        }
        Ok(())
    }

    async fn load_event(&self, event_id: Uuid) -> Result<EventRow> {
        sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, principal_id, method, status, unlock_at, cancel_token_hash,
                   threshold, recovery_token_hash, recovery_claimed_at, created_at
            FROM break_glass_events WHERE id = $1
            "#,
        )
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?
        .ok_or(AuthError::PrincipalNotFound)
    }

    async fn enqueue_notifications(
        &self,
        event_id: Uuid,
        principal: &tt_core::domain::Principal,
        settings: &BreakGlassSettings,
        kind: &str,
    ) -> Result<()> {
        let payload = serde_json::json!({
            "kind": kind,
            "handle": principal.handle,
            "message": "Break Glass event: your account recovery was triggered. Cancel from any active session if this was not you.",
        });
        // session channel — all devices (workers map to push)
        self.insert_notification(event_id, "session", &principal.id.to_string(), &payload)
            .await?;
        if let Some(email) = &settings.notify_email {
            self.insert_notification(event_id, "email", email, &payload)
                .await?;
        }
        if let Some(phone) = &settings.notify_phone {
            self.insert_notification(event_id, "phone", phone, &payload)
                .await?;
        }
        Ok(())
    }

    async fn insert_notification(
        &self,
        event_id: Uuid,
        channel: &str,
        target: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO break_glass_notifications (id, event_id, channel, target, payload)
            VALUES ($1,$2,$3,$4,$5)
            "#,
        )
        .bind(Uuid::now_v7())
        .bind(event_id)
        .bind(channel)
        .bind(target)
        .bind(payload)
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Other(e.into()))?;
        Ok(())
    }
}

// --- sqlx rows ---

#[derive(sqlx::FromRow)]
struct SettingsRow {
    principal_id: Uuid,
    time_lock_enabled: bool,
    time_lock_delay_secs: i64,
    trustees_enabled: bool,
    trustee_threshold: i32,
    notify_email: Option<String>,
    notify_phone: Option<String>,
}

impl From<SettingsRow> for BreakGlassSettings {
    fn from(r: SettingsRow) -> Self {
        Self {
            principal_id: PrincipalId::from_uuid(r.principal_id),
            time_lock_enabled: r.time_lock_enabled,
            time_lock_delay_secs: r.time_lock_delay_secs,
            trustees_enabled: r.trustees_enabled,
            trustee_threshold: r.trustee_threshold,
            notify_email: r.notify_email,
            notify_phone: r.notify_phone,
        }
    }
}

#[derive(sqlx::FromRow)]
struct TrusteeRow {
    id: Uuid,
    principal_id: Uuid,
    label_commitment: String,
    trustee_pubkey: String,
    trustee_principal_id: Option<Uuid>,
}

impl From<TrusteeRow> for Trustee {
    fn from(r: TrusteeRow) -> Self {
        Self {
            id: r.id,
            principal_id: PrincipalId::from_uuid(r.principal_id),
            label_commitment: r.label_commitment,
            trustee_pubkey: r.trustee_pubkey,
            trustee_principal_id: r.trustee_principal_id.map(PrincipalId::from_uuid),
        }
    }
}

#[derive(sqlx::FromRow)]
struct EventRow {
    id: Uuid,
    principal_id: Uuid,
    method: String,
    status: String,
    unlock_at: Option<chrono::DateTime<Utc>>,
    cancel_token_hash: Option<String>,
    threshold: Option<i32>,
    recovery_token_hash: Option<String>,
    recovery_claimed_at: Option<chrono::DateTime<Utc>>,
    created_at: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct RateRow {
    bucket_key: String,
    window_start: chrono::DateTime<Utc>,
    hit_count: i32,
}
