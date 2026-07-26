use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sw_domain::UserId;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: Uuid,
    pub exp: i64,
    pub iat: i64,
}

impl JwtClaims {
    pub fn user_id(&self) -> UserId {
        UserId(self.sub)
    }
}

/// Encode helper kept for future auth flows. Unused by stub routes today.
#[allow(dead_code)]
pub fn encode_token(secret: &str, claims: &JwtClaims) -> AppResult<String> {
    jsonwebtoken::encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|err| AppError::Internal(err.into()))
}

pub fn decode_token(secret: &str, token: &str) -> AppResult<JwtClaims> {
    let mut validation = Validation::default();
    validation.validate_exp = true;

    jsonwebtoken::decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|_| AppError::Unauthorized("invalid or expired token"))
}
