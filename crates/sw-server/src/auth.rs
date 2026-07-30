//! Authenticated caller extracted from Neon Auth JWT.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use sw_domain::UserId;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::services::neon_jwt::bearer_token_from_header;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: UserId,
    pub email: Option<String>,
    pub email_verified: bool,
}

impl AuthUser {
    pub fn require_self(&self, path_user_id: Uuid) -> AppResult<()> {
        if self.user_id.as_uuid() != path_user_id {
            return Err(AppError::Unauthorized("token subject mismatch"));
        }
        Ok(())
    }

    /// Admin gate: allowlisted email + verified email claim.
    pub fn require_admin(&self, admin_emails: &[String]) -> AppResult<()> {
        let email = self
            .email
            .as_deref()
            .ok_or(AppError::Unauthorized("admin requires email claim"))?;
        if !self.email_verified {
            return Err(AppError::Unauthorized("email not verified"));
        }
        if !admin_emails.iter().any(|a| a == email) {
            return Err(AppError::Unauthorized("not an admin"));
        }
        Ok(())
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        let token = bearer_token_from_header(header)
            .ok_or(AppError::Unauthorized("missing bearer token"))?;
        let claims = state.jwt.verify(token).await?;
        Ok(AuthUser {
            user_id: UserId::from(claims.user_id),
            email: claims.email,
            email_verified: claims.email_verified,
        })
    }
}
