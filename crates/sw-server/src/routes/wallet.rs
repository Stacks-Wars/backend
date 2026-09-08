//! Custodial on-chain balance, activity, withdrawals (no ledger).

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sw_domain::{ChainActivityItem, ChainId, UserId, WalletBalance};
use uuid::Uuid;

use crate::auth::{AuthUser, InternalSecret};
use crate::config::{MAX_WITHDRAW_MICRO, MIN_WITHDRAW_MICRO, USDCX_ASSET_NAME};
use crate::data::users::{CustodialWalletInput, PgUserRepo, kms_key_uses_aad};
use crate::error::{AppError, AppResult};
use crate::services::hiro::HiroClient;
use crate::services::{arbitrum_chain, arbitrum_vault, botchain_chain, botchain_vault, solana_chain};
use crate::services::wallet_chain::WalletChainService;
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
struct ChainQuery {
    chain: Option<String>,
}

fn parsed_chain(query: &ChainQuery) -> ChainId {
    ChainId::from_optional(query.chain.as_deref())
}

fn chain_param(query: &ChainQuery) -> String {
    parsed_chain(query).as_str().to_owned()
}

async fn live_balance(
    state: &AppState,
    user_id: UserId,
    chain: ChainId,
    refresh: bool,
) -> AppResult<WalletBalance> {
    match chain {
        ChainId::Solana => solana_chain::get_balance(state, user_id).await,
        ChainId::Arbitrum => arbitrum_chain::get_balance(state, user_id).await,
        ChainId::Botchain => botchain_chain::get_balance(state, user_id).await,
        ChainId::Stacks => {
            let svc = wallet_chain(state);
            if refresh {
                svc.refresh_balance(user_id).await
            } else {
                svc.get_balance(user_id).await
            }
        }
    }
}

async fn live_activity(
    state: &AppState,
    user_id: UserId,
    chain: ChainId,
) -> AppResult<Vec<ChainActivityItem>> {
    match chain {
        ChainId::Solana => solana_chain::list_activity(state, user_id, 50).await,
        ChainId::Arbitrum => arbitrum_chain::list_activity(state, user_id, 50).await,
        ChainId::Botchain => botchain_chain::list_activity(state, user_id, 50).await,
        ChainId::Stacks => wallet_chain(state).activity(user_id, 50).await,
    }
}

/// Withdraw prepare/complete — Sensitive rate tier.
pub fn sensitive_router() -> Router<AppState> {
    Router::new()
        .route("/withdrawals/prepare", post(prepare_withdrawal))
        .route("/withdrawals/complete", post(complete_withdrawal))
        .route("/withdrawals/abort", post(abort_withdrawal))
}

/// Remaining wallet mutations — Write rate tier.
pub fn write_router() -> Router<AppState> {
    Router::new()
        .route("/balance/{user_id}/refresh", post(refresh_balance))
        .route(
            "/custodial/{user_id}",
            post(create_custodial_wallet_internal),
        )
        .route(
            "/custodial/{user_id}/signing-material",
            get(get_signing_material),
        )
        .route(
            "/custodial/{user_id}/encryption",
            axum::routing::patch(update_custodial_encryption),
        )
}

/// Wallet reads — Global tier only. Ciphertext never lives here.
pub fn read_router() -> Router<AppState> {
    Router::new()
        .route("/balance/{user_id}", get(get_balance))
        .route("/activity/{user_id}", get(list_activity))
}

fn wallet_chain(state: &AppState) -> WalletChainService {
    let hiro = HiroClient::new(
        state.config.hiro_api_url.clone(),
        state.config.hiro_api_key.clone(),
        &state.config.usdcx_contract,
        USDCX_ASSET_NAME,
        Some(state.config.sw_vault_contract.clone()),
    );
    WalletChainService::new(state.db.clone(), state.redis.clone(), hiro)
}

async fn get_balance(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
    Query(query): Query<ChainQuery>,
) -> AppResult<Json<WalletBalance>> {
    auth.require_self(user_id)?;
    let chain = parsed_chain(&query);
    let bal = live_balance(&state, UserId::from(user_id), chain, false).await?;
    Ok(Json(bal))
}

async fn refresh_balance(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
    Query(query): Query<ChainQuery>,
) -> AppResult<Json<WalletBalance>> {
    auth.require_self(user_id)?;
    let chain = parsed_chain(&query);
    let bal = live_balance(&state, UserId::from(user_id), chain, true).await?;

    let topic = format!("user:{user_id}");
    state.subscriptions.publish(
        &state.sessions,
        &topic,
        crate::ws::ServerMessage {
            kind: "wallet.balance.updated".into(),
            payload: serde_json::json!({
                "availableMicro": bal.available_micro,
                "address": bal.address,
                "chain": chain,
            }),
        },
    );

    Ok(Json(bal))
}

async fn list_activity(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
    Query(query): Query<ChainQuery>,
) -> AppResult<Json<Vec<ChainActivityItem>>> {
    auth.require_self(user_id)?;
    let chain = parsed_chain(&query);
    let items = live_activity(&state, UserId::from(user_id), chain).await?;
    Ok(Json(items))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawPrepareBody {
    amount_micro: i64,
    to_address: String,
    #[serde(default)]
    chain: Option<String>,
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
    let chain = ChainId::from_optional(body.chain.as_deref());
    let to_address = body.to_address.trim().to_owned();
    if to_address.is_empty() {
        return Err(AppError::BadRequest(
            "destination address is required".into(),
        ));
    }
    if !chain.matches_address(&to_address) {
        return Err(AppError::BadRequest(format!(
            "destination is not a {} address",
            chain.as_str()
        )));
    }

    let users = PgUserRepo::new(state.db.clone());
    let custodial = users
        .get_custodial_wallet(user_id, chain.as_str())
        .await?
        .ok_or(AppError::NotFound("custodial wallet not found"))?;

    let bal = live_balance(&state, user_id, chain, true).await?;
    if bal.available_micro < body.amount_micro {
        return Err(AppError::InsufficientBalance {
            required_micro: body.amount_micro,
            available_micro: bal.available_micro,
        });
    }

    wallet_chain(&state)
        .acquire_withdraw_lock(user_id, 900)
        .await?;

    Ok(Json(WithdrawPrepareResponse {
        user_id: user_id.as_uuid(),
        amount_micro: body.amount_micro,
        from_address: custodial.address,
        to_address,
        usdcx_contract: state.config.usdcx_contract.clone(),
        usdcx_asset_name: USDCX_ASSET_NAME.to_owned(),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawCompleteBody {
    txid: String,
    #[serde(default)]
    chain: Option<String>,
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

    let chain = ChainId::from_optional(body.chain.as_deref());
    let svc = wallet_chain(&state);
    match chain {
        ChainId::Solana => crate::services::solana_vault::assert_tx_ok(&state, txid).await?,
        ChainId::Arbitrum => arbitrum_vault::assert_tx_ok(&state, txid).await?,
        ChainId::Botchain => botchain_vault::assert_tx_ok(&state, txid).await?,
        ChainId::Stacks => svc.hiro().require_tx_success(txid).await?,
    }
    svc.release_withdraw_lock(user_id).await?;
    let bal = live_balance(&state, user_id, chain, true).await?;
    let topic = format!("user:{}", user_id.as_uuid());
    state.subscriptions.publish(
        &state.sessions,
        &topic,
        crate::ws::ServerMessage {
            kind: "wallet.balance.updated".into(),
            payload: serde_json::json!({
                "availableMicro": bal.available_micro,
                "address": bal.address,
                "chain": chain.as_str(),
            }),
        },
    );
    Ok(Json(bal))
}

async fn abort_withdrawal(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    wallet_chain(&state)
        .release_withdraw_lock(auth.user_id)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SigningMaterial {
    id: Uuid,
    user_id: Uuid,
    address: String,
    public_key: String,
    network: String,
    chain: String,
    encrypted_signing_material: String,
    kms_key_version: String,
    usdcx_contract: String,
}

async fn get_signing_material(
    State(state): State<AppState>,
    _secret: InternalSecret,
    Path(user_id): Path<Uuid>,
    Query(query): Query<ChainQuery>,
) -> AppResult<Json<SigningMaterial>> {
    let secret = PgUserRepo::new(state.db.clone())
        .get_custodial_wallet_secret(UserId::from(user_id), &chain_param(&query))
        .await?
        .ok_or(AppError::NotFound("custodial wallet not found"))?;

    Ok(Json(SigningMaterial {
        id: secret.id,
        user_id: secret.user_id,
        address: secret.address,
        public_key: secret.public_key,
        network: secret.network,
        chain: secret.chain,
        encrypted_signing_material: secret.encrypted_signing_material,
        kms_key_version: secret.kms_key_version,
        usdcx_contract: state.config.usdcx_contract.clone(),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateEncryptionBody {
    encrypted_signing_material: String,
    kms_key_version: String,
    #[serde(default)]
    chain: Option<String>,
    /// Rewrite an already-v2 envelope (EVM AAD family migration).
    #[serde(default)]
    rewrap: bool,
}

async fn update_custodial_encryption(
    State(state): State<AppState>,
    _secret: InternalSecret,
    Path(user_id): Path<Uuid>,
    Json(body): Json<UpdateEncryptionBody>,
) -> AppResult<Json<serde_json::Value>> {
    if body.encrypted_signing_material.trim().is_empty() || body.kms_key_version.trim().is_empty() {
        return Err(AppError::BadRequest(
            "encryption fields must be non-empty".into(),
        ));
    }
    if !kms_key_uses_aad(&body.kms_key_version) {
        return Err(AppError::BadRequest(
            "encryption updates must use KMS key version 2+".into(),
        ));
    }

    let updated = PgUserRepo::new(state.db.clone())
        .update_custodial_wallet_encryption(
            UserId::from(user_id),
            ChainId::from_optional(body.chain.as_deref()).as_str(),
            body.encrypted_signing_material.trim(),
            body.kms_key_version.trim(),
            body.rewrap,
        )
        .await?;
    if !updated {
        return Err(AppError::NotFound("custodial wallet not found"));
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateCustodialWalletBody {
    address: String,
    public_key: String,
    encrypted_signing_material: String,
    kms_key_version: String,
    network: String,
    #[serde(default)]
    chain: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CustodialWalletResponse {
    user_id: Uuid,
    address: String,
    public_key: String,
    network: String,
    chain: String,
}

/// Winner claim path may provision the game dest's missing chain wallet.
async fn create_custodial_wallet_internal(
    State(state): State<AppState>,
    _secret: InternalSecret,
    Path(user_id): Path<Uuid>,
    Json(body): Json<CreateCustodialWalletBody>,
) -> AppResult<Json<CustodialWalletResponse>> {
    if body.address.trim().is_empty()
        || body.public_key.trim().is_empty()
        || body.encrypted_signing_material.trim().is_empty()
        || body.kms_key_version.trim().is_empty()
        || body.network.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "custodial wallet fields must be non-empty".into(),
        ));
    }
    if !kms_key_uses_aad(&body.kms_key_version) {
        return Err(AppError::BadRequest(
            "new custodial wallets must use KMS key version 2+".into(),
        ));
    }

    let chain = ChainId::from_optional(body.chain.as_deref())
        .as_str()
        .to_owned();

    let wallet = PgUserRepo::new(state.db.clone())
        .create_custodial_wallet(
            user_id.into(),
            CustodialWalletInput {
                address: body.address.trim().to_owned(),
                public_key: body.public_key.trim().to_owned(),
                encrypted_signing_material: body.encrypted_signing_material.trim().to_owned(),
                kms_key_version: body.kms_key_version.trim().to_owned(),
                network: body.network.trim().to_owned(),
                chain,
            },
        )
        .await?;

    Ok(Json(CustodialWalletResponse {
        user_id: wallet.user_id,
        address: wallet.address,
        public_key: wallet.public_key,
        network: wallet.network,
        chain: wallet.chain,
    }))
}
