//! Neon Managed Better Auth JWT verification (EdDSA + JWKS).

use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use parking_lot::RwLock;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct NeonJwtConfig {
    pub jwks_url: String,
    pub issuer: String,
    pub audience: String,
}

impl NeonJwtConfig {
    /// Derive JWKS URL + iss/aud origin from `NEON_AUTH_BASE_URL`.
    pub fn from_auth_base_url(base_url: &str) -> AppResult<Self> {
        let base = base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "NEON_AUTH_BASE_URL is empty"
            )));
        }
        let origin = auth_origin(base)?;
        let jwks_url = format!("{base}/.well-known/jwks.json");
        Ok(Self {
            jwks_url,
            issuer: origin.clone(),
            audience: origin,
        })
    }
}

fn auth_origin(base_url: &str) -> AppResult<String> {
    // e.g. https://ep-xxx.neonauth.../neondb/auth → https://ep-xxx.neonauth...
    let without_scheme = base_url
        .strip_prefix("https://")
        .or_else(|| base_url.strip_prefix("http://"))
        .ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!(
                "NEON_AUTH_BASE_URL must start with http(s)://"
            ))
        })?;
    let scheme = if base_url.starts_with("https://") {
        "https"
    } else {
        "http"
    };
    let host = without_scheme
        .split('/')
        .next()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("NEON_AUTH_BASE_URL missing host")))?;
    Ok(format!("{scheme}://{host}"))
}

#[derive(Debug, Clone)]
pub struct NeonClaims {
    pub user_id: Uuid,
    /// Present on typical Neon tokens; not required for auth (sig + sub suffice).
    pub email: Option<String>,
    pub email_verified: bool,
}

#[derive(Debug, Deserialize)]
struct RawClaims {
    sub: String,
    email: Option<String>,
    #[serde(default, rename = "emailVerified")]
    email_verified: Option<bool>,
    #[serde(flatten)]
    _rest: Value,
}

struct JwksCache {
    keys: Vec<(String, DecodingKey)>,
    fetched_at: Instant,
}

pub struct NeonJwtVerifier {
    config: NeonJwtConfig,
    http: reqwest::Client,
    cache: RwLock<Option<JwksCache>>,
}

impl NeonJwtVerifier {
    pub fn new(config: NeonJwtConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            cache: RwLock::new(None),
        }
    }

    pub fn arc(config: NeonJwtConfig) -> Arc<Self> {
        Arc::new(Self::new(config))
    }

    pub async fn verify(&self, token: &str) -> AppResult<NeonClaims> {
        let token = token.trim();
        if token.is_empty() {
            return Err(AppError::Unauthorized("missing bearer token"));
        }

        let header =
            decode_header(token).map_err(|_| AppError::Unauthorized("invalid token header"))?;
        if header.alg != Algorithm::EdDSA {
            return Err(AppError::Unauthorized("unsupported token algorithm"));
        }
        let kid = header
            .kid
            .clone()
            .ok_or(AppError::Unauthorized("token missing kid"))?;

        let key = self.decoding_key_for_kid(&kid).await?;
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&[self.config.issuer.clone()]);
        validation.set_audience(&[self.config.audience.clone()]);
        validation.validate_exp = true;
        validation.leeway = 30;

        let data = decode::<RawClaims>(token, &key, &validation)
            .map_err(|_| AppError::Unauthorized("invalid or expired token"))?;

        let user_id = parse_neon_sub(&data.claims.sub)?;
        let email = data
            .claims
            .email
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase());
        let email_verified = data.claims.email_verified.unwrap_or(false);

        Ok(NeonClaims {
            user_id,
            email,
            email_verified,
        })
    }

    async fn decoding_key_for_kid(&self, kid: &str) -> AppResult<DecodingKey> {
        if let Some(key) = self.cached_key(kid) {
            return Ok(key);
        }
        self.refresh_jwks().await?;
        self.cached_key(kid)
            .ok_or(AppError::Unauthorized("unknown token kid"))
    }

    fn cached_key(&self, kid: &str) -> Option<DecodingKey> {
        let guard = self.cache.read();
        let cache = guard.as_ref()?;
        if cache.fetched_at.elapsed() > Duration::from_secs(3600) {
            return None;
        }
        cache
            .keys
            .iter()
            .find(|(k, _)| k == kid)
            .map(|(_, key)| key.clone())
    }

    async fn refresh_jwks(&self) -> AppResult<()> {
        let res = self
            .http
            .get(&self.config.jwks_url)
            .send()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        if !res.status().is_success() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "JWKS fetch failed: {}",
                res.status()
            )));
        }
        let body: Value = res.json().await.map_err(|e| AppError::Internal(e.into()))?;
        let keys_json = body
            .get("keys")
            .and_then(|v| v.as_array())
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("JWKS missing keys")))?;

        let mut keys = Vec::new();
        for key_val in keys_json {
            let kid = key_val
                .get("kid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            if kid.is_empty() {
                continue;
            }
            let kty = key_val.get("kty").and_then(|v| v.as_str()).unwrap_or("");
            let crv = key_val.get("crv").and_then(|v| v.as_str()).unwrap_or("");
            let x = key_val.get("x").and_then(|v| v.as_str()).unwrap_or("");
            if kty != "OKP" || crv != "Ed25519" || x.is_empty() {
                continue;
            }
            match DecodingKey::from_ed_components(x) {
                Ok(decoding) => keys.push((kid, decoding)),
                Err(err) => {
                    tracing::warn!(%kid, error = %err, "skip JWKS key");
                }
            }
        }
        if keys.is_empty() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "JWKS contained no usable Ed25519 keys"
            )));
        }
        *self.cache.write() = Some(JwksCache {
            keys,
            fetched_at: Instant::now(),
        });
        Ok(())
    }
}

/// Neon Auth `sub` is the primary key of `users`.
pub fn parse_neon_sub(sub: &str) -> AppResult<Uuid> {
    let id = Uuid::parse_str(sub.trim())
        .map_err(|_| AppError::Unauthorized("token sub is not a uuid"))?;
    if id.is_nil() {
        return Err(AppError::Unauthorized("token sub is not a uuid"));
    }
    Ok(id)
}

pub fn bearer_token_from_header(value: Option<&str>) -> Option<&str> {
    let value = value?.trim();
    let rest = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let token = rest.trim();
    if token.is_empty() { None } else { Some(token) }
}
