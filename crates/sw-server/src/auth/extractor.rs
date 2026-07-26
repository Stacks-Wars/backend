use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::header::AUTHORIZATION;

use crate::error::AppError;
use crate::state::AppState;

use super::{decode_token, JwtClaims};

/// Optional bearer auth. Missing / invalid tokens become `None` rather than errors.
pub struct OptionalAuth(pub Option<JwtClaims>);

/// Required bearer auth for protected routes.
pub struct AuthUser(pub JwtClaims);

impl FromRequestParts<AppState> for OptionalAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(extract_claims(parts, state).ok()))
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let claims = extract_claims(parts, state)?;
        Ok(Self(claims))
    }
}

fn extract_claims(parts: &Parts, state: &AppState) -> Result<JwtClaims, AppError> {
    let header = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized("missing Authorization header"))?;

    let token = header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized("expected Bearer token"))?;

    decode_token(&state.config.jwt_secret, token)
}
