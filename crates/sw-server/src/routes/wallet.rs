//! Custodial on-chain balance, activity, withdrawals (no ledger).

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sw_domain::{ChainActivityItem, UserId, WalletBalance};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::config::{
    MAX_WITHDRAW_MICRO, MIN_WITHDRAW_MICRO, USDCX_ASSET_NAME, USDCX_CONTRACT,
};
use crate::data::users::PgUserRepo;
use crate::error::{AppError, AppResult};
use crate::services::hiro::HiroClient;
use crate::services::wallet_chain::WalletChainService;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/balance/{user_id}", get(get_balance))
        .route("/balance/{user_id}/refresh", post(refresh_balance))
        .route("/activity/{user_id}", get(list_activity))
        .route("/withdrawals/prepare", post(prepare_withdrawal))
        .route("/withdrawals/complete", post(complete_withdrawal))
        .route(
            "/custodial/{user_id}/signing-material",
            get(get_signing_material),
        )
}

fn wallet_chain(state: &AppState) -> WalletChainService {
    let hiro = HiroClient::new(
        state.config.hiro_api_url.clone(),
        state.config.hiro_api_key.clone(),
        USDCX_CONTRACT,
        USDCX_ASSET_NAME,
        Some(state.config.sw_vault_contract.clone()),
    );
    WalletChainService::new(state.db.clone(), state.redis.clone(), hiro)
}

async fn get_balance(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
) -> AppResult<Json<WalletBalance>> {
    auth.require_self(user_id)?;
    let bal = wallet_chain(&state)
        .get_balance(UserId::from(user_id))
        .await?;
    Ok(Json(bal))
}

async fn refresh_balance(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
) -> AppResult<Json<WalletBalance>> {
    auth.require_self(user_id)?;
    let bal = wallet_chain(&state)
        .refresh_balance(UserId::from(user_id))
        .await?;

    let topic = format!("user:{user_id}");
    state.subscriptions.publish(
        &state.sessions,
        &topic,
        crate::ws::ServerMessage {
            kind: "wallet.balance.updated".into(),
            payload: serde_json::json!({
                "availableMicro": bal.available_micro,
                "stxAddress": bal.stx_address,
            }),
        },
    );

    Ok(Json(bal))
}

async fn list_activity(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
) -> AppResult<Json<Vec<ChainActivityItem>>> {
    auth.require_self(user_id)?;
    let items = wallet_chain(&state)
        .activity(UserId::from(user_id), 50)
        .await?;
    Ok(Json(items))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawPrepareBody {
    amount_micro: i64,
    to_address: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawPrepareResponse {
    user_id: Uuid,
    amount_micro: i64,
    from_address: String,
    to_address: String,
    usdcx_contract: String,
    usdcx_asset_name: String,
}

async fn prepare_withdrawal(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<WithdrawPrepareBody>,
) -> AppResult<Json<WithdrawPrepareResponse>> {
    if body.amount_micro < MIN_WITHDRAW_MICRO {
        return Err(AppError::BadRequest(format!(
            "minimum withdrawal is {MIN_WITHDRAW_MICRO} micro-USDCx"
        )));
    }
    if body.amount_micro > MAX_WITHDRAW_MICRO {
        return Err(AppError::BadRequest(format!(
            "maximum withdrawal is {MAX_WITHDRAW_MICRO} micro-USDCx"
        )));
    }

    let user_id = auth.user_id;
    let users = PgUserRepo::new(state.db.clone());
    let user = users
        .get_by_id(user_id)
        .await?
        .ok_or(AppError::NotFound("user not found"))?;
    let custodial = users
        .get_custodial_wallet(user_id)
        .await?
        .ok_or(AppError::NotFound("custodial wallet not found"))?;

    let to_address = body
        .to_address
        .or(user.wallet_address)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            AppError::BadRequest("set a withdrawal address on your profile first".into())
        })?;

    let svc = wallet_chain(&state);
    let bal = svc.refresh_balance(user_id).await?;
    if bal.available_micro < body.amount_micro {
        return Err(AppError::InsufficientBalance {
            required_micro: body.amount_micro,
            available_micro: bal.available_micro,
        });
    }

    svc.acquire_withdraw_lock(user_id, 900).await?;

    Ok(Json(WithdrawPrepareResponse {
        user_id: user_id.as_uuid(),
        amount_micro: body.amount_micro,
        from_address: custodial.stx_address,
        to_address,
        usdcx_contract: USDCX_CONTRACT.to_owned(),
        usdcx_asset_name: USDCX_ASSET_NAME.to_owned(),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawCompleteBody {
    txid: String,
}

async fn complete_withdrawal(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<WithdrawCompleteBody>,
) -> AppResult<Json<WalletBalance>> {
    let user_id = auth.user_id;
    let txid = body.txid.trim();
    if txid.is_empty() {
        return Err(AppError::BadRequest("txid required".into()));
    }

    let svc = wallet_chain(&state);
    svc.hiro().require_tx_success(txid).await?;
    svc.release_withdraw_lock(user_id).await?;
    let bal = svc.refresh_balance(user_id).await?;

    let topic = format!("user:{}", user_id.as_uuid());
    state.subscriptions.publish(
        &state.sessions,
        &topic,
        crate::ws::ServerMessage {
            kind: "wallet.balance.updated".into(),
            payload: serde_json::json!({
                "availableMicro": bal.available_micro,
                "stxAddress": bal.stx_address,
            }),
        },
    );

    Ok(Json(bal))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SigningMaterial {
    user_id: Uuid,
    stx_address: String,
    public_key: String,
    network: String,
    encrypted_mnemonic: String,
    kms_key_version: String,
    usdcx_contract: String,
}

async fn get_signing_material(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
) -> AppResult<Json<SigningMaterial>> {
    auth.require_self(user_id)?;
    let secret = PgUserRepo::new(state.db.clone())
        .get_custodial_wallet_secret(UserId::from(user_id))
        .await?
        .ok_or(AppError::NotFound("custodial wallet not found"))?;

    Ok(Json(SigningMaterial {
        user_id: secret.user_id,
        stx_address: secret.stx_address,
        public_key: secret.public_key,
        network: secret.network,
        encrypted_mnemonic: secret.encrypted_mnemonic,
        kms_key_version: secret.kms_key_version,
        usdcx_contract: USDCX_CONTRACT.to_owned(),
    }))
}
