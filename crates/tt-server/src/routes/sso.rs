//! Public SSO: domain discovery, OIDC (PKCE + JWKS), SAML ACS.

use std::collections::HashMap;
use std::io::Read;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine;
use chrono::{Duration, Utc};
use flate2::read::DeflateDecoder;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use tt_core::domain::{Principal, PrincipalKind};
use tt_core::ids::{OrgId, PrincipalId};
use uuid::Uuid;

use crate::routes::jwks_cache::JwksCache;
use crate::routes::saml_dsig;
use crate::state::AppState;
use std::sync::LazyLock;

static JWKS_CACHE: LazyLock<JwksCache> = LazyLock::new(JwksCache::new);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/sso/discover", post(discover))
        .route("/api/auth/sso/oidc/start", post(oidc_start))
        .route("/api/auth/sso/oidc/callback", get(oidc_callback))
        .route("/api/auth/sso/saml/start", post(saml_start))
        .route("/api/auth/sso/saml/acs", post(saml_acs))
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DiscoverBody {
    pub email_or_domain: String,
}

#[derive(Debug, Serialize)]
pub struct DiscoverResponse {
    pub found: bool,
    pub org_slug: Option<String>,
    pub protocol: Option<String>,
    pub provider: Option<String>,
    pub enforce_sso: bool,
    pub domains: Vec<String>,
    pub issuer_url: Option<String>,
    pub message: String,
}

async fn discover(
    State(state): State<AppState>,
    Json(body): Json<DiscoverBody>,
) -> impl IntoResponse {
    match state
        .orgs
        .discover_sso_by_domain(&body.email_or_domain)
        .await
    {
        Ok(Some((slug, cfg))) => {
            let entitled = state.billing.assert_sso_entitled(cfg.org_id).await.is_ok();
            if !entitled {
                return Json(DiscoverResponse {
                    found: false,
                    org_slug: Some(slug),
                    protocol: None,
                    provider: None,
                    enforce_sso: false,
                    domains: vec![],
                    issuer_url: None,
                    message: "SSO is configured but the organization plan does not include SSO (SSO wall)"
                        .into(),
                })
                .into_response();
            }
            Json(DiscoverResponse {
                found: true,
                org_slug: Some(slug),
                protocol: Some(cfg.protocol),
                provider: Some(cfg.provider),
                enforce_sso: cfg.enforce_sso,
                domains: cfg.domains,
                issuer_url: cfg.issuer_url,
                message: "SSO available for this domain".into(),
            })
            .into_response()
        }
        Ok(None) => Json(DiscoverResponse {
            found: false,
            org_slug: None,
            protocol: None,
            provider: None,
            enforce_sso: false,
            domains: vec![],
            issuer_url: None,
            message: "no SSO organization for this domain".into(),
        })
        .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// OIDC + JWKS
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OidcStartBody {
    pub email_or_domain: String,
    pub client_redirect_uri: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OidcStartResponse {
    pub authorization_url: String,
    pub state: String,
}

async fn oidc_start(
    State(state): State<AppState>,
    Json(body): Json<OidcStartBody>,
) -> impl IntoResponse {
    let (slug, cfg) = match state.orgs.discover_sso_by_domain(&body.email_or_domain).await {
        Ok(Some(x)) => x,
        Ok(None) => return (StatusCode::NOT_FOUND, "no SSO for domain").into_response(),
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    if cfg.protocol != "oidc" {
        return (
            StatusCode::BAD_REQUEST,
            "domain is configured for SAML; use /api/auth/sso/saml/start".to_string(),
        )
            .into_response();
    }
    if let Err(e) = state.billing.assert_sso_entitled(cfg.org_id).await {
        return (
            StatusCode::PAYMENT_REQUIRED,
            Json(serde_json::json!({ "error": e.to_string(), "code": "sso_wall" })),
        )
            .into_response();
    }
    let issuer = match cfg.issuer_url.filter(|u| !u.is_empty()) {
        Some(u) => u,
        None => return (StatusCode::BAD_REQUEST, "issuer_url not configured").into_response(),
    };
    let client_id = match cfg.client_id.filter(|c| !c.is_empty()) {
        Some(c) => c,
        None => return (StatusCode::BAD_REQUEST, "client_id not configured").into_response(),
    };

    let meta = match fetch_oidc_metadata(&issuer).await {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, "oidc metadata");
            return (StatusCode::BAD_GATEWAY, format!("IdP metadata: {e}")).into_response();
        }
    };

    let state_token = random_token("sso");
    let code_verifier = random_token("pkce");
    let code_challenge = pkce_challenge(&code_verifier);
    let nonce = random_token("nonce");
    let redirect_uri = callback_url(&state, "oidc");

    let expires = Utc::now() + Duration::minutes(10);
    if let Err(e) = sqlx::query(
        r#"
        INSERT INTO sso_login_states (state, org_id, protocol, code_verifier, nonce, redirect_uri, email_hint, expires_at)
        VALUES ($1,$2,'oidc',$3,$4,$5,$6,$7)
        "#,
    )
    .bind(&state_token)
    .bind(cfg.org_id.as_uuid())
    .bind(&code_verifier)
    .bind(&nonce)
    .bind(&redirect_uri)
    .bind(&body.email_or_domain)
    .bind(expires)
    .execute(&state.db)
    .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    let mut auth = meta.authorization_endpoint;
    let sep = if auth.contains('?') { '&' } else { '?' };
    auth.push(sep);
    auth.push_str(&format!(
        "response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256",
        urlencoding(&client_id),
        urlencoding(&redirect_uri),
        urlencoding("openid profile email"),
        urlencoding(&state_token),
        urlencoding(&nonce),
        urlencoding(&code_challenge),
    ));

    info!(org = %slug, "oidc start");
    Json(OidcStartResponse {
        authorization_url: auth,
        state: state_token,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct OidcCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

async fn oidc_callback(
    State(state): State<AppState>,
    Query(q): Query<OidcCallbackQuery>,
) -> impl IntoResponse {
    if let Some(err) = q.error {
        return (StatusCode::BAD_REQUEST, format!("IdP error: {err}")).into_response();
    }
    let code = match q.code {
        Some(c) => c,
        None => return (StatusCode::BAD_REQUEST, "missing code").into_response(),
    };
    let st = match q.state {
        Some(s) => s,
        None => return (StatusCode::BAD_REQUEST, "missing state").into_response(),
    };

    let row = match take_login_state(&state, &st).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let org_id = OrgId::from_uuid(row.org_id);
    if let Err(e) = state.billing.assert_sso_entitled(org_id).await {
        return (StatusCode::PAYMENT_REQUIRED, e.to_string()).into_response();
    }

    let cfg = match state.orgs.get_sso(org_id).await {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let issuer = cfg.issuer_url.clone().unwrap_or_default();
    let client_id = cfg.client_id.clone().unwrap_or_default();
    let client_secret = state
        .orgs
        .get_sso_client_secret(org_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    let meta = match fetch_oidc_metadata(&issuer).await {
        Ok(m) => m,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };

    let token = match exchange_code(
        &meta.token_endpoint,
        &client_id,
        &client_secret,
        &code,
        &row.redirect_uri,
        row.code_verifier.as_deref().unwrap_or(""),
    )
    .await
    {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("token exchange: {e}")).into_response(),
    };

    let id_token = match token.id_token.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return (StatusCode::BAD_GATEWAY, "missing id_token").into_response(),
    };

    let claims = match verify_id_token_jwks(
        id_token,
        &issuer,
        &client_id,
        meta.jwks_uri.as_deref(),
        row.nonce.as_deref(),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "id_token jwks verify failed");
            return (StatusCode::UNAUTHORIZED, format!("id_token invalid: {e}")).into_response();
        }
    };

    finish_sso_login(&state, org_id, &claims, cfg.jit_provisioning, "sso-oidc").await
}

// ---------------------------------------------------------------------------
// SAML ACS
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SamlStartBody {
    pub email_or_domain: String,
}

#[derive(Debug, Serialize)]
pub struct SamlStartResponse {
    /// IdP SSO URL with SAMLRequest (Redirect binding)
    pub redirect_url: String,
    pub state: String,
}

async fn saml_start(
    State(state): State<AppState>,
    Json(body): Json<SamlStartBody>,
) -> impl IntoResponse {
    let (slug, cfg) = match state.orgs.discover_sso_by_domain(&body.email_or_domain).await {
        Ok(Some(x)) => x,
        Ok(None) => return (StatusCode::NOT_FOUND, "no SSO for domain").into_response(),
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    if cfg.protocol != "saml" {
        return (
            StatusCode::BAD_REQUEST,
            "domain is configured for OIDC; use /api/auth/sso/oidc/start".to_string(),
        )
            .into_response();
    }
    if let Err(e) = state.billing.assert_sso_entitled(cfg.org_id).await {
        return (
            StatusCode::PAYMENT_REQUIRED,
            Json(serde_json::json!({ "error": e.to_string(), "code": "sso_wall" })),
        )
            .into_response();
    }

    let idp_sso = match cfg.saml_sso_url.filter(|u| !u.is_empty()) {
        Some(u) => u,
        None => {
            return (StatusCode::BAD_REQUEST, "saml_sso_url not configured").into_response();
        }
    };
    let sp_entity = cfg
        .saml_sp_entity_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("https://{}/saml/{}", public_host(&state), slug));

    let state_token = random_token("saml");
    let acs = callback_url(&state, "saml");
    let expires = Utc::now() + Duration::minutes(10);
    if let Err(e) = sqlx::query(
        r#"
        INSERT INTO sso_login_states (state, org_id, protocol, redirect_uri, email_hint, expires_at)
        VALUES ($1,$2,'saml',$3,$4,$5)
        "#,
    )
    .bind(&state_token)
    .bind(cfg.org_id.as_uuid())
    .bind(&acs)
    .bind(&body.email_or_domain)
    .bind(expires)
    .execute(&state.db)
    .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    let saml_request_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol"
    xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion"
    ID="_{state_token}"
    Version="2.0"
    IssueInstant="{instant}"
    Destination="{dest}"
    AssertionConsumerServiceURL="{acs}"
    ProtocolBinding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST">
  <saml:Issuer>{issuer}</saml:Issuer>
  <samlp:NameIDPolicy Format="urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress" AllowCreate="true"/>
</samlp:AuthnRequest>"#,
        instant = Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
        dest = idp_sso,
        acs = acs,
        issuer = sp_entity,
        state_token = state_token.replace(['+', '/', '='], ""),
    );

    let deflated = deflate_raw(saml_request_xml.as_bytes());
    let b64 = base64::engine::general_purpose::STANDARD.encode(deflated);
    let redirect_url = format!(
        "{}{}SAMLRequest={}&RelayState={}",
        idp_sso,
        if idp_sso.contains('?') { '&' } else { '?' },
        urlencoding(&b64),
        urlencoding(&state_token),
    );

    info!(org = %slug, "saml start");
    Json(SamlStartResponse {
        redirect_url,
        state: state_token,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct SamlAcsForm {
    #[serde(rename = "SAMLResponse")]
    pub saml_response: String,
    #[serde(rename = "RelayState")]
    pub relay_state: Option<String>,
}

async fn saml_acs(
    State(state): State<AppState>,
    Form(form): Form<SamlAcsForm>,
) -> impl IntoResponse {
    let relay = form.relay_state.unwrap_or_default();
    if relay.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing RelayState").into_response();
    }

    let row = match take_login_state(&state, &relay).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    if row.protocol != "saml" {
        return (StatusCode::BAD_REQUEST, "state is not SAML").into_response();
    }

    let org_id = OrgId::from_uuid(row.org_id);
    if let Err(e) = state.billing.assert_sso_entitled(org_id).await {
        return (StatusCode::PAYMENT_REQUIRED, e.to_string()).into_response();
    }

    let cfg = match state.orgs.get_sso(org_id).await {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let xml = match decode_saml_response(&form.saml_response) {
        Ok(x) => x,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("SAMLResponse: {e}")).into_response(),
    };

    // XML-DSig: require IdP cert and valid signature
    let cert = match state.orgs.get_sso_saml_idp_x509(org_id).await {
        Ok(Some(c)) if !c.trim().is_empty() => c,
        Ok(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                "SAML IdP certificate not configured (saml_idp_x509 required)",
            )
                .into_response();
        }
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if let Err(e) = saml_dsig::verify_saml_xml_dsig(&xml, &cert) {
        warn!(org_id = %org_id, error = %e, "SAML XML-DSig verification failed");
        return (StatusCode::UNAUTHORIZED, format!("SAML signature invalid: {e}")).into_response();
    }

    let claims = match parse_saml_claims(&xml) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("assertion: {e}")).into_response(),
    };

    // Basic audience / destination checks when present
    if let Some(ref dest) = claims.destination {
        let acs = callback_url(&state, "saml");
        if !dest.is_empty() && *dest != acs && !dest.contains("/api/auth/sso/saml/acs") {
            warn!(%dest, %acs, "SAML Destination mismatch");
        }
    }

    finish_sso_login(&state, org_id, &claims.to_oidc_like(), cfg.jit_provisioning, "sso-saml")
        .await
}

// ---------------------------------------------------------------------------
// Shared login completion
// ---------------------------------------------------------------------------

async fn finish_sso_login(
    state: &AppState,
    org_id: OrgId,
    claims: &serde_json::Value,
    jit: bool,
    session_label: &str,
) -> axum::response::Response {
    let email = claims
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sub = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = claims
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| claims.get("preferred_username").and_then(|v| v.as_str()))
        .unwrap_or("SSO User")
        .to_string();

    if email.is_empty() && sub.is_empty() {
        return (StatusCode::BAD_GATEWAY, "IdP did not return email or subject").into_response();
    }

    let principal = match resolve_or_provision(state, &email, &sub, &name, jit).await {
        Ok(p) => p,
        Err(e) => return (StatusCode::FORBIDDEN, e).into_response(),
    };

    let session_token = match state
        .sessions
        .create_session(principal.id, Some(session_label.into()), None)
        .await
    {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    info!(principal_id = %principal.id, %org_id, label = session_label, "sso login ok");
    let dest = format!(
        "/auth/sso/complete#token={}&handle={}",
        urlencoding(&session_token),
        urlencoding(&principal.handle)
    );
    Redirect::temporary(&dest).into_response()
}

async fn resolve_or_provision(
    state: &AppState,
    email: &str,
    sub: &str,
    name: &str,
    jit: bool,
) -> Result<Principal, String> {
    if !email.is_empty() {
        if let Ok(Some(p)) = state.auth_store.get_principal_by_email(email).await {
            return Ok(p);
        }
    }
    let base = if !email.is_empty() {
        email.split('@').next().unwrap_or("user")
    } else {
        "sso"
    };
    let handle = format!(
        "{}_{}",
        sanitize_handle(base),
        &sub.chars().take(8).collect::<String>()
    );
    if let Ok(Some(p)) = state.auth_store.get_principal_by_handle(&handle).await {
        return Ok(p);
    }
    if !jit {
        return Err("user not provisioned and JIT is disabled".into());
    }
    let now = Utc::now();
    let p = Principal {
        id: PrincipalId::new(),
        kind: PrincipalKind::Human,
        display_name: name.into(),
        handle,
        avatar_url: None,
        email: if email.is_empty() {
            None
        } else {
            Some(email.into())
        },
        owner_id: None,
        is_active: true,
        created_at: now,
        updated_at: now,
    };
    state
        .auth_store
        .create_principal(&p)
        .await
        .map_err(|e| e.to_string())?;
    Ok(p)
}

// ---------------------------------------------------------------------------
// State helpers
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct StateRow {
    state: String,
    org_id: Uuid,
    protocol: String,
    code_verifier: Option<String>,
    nonce: Option<String>,
    redirect_uri: String,
    email_hint: Option<String>,
    expires_at: chrono::DateTime<Utc>,
}

async fn take_login_state(
    state: &AppState,
    st: &str,
) -> Result<StateRow, axum::response::Response> {
    let row = sqlx::query_as::<_, StateRow>(
        r#"
        SELECT state, org_id, protocol, code_verifier, nonce, redirect_uri, email_hint, expires_at
        FROM sso_login_states WHERE state = $1
        "#,
    )
    .bind(st)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())?;

    let row = row.ok_or_else(|| (StatusCode::BAD_REQUEST, "invalid state").into_response())?;

    let _ = sqlx::query("DELETE FROM sso_login_states WHERE state = $1")
        .bind(st)
        .execute(&state.db)
        .await;

    if row.expires_at < Utc::now() {
        return Err((StatusCode::BAD_REQUEST, "state expired").into_response());
    }
    Ok(row)
}

fn callback_url(state: &AppState, kind: &str) -> String {
    let host = public_host(state);
    match kind {
        "saml" => format!("http://{}:{}/api/auth/sso/saml/acs", host, state.config.port),
        _ => format!(
            "http://{}:{}/api/auth/sso/oidc/callback",
            host, state.config.port
        ),
    }
}

fn public_host(state: &AppState) -> String {
    if state.config.host == "0.0.0.0" {
        "127.0.0.1".into()
    } else {
        state.config.host.clone()
    }
}

// ---------------------------------------------------------------------------
// OIDC metadata + JWKS verify
// ---------------------------------------------------------------------------

struct OidcMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: Option<String>,
}

async fn fetch_oidc_metadata(issuer: &str) -> Result<OidcMetadata, String> {
    let base = issuer.trim_end_matches('/');
    let url = format!("{base}/.well-known/openid-configuration");
    let res = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("metadata HTTP {}", res.status()));
    }
    let v: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(OidcMetadata {
        authorization_endpoint: v["authorization_endpoint"]
            .as_str()
            .ok_or("missing authorization_endpoint")?
            .into(),
        token_endpoint: v["token_endpoint"]
            .as_str()
            .ok_or("missing token_endpoint")?
            .into(),
        jwks_uri: v["jwks_uri"].as_str().map(|s| s.to_string()),
    })
}

struct TokenResponse {
    id_token: Option<String>,
}

async fn exchange_code(
    token_endpoint: &str,
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();
    let mut form = HashMap::new();
    form.insert("grant_type", "authorization_code");
    form.insert("code", code);
    form.insert("redirect_uri", redirect_uri);
    form.insert("client_id", client_id);
    form.insert("code_verifier", code_verifier);
    if !client_secret.is_empty() {
        form.insert("client_secret", client_secret);
    }
    let res = client
        .post(token_endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(body);
    }
    let v: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(TokenResponse {
        id_token: v["id_token"].as_str().map(|s| s.to_string()),
    })
}

/// Verify ID token signature via JWKS and validate iss/aud/exp/nonce.
async fn verify_id_token_jwks(
    id_token: &str,
    issuer: &str,
    client_id: &str,
    jwks_uri: Option<&str>,
    expected_nonce: Option<&str>,
) -> Result<serde_json::Value, String> {
    let header = decode_header(id_token).map_err(|e| format!("jwt header: {e}"))?;
    let kid = header.kid.ok_or("id_token missing kid")?;
    let alg = header.alg;

    let jwks_uri = jwks_uri.ok_or("IdP metadata missing jwks_uri")?;
    let mut jwks = JWKS_CACHE.get_or_fetch(jwks_uri).await?;

    let keys = jwks["keys"]
        .as_array()
        .ok_or("jwks missing keys array")?;
    let mut jwk = keys
        .iter()
        .find(|k| k["kid"].as_str() == Some(kid.as_str()))
        .cloned();
    if jwk.is_none() {
        JWKS_CACHE.invalidate(jwks_uri).await;
        jwks = JWKS_CACHE.get_or_fetch(jwks_uri).await?;
        let keys2 = jwks["keys"].as_array().ok_or("jwks missing keys array")?;
        jwk = keys2
            .iter()
            .find(|k| k["kid"].as_str() == Some(kid.as_str()))
            .cloned();
    }
    let jwk = jwk.ok_or_else(|| format!("no jwk for kid={kid}"))?;

    let decoding_key = jwk_to_decoding_key(&jwk)?;

    let mut validation = Validation::new(alg);
    validation.set_audience(&[client_id]);
    validation.set_issuer(&[issuer.trim_end_matches('/')]);
    // also accept issuer with trailing slash mismatch by adding both
    let issuer_slash = format!("{}/", issuer.trim_end_matches('/'));
    validation.set_issuer(&[issuer.trim_end_matches('/'), issuer_slash.as_str()]);
    validation.validate_exp = true;
    validation.validate_nbf = false;

    let token_data = decode::<serde_json::Value>(id_token, &decoding_key, &validation)
        .map_err(|e| format!("jwt verify: {e}"))?;

    if let Some(expected) = expected_nonce {
        let got = token_data
            .claims
            .get("nonce")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if got != expected {
            return Err("nonce mismatch".into());
        }
    }

    Ok(token_data.claims)
}

fn jwk_to_decoding_key(jwk: &serde_json::Value) -> Result<DecodingKey, String> {
    let kty = jwk["kty"].as_str().unwrap_or("");
    match kty {
        "RSA" => {
            let n = jwk["n"].as_str().ok_or("jwk missing n")?;
            let e = jwk["e"].as_str().ok_or("jwk missing e")?;
            DecodingKey::from_rsa_components(n, e).map_err(|err| err.to_string())
        }
        "EC" => {
            // Prefer RSA JWKs from most enterprise IdPs; EC needs PEM material.
            Err("EC JWKs not supported yet — configure IdP for RSA signing keys".into())
        }
        _ => Err(format!("unsupported jwk kty={kty}")),
    }
}

// ---------------------------------------------------------------------------
// SAML parse helpers
// ---------------------------------------------------------------------------

struct SamlClaims {
    email: Option<String>,
    name_id: Option<String>,
    display_name: Option<String>,
    destination: Option<String>,
}

impl SamlClaims {
    fn to_oidc_like(&self) -> serde_json::Value {
        let sub = self
            .name_id
            .clone()
            .or_else(|| self.email.clone())
            .unwrap_or_default();
        let email = self.email.clone().unwrap_or_default();
        let name = self
            .display_name
            .clone()
            .unwrap_or_else(|| email.clone());
        serde_json::json!({
            "sub": sub,
            "email": email,
            "name": name,
        })
    }
}

fn decode_saml_response(b64: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(b64.trim()))
        .map_err(|e| e.to_string())?;
    // Some IdPs send raw XML; others deflate
    if bytes.starts_with(b"<?xml") || bytes.starts_with(b"<") {
        return String::from_utf8(bytes).map_err(|e| e.to_string());
    }
    let mut decoder = DeflateDecoder::new(&bytes[..]);
    let mut out = String::new();
    match decoder.read_to_string(&mut out) {
        Ok(_) => Ok(out),
        Err(_) => String::from_utf8(bytes).map_err(|e| e.to_string()),
    }
}

fn parse_saml_claims(xml: &str) -> Result<SamlClaims, String> {
    let name_id = extract_xml_text(xml, "NameID");
    let email = extract_saml_attr(xml, "email")
        .or_else(|| extract_saml_attr(xml, "mail"))
        .or_else(|| extract_saml_attr(xml, "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress"))
        .or_else(|| name_id.clone().filter(|s| s.contains('@')));
    let display_name = extract_saml_attr(xml, "displayName")
        .or_else(|| extract_saml_attr(xml, "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/name"));
    let destination = extract_xml_attr(xml, "Destination");

    if name_id.is_none() && email.is_none() {
        return Err("no NameID or email attribute".into());
    }
    Ok(SamlClaims {
        email,
        name_id,
        display_name,
        destination,
    })
}

fn extract_xml_text(xml: &str, tag: &str) -> Option<String> {
    // Handles optional namespace prefix: <saml:NameID>...</saml:NameID>
    let patterns = [
        format!(":{tag}>"),
        format!("<{tag}>"),
        format!("<{tag} "),
    ];
    for p in patterns {
        if let Some(idx) = xml.find(&p) {
            let start = xml[idx..].find('>').map(|i| idx + i + 1)?;
            let end_tag = format!("{tag}>");
            let rest = &xml[start..];
            if let Some(end) = rest.find(&end_tag) {
                // back up over </ or </prefix:
                let slice = &rest[..end];
                if let Some(close) = slice.rfind('<') {
                    return Some(slice[..close].trim().to_string());
                }
            }
        }
    }
    None
}

fn extract_xml_attr(xml: &str, attr: &str) -> Option<String> {
    let key = format!("{attr}=\"");
    let idx = xml.find(&key)?;
    let start = idx + key.len();
    let rest = &xml[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_saml_attr(xml: &str, name: &str) -> Option<String> {
    // Attribute Name="email" ... <AttributeValue>x</AttributeValue>
    let needle = format!("Name=\"{name}\"");
    let idx = xml.find(&needle)?;
    let rest = &xml[idx..];
    if let Some(vstart) = rest.find("AttributeValue") {
        let after = &rest[vstart..];
        if let Some(gt) = after.find('>') {
            let content = &after[gt + 1..];
            if let Some(end) = content.find('<') {
                return Some(content[..end].trim().to_string());
            }
        }
    }
    None
}

fn deflate_raw(data: &[u8]) -> Vec<u8> {
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::fast());
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

fn sanitize_handle(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(24)
        .collect()
}

fn random_token(prefix: &str) -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    format!(
        "{prefix}_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
    )
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
