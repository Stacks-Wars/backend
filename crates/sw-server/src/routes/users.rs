use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::data::matches::{LifetimeTotals, MatchHistoryItem, PgMatchRepo};
use crate::data::push::PushSubscriptionRepo;
use crate::data::seasons::{PgSeasonRepo, SeasonRepo};
use crate::data::stats::{PgStatsRepo, UserStatLine};
use crate::data::users::{
    CustodialWalletInput, PgUserRepo, UpdateProfileInput, UpsertUserInput, UserCard,
    kms_key_uses_aad,
};
use crate::data::vault_drafts::{VaultDraft, VaultDraftRepo};
use crate::error::{AppError, AppResult};
use crate::services::hiro::HiroClient;
use crate::services::neon_jwt::parse_neon_sub;
use crate::services::push;
use crate::state::AppState;
use redis::AsyncCommands;
use sw_domain::{ChainId, User, UserId};

/// User mutations — Write rate tier.
pub fn write_router() -> Router<AppState> {
    Router::new()
        .route("/", post(upsert_user))
        .route("/me", delete(delete_account))
        .route("/me/legal-accept", post(accept_legal))
        .route("/me/preferences", axum::routing::patch(update_preferences))
        .route("/me/push-subscription", post(save_push_subscription))
        .route("/me/push-subscription", delete(delete_push_subscription))
        .route("/me/push-notice", post(push_notice))
        .route("/me/vault-drafts", post(save_vault_draft))
        .route(
            "/me/vault-drafts/{kind}/{lobby_path}",
            delete(delete_vault_draft),
        )
        .route("/{user_id}", patch(update_profile))
        .route("/{user_id}/custodial-wallet", post(create_custodial_wallet))
}

/// User reads — Global tier only.
pub fn read_router() -> Router<AppState> {
    Router::new()
        .route("/cards", get(get_user_cards))
        .route("/by-username/{username}", get(get_user_by_username))
        .route("/username-available/{username}", get(check_username))
        .route("/me/vault-drafts", get(list_vault_drafts))
        .route("/me/vault-drafts/{kind}/{lobby_path}", get(get_vault_draft))
        .route("/{user_id}", get(get_user))
        .route("/{user_id}/profile", get(get_profile))
        .route("/{user_id}/matches", get(get_match_history))
        .route("/{user_id}/custodial-wallet", get(get_custodial_wallet))
        .route("/{user_id}/custodial-wallets", get(list_custodial_wallets))
}

/// 3–24 chars, lowercase alphanumeric plus `_` and `-`, must start with a letter.
fn validate_username(raw: &str) -> AppResult<String> {
    let username = raw.trim().to_lowercase();
    let len = username.chars().count();
    if !(3..=24).contains(&len) {
        return Err(AppError::BadRequest(
            "username must be 3–24 characters".into(),
        ));
    }
    if !username.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err(AppError::BadRequest(
            "username must start with a letter".into(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::BadRequest(
            "username may only contain letters, numbers, _ and -".into(),
        ));
    }
    Ok(username)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertUserBody {
    /// Neon Auth `sub` (UUID v7). Must match Bearer token subject.
    id: Uuid,
    email: String,
    display_name: Option<String>,
    avatar_url: Option<String>,
    email_verified_at: Option<DateTime<Utc>>,
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
struct UserResponse {
    id: Uuid,
    username: Option<String>,
    display_name: Option<String>,
    email: String,
    email_verified_at: Option<DateTime<Utc>>,
    avatar_url: Option<String>,
    lobby_alerts_enabled: bool,
    current_chain: ChainId,
    legal_accepted_at: Option<DateTime<Utc>>,
    legal_version: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl UserResponse {
    fn from_user_prefs(user: User, prefs: Option<crate::data::users::UserPrefs>) -> Self {
        let prefs = prefs.unwrap_or(crate::data::users::UserPrefs {
            lobby_alerts_enabled: true,
            current_chain: ChainId::default(),
            legal_accepted_at: None,
            legal_version: None,
            deleted_at: None,
        });
        Self {
            id: user.id.as_uuid(),
            username: user.username,
            display_name: user.display_name,
            email: user.email,
            email_verified_at: user.email_verified_at,
            avatar_url: user.avatar_url,
            lobby_alerts_enabled: prefs.lobby_alerts_enabled,
            current_chain: prefs.current_chain,
            legal_accepted_at: prefs.legal_accepted_at,
            legal_version: prefs.legal_version,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self::from_user_prefs(user, None)
    }
}

const LEGAL_VERSION: &str = "2026-08-21";

async fn json_user(repo: &PgUserRepo, user: User) -> AppResult<Json<UserResponse>> {
    let prefs = repo.prefs(user.id).await?;
    Ok(Json(UserResponse::from_user_prefs(user, prefs)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProfileBody {
    username: Option<String>,
    display_name: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardsQuery {
    /// Comma-separated user ids.
    ids: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ChainQuery {
    chain: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FavouriteGame {
    game_id: String,
    matches: i64,
    wins: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileResponse {
    user: UserResponse,
    lifetime: LifetimeTotals,
    recent_matches: Vec<MatchHistoryItem>,
    favourite_games: Vec<FavouriteGame>,
    stat_lines: Vec<UserStatLine>,
    current_season_id: Option<i32>,
    current_season_rank: Option<i64>,
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

async fn upsert_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UpsertUserBody>,
) -> AppResult<Json<UserResponse>> {
    let id = parse_neon_sub(&body.id.to_string())?;
    if id != auth.user_id.as_uuid() {
        return Err(AppError::Unauthorized("token subject mismatch"));
    }

    let email = body.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(AppError::BadRequest("valid email is required".into()));
    }

    let repo = PgUserRepo::new(state.db.clone());
    let user = repo
        .upsert(UpsertUserInput {
            id: UserId::from(id),
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

    json_user(&repo, user).await
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

    json_user(&repo, user).await
}

/// Batch public profile lookup: `GET /users/cards?ids=uuid,uuid`.
async fn get_user_cards(
    State(state): State<AppState>,
    Query(params): Query<CardsQuery>,
) -> AppResult<Json<Vec<UserCard>>> {
    let ids: Vec<Uuid> = params
        .ids
        .split(',')
        .filter_map(|raw| Uuid::parse_str(raw.trim()).ok())
        .take(100)
        .collect();

    let cards = PgUserRepo::new(state.db.clone()).cards(&ids).await?;
    Ok(Json(cards))
}

async fn get_user_by_username(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> AppResult<Json<UserResponse>> {
    let user = PgUserRepo::new(state.db.clone())
        .get_by_username(&username.trim().to_lowercase())
        .await?
        .ok_or(AppError::NotFound("user"))?;
    Ok(Json(UserResponse::from(user)))
}

async fn check_username(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let normalized = match validate_username(&username) {
        Ok(value) => value,
        Err(err) => {
            return Ok(Json(serde_json::json!({
                "available": false,
                "reason": err.to_string(),
            })));
        }
    };
    let available = PgUserRepo::new(state.db.clone())
        .username_available(&normalized)
        .await?;
    Ok(Json(serde_json::json!({
        "available": available,
        "username": normalized,
    })))
}

async fn update_profile(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
    Json(body): Json<UpdateProfileBody>,
) -> AppResult<Json<UserResponse>> {
    auth.require_self(user_id)?;

    let username = body
        .username
        .as_deref()
        .map(validate_username)
        .transpose()?;
    let display_name = body
        .display_name
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if let Some(name) = display_name.as_deref() {
        if name.chars().count() > 48 {
            return Err(AppError::BadRequest(
                "display name must be 48 characters or fewer".into(),
            ));
        }
    }

    let user = PgUserRepo::new(state.db.clone())
        .update_profile(
            UserId::from(user_id),
            UpdateProfileInput {
                username,
                display_name,
                avatar_url: body
                    .avatar_url
                    .map(|value| value.trim().to_owned())
                    .filter(|value| !value.is_empty()),
            },
        )
        .await?;

    Ok(Json(UserResponse::from(user)))
}

/// Everything the profile page needs in one round trip.
async fn get_profile(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> AppResult<Json<ProfileResponse>> {
    let user_id = UserId::from(user_id);
    let users = PgUserRepo::new(state.db.clone());
    let matches = PgMatchRepo::new(state.db.clone());
    let stats = PgStatsRepo::new(state.db.clone());
    let seasons = PgSeasonRepo::new(state.db.clone());

    // Two waves of concurrent queries: the rank lookup needs the season id,
    // everything else is independent.
    let (user, current_season) = tokio::try_join!(users.get_by_id(user_id), seasons.current())?;
    let user = user.ok_or(AppError::NotFound("user"))?;

    let season_id = current_season.as_ref().map(|season| season.id);
    let (lifetime, recent_matches, favourites, stat_lines, season_rank) = tokio::try_join!(
        matches.lifetime_totals(user_id),
        matches.history_for_user(user_id, 10, 0),
        matches.favourite_games(user_id, 5),
        stats.user_stat_lines(user_id),
        async {
            match season_id {
                Some(id) => stats.user_season_rank(user_id, id).await,
                None => Ok(None),
            }
        },
    )?;

    let favourite_games = favourites
        .into_iter()
        .map(|(game_id, matches, wins)| FavouriteGame {
            game_id,
            matches,
            wins,
        })
        .collect();

    Ok(Json(ProfileResponse {
        user: UserResponse::from(user),
        lifetime,
        recent_matches,
        favourite_games,
        stat_lines,
        current_season_id: current_season.map(|s| s.id.as_i32()),
        current_season_rank: season_rank,
    }))
}

async fn get_match_history(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Query(params): Query<HistoryQuery>,
) -> AppResult<Json<Vec<MatchHistoryItem>>> {
    let items = PgMatchRepo::new(state.db.clone())
        .history_for_user(
            UserId::from(user_id),
            params.limit.unwrap_or(20).clamp(1, 100),
            params.offset.unwrap_or(0).max(0),
        )
        .await?;
    Ok(Json(items))
}

async fn get_custodial_wallet(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
    Query(query): Query<ChainQuery>,
) -> AppResult<Json<CustodialWalletResponse>> {
    auth.require_self(user_id)?;

    let repo = PgUserRepo::new(state.db.clone());
    let chain = ChainId::from_optional(query.chain.as_deref());
    let chain = chain.as_str();

    let wallet = repo
        .get_custodial_wallet(user_id.into(), chain)
        .await?
        .ok_or(AppError::NotFound("custodial wallet"))?;

    Ok(Json(CustodialWalletResponse {
        user_id: wallet.user_id,
        address: wallet.address,
        public_key: wallet.public_key,
        network: wallet.network,
        chain: wallet.chain,
    }))
}

async fn list_custodial_wallets(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
) -> AppResult<Json<Vec<CustodialWalletResponse>>> {
    auth.require_self(user_id)?;
    let wallets = PgUserRepo::new(state.db.clone())
        .list_custodial_wallets(user_id.into())
        .await?;
    Ok(Json(
        wallets
            .into_iter()
            .map(|wallet| CustodialWalletResponse {
                user_id: wallet.user_id,
                address: wallet.address,
                public_key: wallet.public_key,
                network: wallet.network,
                chain: wallet.chain,
            })
            .collect(),
    ))
}

async fn create_custodial_wallet(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
    Json(body): Json<CreateCustodialWalletBody>,
) -> AppResult<Json<CustodialWalletResponse>> {
    auth.require_self(user_id)?;

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

    let repo = PgUserRepo::new(state.db.clone());
    let wallet = repo
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveVaultDraftBody {
    kind: String,
    lobby_path: String,
    lobby_id: Option<Uuid>,
    /// Empty for pending claim intents that have not broadcast yet.
    #[serde(default)]
    txid: String,
    entry_amount_micro: i64,
    transfer_micro: Option<i64>,
    sponsored: Option<bool>,
    name: Option<String>,
    description: Option<String>,
    game_id: Option<String>,
    is_private: Option<bool>,
    is_sponsored: Option<bool>,
    amount_micro: Option<i64>,
    nonce: Option<u64>,
    paid_micro: Option<i64>,
    dev_wallet: Option<String>,
    dev_fee: Option<i64>,
    dev_id: Option<Uuid>,
    dev_needs_wallet: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct VaultDraftQuery {
    kind: Option<String>,
}

async fn list_vault_drafts(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<VaultDraftQuery>,
) -> AppResult<Json<Vec<VaultDraft>>> {
    let repo = VaultDraftRepo::new(state.redis.clone());
    let mut drafts = repo.list(auth.user_id).await?;
    if let Some(kind) = query.kind.as_deref() {
        drafts.retain(|draft| draft.kind == kind);
    }
    Ok(Json(drafts))
}

async fn get_vault_draft(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((kind, lobby_path)): Path<(String, String)>,
) -> AppResult<Json<Option<VaultDraft>>> {
    Ok(Json(
        VaultDraftRepo::new(state.redis.clone())
            .get(auth.user_id, &kind, &lobby_path)
            .await?,
    ))
}

async fn save_vault_draft(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SaveVaultDraftBody>,
) -> AppResult<Json<VaultDraft>> {
    if body.lobby_path.trim().is_empty() {
        return Err(AppError::BadRequest("lobbyPath required".into()));
    }
    let kind = body.kind.trim().to_owned();
    // Join/leave/create drafts need a txid. Claim intents may be saved before broadcast.
    if kind != "claim" && body.txid.trim().is_empty() {
        return Err(AppError::BadRequest("txid required".into()));
    }
    let draft = VaultDraft {
        kind,
        user_id: auth.user_id.as_uuid(),
        lobby_path: body.lobby_path.trim().to_owned(),
        lobby_id: body.lobby_id,
        txid: body.txid.trim().to_owned(),
        entry_amount_micro: body.entry_amount_micro,
        transfer_micro: body.transfer_micro,
        sponsored: body.sponsored,
        name: body.name,
        description: body.description,
        game_id: body.game_id,
        is_private: body.is_private,
        is_sponsored: body.is_sponsored,
        amount_micro: body.amount_micro,
        nonce: body.nonce,
        paid_micro: body.paid_micro,
        dev_wallet: body.dev_wallet,
        dev_fee: body.dev_fee,
        dev_id: body.dev_id,
        dev_needs_wallet: body.dev_needs_wallet,
        created_at: Utc::now().timestamp(),
    };
    VaultDraftRepo::new(state.redis.clone())
        .save(&draft)
        .await?;
    Ok(Json(draft))
}

async fn delete_vault_draft(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((kind, lobby_path)): Path<(String, String)>,
) -> AppResult<Json<serde_json::Value>> {
    VaultDraftRepo::new(state.redis.clone())
        .delete(auth.user_id, &kind, &lobby_path)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegalAcceptBody {
    version: Option<String>,
}

async fn accept_legal(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<LegalAcceptBody>,
) -> AppResult<Json<UserResponse>> {
    let version = body.version.unwrap_or_else(|| LEGAL_VERSION.to_owned());
    let repo = PgUserRepo::new(state.db.clone());
    repo.accept_legal(auth.user_id, &version).await?;
    let user = repo
        .get_by_id(auth.user_id)
        .await?
        .ok_or(AppError::NotFound("user"))?;
    json_user(&repo, user).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreferencesBody {
    lobby_alerts_enabled: Option<bool>,
    current_chain: Option<String>,
}

async fn update_preferences(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<PreferencesBody>,
) -> AppResult<Json<UserResponse>> {
    let repo = PgUserRepo::new(state.db.clone());
    if let Some(enabled) = body.lobby_alerts_enabled {
        repo.set_lobby_alerts(auth.user_id, enabled).await?;
    }
    if let Some(raw) = body.current_chain {
        let chain: ChainId = raw.parse().map_err(|e| AppError::BadRequest(e))?;
        repo.set_current_chain(auth.user_id, chain).await?;
    }
    let user = repo
        .get_by_id(auth.user_id)
        .await?
        .ok_or(AppError::NotFound("user"))?;
    json_user(&repo, user).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushSubscriptionBody {
    endpoint: String,
    keys: PushKeysBody,
    user_agent: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushKeysBody {
    p256dh: String,
    auth: String,
}

async fn save_push_subscription(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<PushSubscriptionBody>,
) -> AppResult<Json<serde_json::Value>> {
    let endpoint = body.endpoint.trim();
    if !endpoint.starts_with("https://") {
        return Err(AppError::BadRequest("invalid push endpoint".into()));
    }
    PushSubscriptionRepo::new(state.db.clone())
        .upsert(
            auth.user_id,
            endpoint,
            body.keys.p256dh.trim(),
            body.keys.auth.trim(),
            body.user_agent.as_deref(),
        )
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeletePushBody {
    endpoint: String,
}

async fn delete_push_subscription(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<DeletePushBody>,
) -> AppResult<Json<serde_json::Value>> {
    PushSubscriptionRepo::new(state.db.clone())
        .delete_endpoint(auth.user_id, body.endpoint.trim())
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushNoticeBody {
    title: String,
    body: Option<String>,
    path: Option<String>,
}

async fn push_notice(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<PushNoticeBody>,
) -> AppResult<Json<serde_json::Value>> {
    let title = body.title.trim();
    if title.is_empty() || title.len() > 80 {
        return Err(AppError::BadRequest("title required".into()));
    }
    let notice_body = body
        .body
        .as_deref()
        .unwrap_or("")
        .trim()
        .chars()
        .take(200)
        .collect::<String>();
    let path = body.path.as_deref().unwrap_or("/wallet").trim().to_owned();
    if !path.starts_with('/') {
        return Err(AppError::BadRequest("path must be relative".into()));
    }
    push::spawn_user_notice(
        state.push.clone(),
        state.db.clone(),
        auth.user_id,
        title.to_owned(),
        notice_body,
        path,
    );
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete_account(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = auth.user_id;
    let repo = PgUserRepo::new(state.db.clone());
    let prefs = repo
        .prefs(user_id)
        .await?
        .ok_or(AppError::NotFound("user"))?;
    if prefs.deleted_at.is_some() {
        return Ok(Json(serde_json::json!({ "ok": true })));
    }

    let mut pending_claim_micro: i64 = 0;
    let drafts = VaultDraftRepo::new(state.redis.clone())
        .list(user_id)
        .await
        .unwrap_or_default();
    for draft in &drafts {
        if draft.kind == "claim" {
            pending_claim_micro += draft.amount_micro.unwrap_or(0);
        }
    }

    let mut available_micro: i64 = 0;
    if repo
        .get_custodial_wallet(user_id, "stacks")
        .await?
        .is_some()
    {
        let hiro = HiroClient::new(
            state.config.hiro_api_url.clone(),
            state.config.hiro_api_key.clone(),
            crate::config::USDCX_CONTRACT,
            crate::config::USDCX_ASSET_NAME,
            Some(state.config.sw_vault_contract.clone()),
        );
        if let Ok(bal) = crate::services::wallet_chain::WalletChainService::new(
            state.db.clone(),
            state.redis.clone(),
            hiro,
        )
        .get_balance(user_id)
        .await
        {
            available_micro = available_micro.saturating_add(bal.available_micro);
        }
    }
    if repo
        .get_custodial_wallet(user_id, "solana")
        .await?
        .is_some()
    {
        if let Ok(bal) = crate::services::solana_chain::get_balance(&state, user_id).await {
            available_micro = available_micro.saturating_add(bal.available_micro);
        }
    }

    if available_micro > 0 || pending_claim_micro > 0 {
        return Err(AppError::AccountDeleteBlocked {
            code: "funds_remaining",
            available_micro,
            pending_claim_micro,
        });
    }

    let lobbies = crate::data::lobbies::PgLobbyRepo::new(state.db.clone());
    if lobbies.has_active_participation(user_id).await? {
        return Err(AppError::AccountDeleteBlocked {
            code: "active_match",
            available_micro,
            pending_claim_micro,
        });
    }

    for lobby in lobbies.list_waiting_created_by(user_id).await? {
        crate::services::push::spawn_lobby_close(
            state.push.clone(),
            state.db.clone(),
            lobby.creator_id,
            lobby.path.clone(),
            lobby.chain,
            lobby.entry_amount_micro,
        );
        let _ = lobbies.delete(lobby.id).await;
    }

    PushSubscriptionRepo::new(state.db.clone())
        .delete_all_for_user(user_id)
        .await?;

    for draft in drafts {
        let _ = VaultDraftRepo::new(state.redis.clone())
            .delete(user_id, &draft.kind, &draft.lobby_path)
            .await;
    }

    let mut redis = state.redis.clone();
    let uid = user_id.as_uuid();
    let _: Result<(), _> = redis.del(format!("sw:balance:{uid}")).await;
    let _: Result<(), _> = redis.del(format!("sw:withdraw-lock:{uid}")).await;

    repo.anonymize(user_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
