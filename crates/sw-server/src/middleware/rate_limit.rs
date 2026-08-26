//! Redis fixed-window rate limiting (60s) with IP / Neon JWT identity keys.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde::Serialize;
use uuid::Uuid;

use crate::services::neon_jwt::bearer_token_from_header;
use crate::state::AppState;

const WINDOW_SECS: i64 = 60;

/// Named policy with per-identity limits (Global uses different IP vs user caps).
#[derive(Debug, Clone, Copy)]
pub struct RatePolicy {
    pub name: &'static str,
    pub ip_limit: u64,
    pub user_limit: u64,
}

pub const GLOBAL: RatePolicy = RatePolicy {
    name: "global",
    ip_limit: 60,
    user_limit: 240,
};

pub const WRITE: RatePolicy = RatePolicy {
    name: "write",
    ip_limit: 60,
    user_limit: 60,
};

pub const SENSITIVE: RatePolicy = RatePolicy {
    name: "sensitive",
    ip_limit: 20,
    user_limit: 20,
};

pub const WS_CONNECT: RatePolicy = RatePolicy {
    name: "ws_connect",
    ip_limit: 30,
    user_limit: 30,
};

#[derive(Debug, Clone)]
pub struct RateDecision {
    pub allowed: bool,
    pub limit: u64,
    pub remaining: u64,
    pub reset_secs: u64,
}

#[derive(Debug, Clone)]
enum RateIdentity {
    User(Uuid),
    Ip(String),
}

impl RateIdentity {
    fn limit(&self, policy: RatePolicy) -> u64 {
        match self {
            Self::User(_) => policy.user_limit,
            Self::Ip(_) => policy.ip_limit,
        }
    }

    fn redis_key(&self, policy: RatePolicy) -> String {
        match self {
            Self::User(id) => format!("sw:rl:{}:user:{}", policy.name, id),
            Self::Ip(ip) => format!("sw:rl:{}:ip:{}", policy.name, ip),
        }
    }
}

/// INCR + EXPIRE fixed window. Caller handles fail-open on `Err`.
pub async fn check(
    redis: &mut ConnectionManager,
    key: &str,
    limit: u64,
) -> redis::RedisResult<RateDecision> {
    let count: i64 = redis.incr(key, 1).await?;
    if count == 1 {
        let _: bool = redis.expire(key, WINDOW_SECS).await?;
    }
    let ttl: i64 = match redis.ttl(key).await {
        Ok(t) => t,
        Err(_) => WINDOW_SECS,
    };
    let reset_secs = if ttl < 0 {
        WINDOW_SECS as u64
    } else {
        ttl as u64
    };
    let count_u = count.max(0) as u64;
    let allowed = count_u <= limit;
    let remaining = if allowed {
        limit.saturating_sub(count_u)
    } else {
        0
    };
    Ok(RateDecision {
        allowed,
        limit,
        remaining,
        reset_secs,
    })
}

/// First `x-forwarded-for` hop, else `ConnectInfo`, else `"unknown"`.
pub fn client_ip(req: &Request) -> String {
    if let Some(ip) = forwarded_for_first(req.headers()) {
        return ip;
    }
    if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip().to_string();
    }
    "unknown".into()
}

pub fn client_ip_from_parts(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    if let Some(ip) = forwarded_for_first(headers) {
        return ip;
    }
    peer.map(|a| a.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn forwarded_for_first(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("x-forwarded-for")?.to_str().ok()?;
    let first = raw.split(',').next()?.trim();
    if first.is_empty() {
        None
    } else {
        Some(first.to_owned())
    }
}

fn is_internal_exempt(headers: &HeaderMap, secret: &str) -> bool {
    if secret.is_empty() {
        return false;
    }
    headers
        .get("x-internal-secret")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|provided| provided == secret)
}

/// Owned inputs only — `&Request` is `!Send` (body not `Sync`), so never hold it across await.
async fn optional_user_id(state: &AppState, bearer: Option<String>) -> Option<Uuid> {
    let token = bearer_token_from_header(bearer.as_deref())?;
    match state.jwt.verify(token).await {
        Ok(claims) => Some(claims.user_id),
        Err(_) => None,
    }
}

async fn identity(state: &AppState, bearer: Option<String>, ip: String) -> RateIdentity {
    if let Some(user_id) = optional_user_id(state, bearer).await {
        RateIdentity::User(user_id)
    } else {
        RateIdentity::Ip(ip)
    }
}

fn authorization_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitBody {
    error: &'static str,
    code: &'static str,
}

pub fn rate_limited_response(decision: &RateDecision) -> Response {
    let mut res = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(RateLimitBody {
            error: "rate limit exceeded",
            code: "rate_limited",
        }),
    )
        .into_response();
    set_rate_limit_headers(res.headers_mut(), decision);
    res
}

fn set_rate_limit_headers(headers: &mut HeaderMap, decision: &RateDecision) {
    insert_u64(headers, "x-ratelimit-limit", decision.limit);
    insert_u64(headers, "x-ratelimit-remaining", decision.remaining);
    insert_u64(headers, "x-ratelimit-reset", decision.reset_secs);
}

fn insert_u64(headers: &mut HeaderMap, name: &'static str, value: u64) {
    if let Ok(v) = HeaderValue::from_str(&value.to_string()) {
        headers.insert(name, v);
    }
}

async fn enforce(policy: RatePolicy, state: AppState, req: Request, next: Next) -> Response {
    let exempt = is_internal_exempt(req.headers(), state.config.internal_api_secret.as_str());
    let bearer = authorization_header(req.headers());
    let ip = client_ip(&req);

    if exempt {
        return next.run(req).await;
    }

    let id = identity(&state, bearer, ip).await;
    let limit = id.limit(policy);
    let key = id.redis_key(policy);
    let mut redis = state.redis.clone();

    let decision = match check(&mut redis, &key, limit).await {
        Ok(d) => d,
        Err(err) => {
            tracing::warn!(error = %err, policy = policy.name, "rate limit redis error; fail-open");
            return next.run(req).await;
        }
    };

    if !decision.allowed {
        tracing::warn!(
            policy = policy.name,
            key = %key,
            limit = decision.limit,
            "rate limit exceeded"
        );
        return rate_limited_response(&decision);
    }

    let mut res = next.run(req).await;
    set_rate_limit_headers(res.headers_mut(), &decision);
    res
}

pub async fn global_limit(State(state): State<AppState>, req: Request, next: Next) -> Response {
    enforce(GLOBAL, state, req, next).await
}

pub async fn write_limit(State(state): State<AppState>, req: Request, next: Next) -> Response {
    enforce(WRITE, state, req, next).await
}

pub async fn sensitive_limit(State(state): State<AppState>, req: Request, next: Next) -> Response {
    enforce(SENSITIVE, state, req, next).await
}

/// IP for WS upgrade / handlers: `x-forwarded-for` then `ConnectInfo`.
pub struct ClientIp(pub String);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(ClientIp(client_ip_from_parts(
            &parts.headers,
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| *addr),
        )))
    }
}

/// WebSocket connect bucket (IP only). Fail-open on Redis errors.
pub async fn check_ws_connect(
    redis: &mut ConnectionManager,
    ip: &str,
) -> Result<RateDecision, redis::RedisError> {
    let id = RateIdentity::Ip(ip.to_owned());
    let limit = id.limit(WS_CONNECT);
    let key = id.redis_key(WS_CONNECT);
    check(redis, &key, limit).await
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::middleware;
    use axum::routing::{get, post};
    use http_body_util::BodyExt;
    use redis::AsyncCommands;
    use sw_plugin::GameRegistry;
    use tower::ServiceExt;

    use super::*;
    use crate::config::Config;
    use crate::infra::redis_client;
    use crate::services::neon_jwt::NeonJwtConfig;
    use crate::state::AppState;

    const INTERNAL_SECRET: &str = "test-internal-secret";

    #[test]
    fn forwarded_for_uses_first_hop() {
        let req = HttpRequest::builder()
            .header("x-forwarded-for", "1.2.3.4, 5.6.7.8")
            .body(Body::empty())
            .unwrap();
        assert_eq!(client_ip(&req), "1.2.3.4");
    }

    #[test]
    fn redis_key_format() {
        let user = RateIdentity::User(Uuid::nil());
        assert_eq!(
            user.redis_key(SENSITIVE),
            format!("sw:rl:sensitive:user:{}", Uuid::nil())
        );
        let ip = RateIdentity::Ip("9.9.9.9".into());
        assert_eq!(ip.redis_key(GLOBAL), "sw:rl:global:ip:9.9.9.9");
    }

    #[test]
    fn global_limits_differ_by_identity() {
        assert_eq!(RateIdentity::Ip("a".into()).limit(GLOBAL), 60);
        assert_eq!(RateIdentity::User(Uuid::nil()).limit(GLOBAL), 240);
        assert_eq!(RateIdentity::Ip("a".into()).limit(SENSITIVE), 20);
    }

    #[test]
    fn internal_exempt_requires_exact_secret() {
        let mut headers = HeaderMap::new();
        headers.insert("x-internal-secret", HeaderValue::from_static("s3cret"));
        assert!(is_internal_exempt(&headers, "s3cret"));
        assert!(!is_internal_exempt(&headers, "other"));
        assert!(!is_internal_exempt(&HeaderMap::new(), "s3cret"));
        assert!(!is_internal_exempt(&headers, ""));
    }

    async fn test_state() -> AppState {
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
        let redis = redis_client::connect(&redis_url)
            .await
            .expect("redis required for rate_limit tests");
        let db = sqlx::PgPool::connect_lazy("postgres://localhost/sw_rate_limit_test")
            .expect("lazy pg pool");
        let config = Config {
            host: IpAddr::from([127, 0, 0, 1]),
            port: 0,
            database_url: "postgres://localhost/sw_rate_limit_test".into(),
            redis_url,
            hiro_api_url: "https://api.hiro.so".into(),
            hiro_api_key: "test".into(),
            stacks_network: "mainnet".into(),
            sw_vault_contract: "SP000.sw-vault".into(),
            neon_auth_base_url: "https://example.neonauth.test/neondb/auth".into(),
            jwt: NeonJwtConfig {
                jwks_url: "https://example.neonauth.test/.well-known/jwks.json".into(),
                issuer: "https://example.neonauth.test".into(),
                audience: "https://example.neonauth.test".into(),
            },
            admin_emails: vec![],
            internal_api_secret: INTERNAL_SECRET.into(),
            frontend_url: "https://stackswars.com".into(),
            telegram_bot_token: None,
            telegram_chat_id: None,
            vapid_public_key: None,
            vapid_private_key: None,
            vapid_subject: "mailto:contact@mail.stackswars.com".into(),
            solana_rpc_url: "https://api.devnet.solana.com".into(),
            solana_usdc_mint: "2ztYALhLWs2Lg1bGRBje82RgiLhuH4ZbCimRWVeyxUaB".into(),
            solana_vault_program_id: "8NZHj9VH9JkqiAg19CK43ZLuK5hn5jXPBnLfbeKonqfy".into(),
            solana_platform_wallet: String::new(),
        };
        AppState::new(config, db, redis, Arc::new(GameRegistry::new()))
    }

    async fn clear_keys(state: &AppState, keys: &[&str]) {
        let mut redis = state.redis.clone();
        for key in keys {
            let _: () = redis.del(*key).await.unwrap_or(());
        }
    }

    fn header_u64(res: &axum::http::Response<Body>, name: &str) -> u64 {
        res.headers()
            .get(name)
            .unwrap_or_else(|| panic!("missing {name}"))
            .to_str()
            .unwrap()
            .parse()
            .unwrap()
    }

    fn sensitive_app(state: AppState) -> Router {
        Router::new()
            .route("/sensitive", post(|| async { StatusCode::OK }))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                sensitive_limit,
            ))
            .with_state(state)
    }

    fn global_app(state: AppState) -> Router {
        Router::new()
            .route("/ping", get(|| async { StatusCode::OK }))
            .layer(middleware::from_fn_with_state(state.clone(), global_limit))
            .with_state(state)
    }

    #[tokio::test]
    async fn under_limit_sets_headers() {
        let state = test_state().await;
        let ip = format!("198.51.100.{}", Uuid::new_v4().as_u128() % 200 + 1);
        let key = format!("sw:rl:global:ip:{ip}");
        clear_keys(&state, &[&key]).await;

        let app = global_app(state);
        let res = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ping")
                    .header("x-forwarded-for", &ip)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(header_u64(&res, "x-ratelimit-limit"), 60);
        assert_eq!(header_u64(&res, "x-ratelimit-remaining"), 59);
        assert!(header_u64(&res, "x-ratelimit-reset") <= 60);
    }

    #[tokio::test]
    async fn sensitive_over_limit_returns_429() {
        let state = test_state().await;
        let ip = format!("203.0.113.{}", Uuid::new_v4().as_u128() % 200 + 1);
        let key = format!("sw:rl:sensitive:ip:{ip}");
        clear_keys(&state, &[&key]).await;

        let app = sensitive_app(state);
        let mut last_status = StatusCode::OK;
        for i in 1..=21 {
            let res = app
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .method("POST")
                        .uri("/sensitive")
                        .header("x-forwarded-for", &ip)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            last_status = res.status();
            if i <= 20 {
                assert_ne!(res.status(), StatusCode::TOO_MANY_REQUESTS, "req {i}");
                assert_eq!(header_u64(&res, "x-ratelimit-limit"), 20);
            } else {
                assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
                assert_eq!(header_u64(&res, "x-ratelimit-remaining"), 0);
                let body = res.into_body().collect().await.unwrap().to_bytes();
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(v["code"], "rate_limited");
                assert_eq!(v["error"], "rate limit exceeded");
            }
        }
        assert_eq!(last_status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn internal_secret_is_exempt() {
        let state = test_state().await;
        let ip = format!("192.0.2.{}", Uuid::new_v4().as_u128() % 200 + 1);
        let key = format!("sw:rl:sensitive:ip:{ip}");
        clear_keys(&state, &[&key]).await;

        let app = sensitive_app(state);
        for _ in 0..25 {
            let res = app
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .method("POST")
                        .uri("/sensitive")
                        .header("x-forwarded-for", &ip)
                        .header("x-internal-secret", INTERNAL_SECRET)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            assert!(res.headers().get("x-ratelimit-limit").is_none());
        }
    }

    #[tokio::test]
    async fn invalid_bearer_keys_by_ip_without_panic() {
        let state = test_state().await;
        let ip = format!("198.18.0.{}", Uuid::new_v4().as_u128() % 200 + 1);
        let key = format!("sw:rl:global:ip:{ip}");
        clear_keys(&state, &[&key]).await;

        let app = global_app(state);
        let res = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ping")
                    .header("x-forwarded-for", &ip)
                    .header("authorization", "Bearer not-a-valid-jwt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(header_u64(&res, "x-ratelimit-limit"), 60);
    }
}
