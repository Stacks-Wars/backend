use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("unauthorized: {0}")]
    Unauthorized(&'static str),

    #[error("not found: {0}")]
    NotFound(&'static str),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("insufficient balance: need {required_micro} micro-USDCx, available {available_micro}")]
    InsufficientBalance {
        required_micro: i64,
        available_micro: i64,
    },

    #[error("conflict: {0}")]
    Conflict(String),

    /// Lobby capacity reached (including concurrent seat holds).
    #[error("lobby is full")]
    LobbyFull,

    #[error("rate limit exceeded")]
    RateLimited,

    #[error("cannot delete account: {code}")]
    AccountDeleteBlocked {
        code: &'static str,
        available_micro: i64,
        pending_claim_micro: i64,
    },

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    error: String,
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    required_micro: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    available_micro: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_claim_micro: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delete_code: Option<&'static str>,
}

impl AppError {
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            Self::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::InsufficientBalance { .. } => {
                (StatusCode::PAYMENT_REQUIRED, "insufficient_balance")
            }
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Self::LobbyFull => (StatusCode::CONFLICT, "lobby_full"),
            Self::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::AccountDeleteBlocked { .. } => (StatusCode::CONFLICT, "account_delete_blocked"),
            Self::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        let (required_micro, available_micro, pending_claim_micro, delete_code) = match &self {
            Self::InsufficientBalance {
                required_micro,
                available_micro,
            } => (Some(*required_micro), Some(*available_micro), None, None),
            Self::AccountDeleteBlocked {
                code,
                available_micro,
                pending_claim_micro,
            } => (
                None,
                Some(*available_micro),
                Some(*pending_claim_micro),
                Some(*code),
            ),
            _ => (None, None, None, None),
        };
        let code = if let Some(delete_code) = delete_code {
            delete_code
        } else {
            code
        };
        let body = ErrorBody {
            error: self.to_string(),
            code,
            required_micro,
            available_micro,
            pending_claim_micro,
            delete_code,
        };
        (status, Json(body)).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
