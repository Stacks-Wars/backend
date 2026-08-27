use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use sw_domain::LobbyStatus;
use thiserror::Error;

/// Compact lobby the create-cap 409 returns so the modal can link rooms.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedLobbyRef {
    pub path: String,
    pub name: String,
    pub status: LobbyStatus,
}

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

    /// Host already has the maximum unfinished lobbies.
    #[error("settle your open lobbies before hosting another")]
    TooManyLobbies { lobbies: Vec<HostedLobbyRef> },

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
    #[serde(skip_serializing_if = "Option::is_none")]
    lobbies: Option<Vec<HostedLobbyRef>>,
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
            Self::TooManyLobbies { .. } => (StatusCode::CONFLICT, "too_many_lobbies"),
            Self::LobbyFull => (StatusCode::CONFLICT, "lobby_full"),
            Self::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::AccountDeleteBlocked { .. } => (StatusCode::CONFLICT, "account_delete_blocked"),
            Self::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, mut code) = self.status_and_code();
        let error = self.to_string();
        let (required_micro, available_micro, pending_claim_micro, delete_code, lobbies) =
            match self {
                Self::InsufficientBalance {
                    required_micro,
                    available_micro,
                } => (
                    Some(required_micro),
                    Some(available_micro),
                    None,
                    None,
                    None,
                ),
                Self::AccountDeleteBlocked {
                    code: delete_code,
                    available_micro,
                    pending_claim_micro,
                } => {
                    code = delete_code;
                    (
                        None,
                        Some(available_micro),
                        Some(pending_claim_micro),
                        Some(delete_code),
                        None,
                    )
                }
                Self::TooManyLobbies { lobbies } => (None, None, None, None, Some(lobbies)),
                _ => (None, None, None, None, None),
            };
        let body = ErrorBody {
            error,
            code,
            required_micro,
            available_micro,
            pending_claim_micro,
            delete_code,
            lobbies,
        };
        (status, Json(body)).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
