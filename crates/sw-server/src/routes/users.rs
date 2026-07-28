use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data::users::{CustodialWalletInput, PgUserRepo, UpsertUserInput};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use sw_domain::User;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_users).post(upsert_user))
        .route("/{user_id}", get(get_user))
        .route("/{user_id}/stats", get(user_stats))
        .route(
            "/{user_id}/custodial-wallet",
            get(get_custodial_wallet).post(create_custodial_wallet),
        )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertUserBody {
    email: String,
    display_name: Option<String>,
    avatar_url: Option<String>,
    email_verified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateCustodialWalletBody {
    stx_address: String,
    public_key: String,
    encrypted_mnemonic: String,
    kms_key_version: String,
    network: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserResponse {
    id: Uuid,
    username: Option<String>,
    display_name: Option<String>,
    email: String,
    email_verified_at: Option<DateTime<Utc>>,
    wallet_address: Option<String>,
    wallet_verified_at: Option<DateTime<Utc>>,
    avatar_url: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id.as_uuid(),
            username: user.username,
            display_name: user.display_name,
            email: user.email,
            email_verified_at: user.email_verified_at,
            wallet_address: user.wallet_address,
            wallet_verified_at: user.wallet_verified_at,
            avatar_url: user.avatar_url,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CustodialWalletResponse {
    user_id: Uuid,
    stx_address: String,
    public_key: String,
    network: String,
}

fn require_internal_secret(headers: &HeaderMap, expected: &str) -> AppResult<()> {
    let provided = headers
        .get("x-internal-secret")
        .and_then(|value| value.to_str().ok());

    match provided {
        Some(value) if value == expected => Ok(()),
        _ => Err(AppError::Unauthorized("invalid internal secret")),
    }
}

async fn upsert_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpsertUserBody>,
) -> AppResult<Json<UserResponse>> {
    require_internal_secret(&headers, &state.config.internal_api_secret)?;

    let email = body.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(AppError::BadRequest("valid email is required".into()));
    }

    let repo = PgUserRepo::new(state.db.clone());
    let user = repo
        .upsert(UpsertUserInput {
            email,
            display_name: body
                .display_name
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            avatar_url: body
                .avatar_url
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            email_verified_at: body.email_verified_at,
        })
        .await?;

    Ok(Json(UserResponse::from(user)))
}

async fn list_users() -> AppResult<()> {
    Err(AppError::NotImplemented("list users"))
}

async fn get_user(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> AppResult<Json<UserResponse>> {
    let repo = PgUserRepo::new(state.db.clone());
    let user = repo
        .get_by_id(user_id.into())
        .await?
        .ok_or(AppError::NotFound("user"))?;

    Ok(Json(UserResponse::from(user)))
}

async fn user_stats() -> AppResult<()> {
    Err(AppError::NotImplemented("user stats"))
}

async fn get_custodial_wallet(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
) -> AppResult<Json<CustodialWalletResponse>> {
    require_internal_secret(&headers, &state.config.internal_api_secret)?;

    let repo = PgUserRepo::new(state.db.clone());
    let wallet = repo
        .get_custodial_wallet(user_id.into())
        .await?
        .ok_or(AppError::NotFound("custodial wallet"))?;

    Ok(Json(CustodialWalletResponse {
        user_id: wallet.user_id,
        stx_address: wallet.stx_address,
        public_key: wallet.public_key,
        network: wallet.network,
    }))
}

async fn create_custodial_wallet(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
    Json(body): Json<CreateCustodialWalletBody>,
) -> AppResult<Json<CustodialWalletResponse>> {
    require_internal_secret(&headers, &state.config.internal_api_secret)?;

    if body.stx_address.trim().is_empty()
        || body.public_key.trim().is_empty()
        || body.encrypted_mnemonic.trim().is_empty()
        || body.kms_key_version.trim().is_empty()
        || body.network.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "custodial wallet fields must be non-empty".into(),
        ));
    }

    let repo = PgUserRepo::new(state.db.clone());
    let wallet = repo
        .create_custodial_wallet(
            user_id.into(),
            CustodialWalletInput {
                stx_address: body.stx_address.trim().to_owned(),
                public_key: body.public_key.trim().to_owned(),
                encrypted_mnemonic: body.encrypted_mnemonic.trim().to_owned(),
                kms_key_version: body.kms_key_version.trim().to_owned(),
                network: body.network.trim().to_owned(),
            },
        )
        .await?;

    Ok(Json(CustodialWalletResponse {
        user_id: wallet.user_id,
        stx_address: wallet.stx_address,
        public_key: wallet.public_key,
        network: wallet.network,
    }))
}
