use axum::routing::{get, post};
use axum::Router;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// On-chain helpers. Verification / claims are intentionally unfinished.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/balances/{address}", get(get_balances))
        .route("/verify-join", post(verify_join))
        .route("/verify-claim", post(verify_claim))
}

async fn get_balances() -> AppResult<()> {
    Err(AppError::NotImplemented("chain balances"))
}

async fn verify_join() -> AppResult<()> {
    Err(AppError::NotImplemented("verify join tx"))
}

async fn verify_claim() -> AppResult<()> {
    Err(AppError::NotImplemented("verify claim tx"))
}
