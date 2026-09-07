//! Arbitrum USDCx balance + activity for custodial wallets.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use futures::stream::{self, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use sw_domain::{ChainActivityItem, ChainActivityKind, ChainId, UserId, WalletBalance};

use crate::data::users::PgUserRepo;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const BALANCE_OF_SELECTOR: &str = "70a08231";
const PLATFORM_FEE_PCT: i64 = 2;
const LOG_LOOKBACK_BLOCKS: u64 = 1_500_000;

const TRANSFER_TOPIC: &str = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const JOINED_TOPIC: &str = "0x91a4dbdcbc6d322357bda498e04d2259453ee2b85812554356e6ae0b5935f3bd";
const LEFT_TOPIC: &str = "0x155988c63500f3f140b71291b6875b1ad7ccf75574bcddc52838719c5c46b1a5";
const KICKED_TOPIC: &str = "0xfaca43c7ec7199b4b8166d1b1c612724829cc5c12bfaec83d1c999496451fc0f";
const CLAIMED_TOPIC: &str = "0x1ffa94cad77738cd60e628f099441b4e921a2379f19dcc999d133f8c33d4b2c7";

#[derive(Debug, Deserialize)]
struct RpcEnvelope<T> {
    result: Option<T>,
    error: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct RpcLog {
    topics: Vec<String>,
    data: String,
    #[serde(default, rename = "blockNumber")]
    block_number: Option<String>,
    #[serde(default, rename = "transactionHash")]
    transaction_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcBlock {
    timestamp: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VaultKind {
    Join,
    Leave,
    Kick,
    Claim,
}

#[derive(Debug, Clone)]
struct TransferLog {
    txid: String,
    block_number: u64,
    from: String,
    to: String,
    amount: i64,
}

#[derive(Debug, Clone)]
struct VaultLog {
    kind: VaultKind,
    txid: String,
    block_number: u64,
    player: String,
    amount: i64,
    dest: String,
    dest_fee_pct: u8,
}

#[derive(Debug, Clone)]
struct StampedActivity {
    item: ChainActivityItem,
    block_number: u64,
}

pub async fn get_balance(state: &AppState, user_id: UserId) -> AppResult<WalletBalance> {
    let wallet = PgUserRepo::new(state.db.clone())
        .get_custodial_wallet(user_id, "arbitrum")
        .await?
        .ok_or(AppError::NotFound("custodial wallet not found"))?;

    let available_micro = match fetch_usdc_amount(
        &state.config.arbitrum_rpc_url,
        &state.config.arbitrum_usdc,
        &wallet.address,
    )
    .await
    {
        Ok(amount) => amount,
        Err(err) => {
            tracing::error!(error = %err, "arbitrum USDCx balance read failed");
            return Err(AppError::BadRequest(format!(
                "unable to query wallet balance ({err})"
            )));
        }
    };

    Ok(WalletBalance {
        user_id,
        address: wallet.address,
        chain: wallet.chain.parse().unwrap_or(ChainId::Arbitrum),
        available_micro,
        updated_at: Utc::now(),
        cached: false,
    })
}

pub async fn list_activity(
    state: &AppState,
    user_id: UserId,
    limit: u32,
) -> AppResult<Vec<ChainActivityItem>> {
    let wallet = PgUserRepo::new(state.db.clone())
        .get_custodial_wallet(user_id, "arbitrum")
        .await?
        .ok_or(AppError::NotFound("custodial wallet not found"))?;

    if undeployed(&state.config.arbitrum_usdc) {
        return Ok(Vec::new());
    }

    let cap = limit.clamp(1, 50);
    match fetch_activity(
        &state.config.arbitrum_rpc_url,
        &state.config.arbitrum_usdc,
        &state.config.arbitrum_vault,
        &state.config.arbitrum_platform_wallet,
        &wallet.address,
        cap,
    )
    .await
    {
        Ok(items) => Ok(items),
        Err(err) => {
            tracing::warn!(error = %err, "arbitrum activity read failed");
            Ok(Vec::new())
        }
    }
}

fn undeployed(contract: &str) -> bool {
    let value = contract.trim();
    value.is_empty() || value.bytes().filter(|b| b.is_ascii_hexdigit() && *b != b'0').count() == 0
}

async fn fetch_usdc_amount(rpc_url: &str, usdc: &str, owner: &str) -> Result<i64, String> {
    if undeployed(usdc) {
        return Ok(0);
    }
    let data = balance_of_data(owner)?;
    let to = normalize_address(usdc)?;
    let result: String = rpc_call(
        rpc_url,
        "eth_call",
        json!([{ "to": to, "data": data }, "latest"]),
    )
    .await?;
    parse_uint256(&result)
}

async fn fetch_activity(
    rpc_url: &str,
    usdc: &str,
    vault: &str,
    platform: &str,
    player: &str,
    limit: u32,
) -> Result<Vec<ChainActivityItem>, String> {
    let latest = parse_u64(&rpc_call::<String>(rpc_url, "eth_blockNumber", json!([])).await?)?;
    let from_block = latest.saturating_sub(LOG_LOOKBACK_BLOCKS);
    let player_topic = topic_address(player)?;
    let usdc_addr = normalize_address(usdc)?;

    let from_player = get_logs(
        rpc_url,
        &usdc_addr,
        json!([TRANSFER_TOPIC, player_topic.clone(), Value::Null]),
        from_block,
        latest,
    )
    .await?;
    let to_player = get_logs(
        rpc_url,
        &usdc_addr,
        json!([TRANSFER_TOPIC, Value::Null, player_topic.clone()]),
        from_block,
        latest,
    )
    .await?;

    let mut transfers = Vec::new();
    for log in from_player.into_iter().chain(to_player) {
        if let Some(parsed) = parse_transfer(&log) {
            transfers.push(parsed);
        }
    }
    transfers.sort_by(|a, b| a.txid.cmp(&b.txid).then(a.from.cmp(&b.from).then(a.to.cmp(&b.to))));
    transfers.dedup_by(|a, b| a.txid == b.txid && a.from == b.from && a.to == b.to && a.amount == b.amount);

    let mut vault_logs = Vec::new();
    if !undeployed(vault) {
        let vault_addr = normalize_address(vault)?;
        let logs = get_logs(
            rpc_url,
            &vault_addr,
            json!([Value::Null, Value::Null, player_topic]),
            from_block,
            latest,
        )
        .await?;
        vault_logs = logs.iter().filter_map(parse_vault).collect();
    }

    let mut stamped = classify_activity(player, vault, platform, &transfers, &vault_logs);
    stamped.sort_by(|a, b| {
        b.block_number
            .cmp(&a.block_number)
            .then(b.item.txid.cmp(&a.item.txid))
    });
    stamped.truncate(limit as usize);

    fill_block_times(rpc_url, &mut stamped).await;
    Ok(stamped.into_iter().map(|row| row.item).collect())
}

fn classify_activity(
    player: &str,
    vault: &str,
    platform: &str,
    transfers: &[TransferLog],
    vault_logs: &[VaultLog],
) -> Vec<StampedActivity> {
    let mut consumed: HashSet<String> = HashSet::new();
    let mut out = Vec::new();

    for log in vault_logs {
        if !same_addr(&log.player, player) {
            continue;
        }
        match log.kind {
            VaultKind::Join => {
                consume(&mut consumed, &log.txid, player, vault);
                out.push(stamped(
                    &log.txid,
                    log.block_number,
                    ChainActivityKind::VaultJoin,
                    log.amount,
                    Some(player),
                    Some(vault),
                ));
            }
            VaultKind::Leave => {
                consume(&mut consumed, &log.txid, vault, player);
                out.push(stamped(
                    &log.txid,
                    log.block_number,
                    ChainActivityKind::VaultLeave,
                    log.amount,
                    Some(vault),
                    Some(player),
                ));
            }
            VaultKind::Kick => {
                consume(&mut consumed, &log.txid, vault, player);
                out.push(stamped(
                    &log.txid,
                    log.block_number,
                    ChainActivityKind::VaultKick,
                    log.amount,
                    Some(vault),
                    Some(player),
                ));
            }
            VaultKind::Claim => {
                consume(&mut consumed, &log.txid, vault, player);
                let winner = transfer_amount(transfers, &log.txid, vault, player)
                    .unwrap_or_else(|| winner_amount(log.amount, &log.dest, log.dest_fee_pct, player, platform));
                if winner > 0 {
                    out.push(stamped(
                        &log.txid,
                        log.block_number,
                        ChainActivityKind::VaultClaim,
                        winner,
                        Some(vault),
                        Some(player),
                    ));
                }
            }
        }
    }

    for transfer in transfers {
        if same_addr(&transfer.to, player)
            && same_addr(&transfer.from, vault)
            && !consumed.contains(&transfer_key(&transfer.txid, vault, player))
        {
            consume(&mut consumed, &transfer.txid, vault, player);
            out.push(stamped(
                &transfer.txid,
                transfer.block_number,
                ChainActivityKind::VaultDevFee,
                transfer.amount,
                Some(vault),
                Some(player),
            ));
        }
    }

    for transfer in transfers {
        let key = transfer_key(&transfer.txid, &transfer.from, &transfer.to);
        if consumed.contains(&key) {
            continue;
        }
        if transfer.amount <= 0 {
            continue;
        }
        if same_addr(&transfer.to, player) && !same_addr(&transfer.from, vault) {
            out.push(stamped(
                &transfer.txid,
                transfer.block_number,
                ChainActivityKind::Deposit,
                transfer.amount,
                Some(&transfer.from),
                Some(player),
            ));
            continue;
        }
        if same_addr(&transfer.from, player)
            && !same_addr(&transfer.to, vault)
            && !is_zero_address(&transfer.to)
        {
            out.push(stamped(
                &transfer.txid,
                transfer.block_number,
                ChainActivityKind::Withdraw,
                transfer.amount,
                Some(player),
                Some(&transfer.to),
            ));
        }
    }

    out
}

fn winner_amount(pot: i64, dest: &str, dest_fee_pct: u8, player: &str, platform: &str) -> i64 {
    let platform_amt = pot.saturating_mul(PLATFORM_FEE_PCT) / 100;
    let dest_amt = if dest_fee_pct == 0
        || is_zero_address(dest)
        || same_addr(dest, player)
        || same_addr(dest, platform)
    {
        0
    } else {
        pot.saturating_mul(dest_fee_pct as i64) / 100
    };
    pot.saturating_sub(platform_amt).saturating_sub(dest_amt).max(0)
}

fn stamped(
    txid: &str,
    block_number: u64,
    kind: ChainActivityKind,
    amount: i64,
    from: Option<&str>,
    to: Option<&str>,
) -> StampedActivity {
    StampedActivity {
        block_number,
        item: ChainActivityItem {
            txid: txid.to_owned(),
            kind,
            amount_micro: amount,
            from_address: from.map(ToOwned::to_owned),
            to_address: to.map(ToOwned::to_owned),
            lobby_path: None,
            status: "success".into(),
            block_time: None,
        },
    }
}

fn consume(consumed: &mut HashSet<String>, txid: &str, from: &str, to: &str) {
    consumed.insert(transfer_key(txid, from, to));
}

fn transfer_key(txid: &str, from: &str, to: &str) -> String {
    format!(
        "{}:{}:{}",
        txid.to_ascii_lowercase(),
        normalize_address(from).unwrap_or_default(),
        normalize_address(to).unwrap_or_default()
    )
}

fn transfer_amount(transfers: &[TransferLog], txid: &str, from: &str, to: &str) -> Option<i64> {
    transfers.iter().find_map(|row| {
        if row.txid.eq_ignore_ascii_case(txid)
            && same_addr(&row.from, from)
            && same_addr(&row.to, to)
            && row.amount > 0
        {
            Some(row.amount)
        } else {
            None
        }
    })
}

async fn fill_block_times(rpc_url: &str, rows: &mut [StampedActivity]) {
    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for row in rows.iter() {
        if row.block_number != 0 && seen.insert(row.block_number) {
            unique.push(row.block_number);
        }
    }
    let resolved: HashMap<u64, i64> = stream::iter(unique)
        .map(|block_number| {
            let rpc_url = rpc_url.to_owned();
            async move {
                let hex = format!("0x{block_number:x}");
                let ts = match rpc_call::<RpcBlock>(
                    &rpc_url,
                    "eth_getBlockByNumber",
                    json!([hex, false]),
                )
                .await
                {
                    Ok(block) => block
                        .timestamp
                        .as_deref()
                        .and_then(|v| parse_u64(v).ok())
                        .map(|n| n as i64),
                    Err(err) => {
                        tracing::debug!(
                            error = %err,
                            block = block_number,
                            "arbitrum block timestamp skipped"
                        );
                        None
                    }
                };
                (block_number, ts)
            }
        })
        .buffer_unordered(8)
        .filter_map(|(block_number, ts)| async move { ts.map(|t| (block_number, t)) })
        .collect()
        .await;
    for row in rows.iter_mut() {
        if let Some(&ts) = resolved.get(&row.block_number) {
            row.item.block_time = Some(ts);
        }
    }
}

async fn get_logs(
    rpc_url: &str,
    address: &str,
    topics: Value,
    from_block: u64,
    to_block: u64,
) -> Result<Vec<RpcLog>, String> {
    if from_block > to_block {
        return Ok(Vec::new());
    }
    let params = json!([{
        "fromBlock": format!("0x{from_block:x}"),
        "toBlock": format!("0x{to_block:x}"),
        "address": address,
        "topics": topics,
    }]);
    match rpc_call::<Vec<RpcLog>>(rpc_url, "eth_getLogs", params.clone()).await {
        Ok(logs) => Ok(logs),
        Err(err) if should_split_log_range(&err) && to_block > from_block => {
            let mid = from_block + (to_block - from_block) / 2;
            let mut left = Box::pin(get_logs(rpc_url, address, topics.clone(), from_block, mid)).await?;
            let right = Box::pin(get_logs(rpc_url, address, topics, mid + 1, to_block)).await?;
            left.extend(right);
            Ok(left)
        }
        Err(err) => Err(err),
    }
}

fn should_split_log_range(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("block range")
        || lower.contains("too large")
        || lower.contains("query returned more")
        || lower.contains("log response size")
        || lower.contains("timeout")
        || lower.contains("-32005")
        || lower.contains("limit exceeded")
}

fn parse_transfer(log: &RpcLog) -> Option<TransferLog> {
    if log.topics.len() < 3 || !topic_eq(&log.topics[0], TRANSFER_TOPIC) {
        return None;
    }
    Some(TransferLog {
        txid: log.transaction_hash.clone().unwrap_or_default(),
        block_number: log.block_number.as_deref().and_then(|v| parse_u64(v).ok()).unwrap_or(0),
        from: addr_from_topic(&log.topics[1])?,
        to: addr_from_topic(&log.topics[2])?,
        amount: parse_uint256(&log.data).ok().unwrap_or(0),
    })
}

fn parse_vault(log: &RpcLog) -> Option<VaultLog> {
    if log.topics.len() < 3 {
        return None;
    }
    let kind = if topic_eq(&log.topics[0], JOINED_TOPIC) {
        VaultKind::Join
    } else if topic_eq(&log.topics[0], LEFT_TOPIC) {
        VaultKind::Leave
    } else if topic_eq(&log.topics[0], KICKED_TOPIC) {
        VaultKind::Kick
    } else if topic_eq(&log.topics[0], CLAIMED_TOPIC) {
        VaultKind::Claim
    } else {
        return None;
    };
    let amount = word_uint(&log.data, 0).unwrap_or(0);
    let (dest, dest_fee_pct) = if kind == VaultKind::Claim {
        (
            word_addr(&log.data, 1).unwrap_or_else(|| "0x0000000000000000000000000000000000000000".into()),
            word_uint(&log.data, 2).unwrap_or(0).clamp(0, 255) as u8,
        )
    } else {
        ("0x0000000000000000000000000000000000000000".into(), 0)
    };
    Some(VaultLog {
        kind,
        txid: log.transaction_hash.clone().unwrap_or_default(),
        block_number: log.block_number.as_deref().and_then(|v| parse_u64(v).ok()).unwrap_or(0),
        player: addr_from_topic(&log.topics[2])?,
        amount,
        dest,
        dest_fee_pct,
    })
}

fn topic_eq(actual: &str, expected: &str) -> bool {
    strip0x(actual)
        .ok()
        .zip(strip0x(expected).ok())
        .is_some_and(|(a, b)| a == b)
}

fn topic_address(address: &str) -> Result<String, String> {
    let hex = strip0x(address)?;
    if hex.len() != 40 {
        return Err("invalid Arbitrum address".into());
    }
    Ok(format!("0x{hex:0>64}"))
}

fn addr_from_topic(topic: &str) -> Option<String> {
    let hex = strip0x(topic).ok()?;
    if hex.len() < 40 {
        return None;
    }
    Some(format!("0x{}", &hex[hex.len() - 40..]))
}

fn word_hex(data: &str, index: usize) -> Option<String> {
    let body = strip0x(data).ok()?;
    let start = index.checked_mul(64)?;
    let end = start.checked_add(64)?;
    if body.len() < end {
        return None;
    }
    Some(body[start..end].to_owned())
}

fn word_uint(data: &str, index: usize) -> Option<i64> {
    parse_uint256(&format!("0x{}", word_hex(data, index)?)).ok()
}

fn word_addr(data: &str, index: usize) -> Option<String> {
    let word = word_hex(data, index)?;
    Some(format!("0x{}", &word[word.len() - 40..]))
}

fn same_addr(a: &str, b: &str) -> bool {
    match (strip0x(a), strip0x(b)) {
        (Ok(left), Ok(right)) => left == right,
        _ => a.eq_ignore_ascii_case(b),
    }
}

fn is_zero_address(address: &str) -> bool {
    strip0x(address)
        .map(|hex| hex.chars().all(|c| c == '0'))
        .unwrap_or(false)
}

fn balance_of_data(owner: &str) -> Result<String, String> {
    let hex = strip0x(owner)?;
    if hex.len() != 40 {
        return Err("invalid Arbitrum address".into());
    }
    Ok(format!("0x{BALANCE_OF_SELECTOR}000000000000000000000000{hex}"))
}

fn normalize_address(address: &str) -> Result<String, String> {
    let hex = strip0x(address)?;
    if hex.len() != 40 {
        return Err("invalid contract address".into());
    }
    Ok(format!("0x{hex}"))
}

fn strip0x(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed)
        .to_ascii_lowercase();
    if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid hex address".into());
    }
    Ok(hex)
}

fn parse_uint256(hex: &str) -> Result<i64, String> {
    let body = hex.trim().trim_start_matches("0x").trim_start_matches("0X");
    if body.is_empty() {
        return Ok(0);
    }
    let n = u128::from_str_radix(body, 16).map_err(|e| format!("token amount: {e}"))?;
    Ok(n.min(i64::MAX as u128) as i64)
}

fn parse_u64(hex: &str) -> Result<u64, String> {
    let body = hex.trim().trim_start_matches("0x").trim_start_matches("0X");
    if body.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(body, 16).map_err(|e| format!("block number: {e}"))
}

async fn rpc_call<T: serde::de::DeserializeOwned>(
    rpc_url: &str,
    method: &str,
    params: Value,
) -> Result<T, String> {
    let client = reqwest::Client::new();
    let envelope: RpcEnvelope<T> = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(err) = envelope.error {
        return Err(format!("{method}: {err}"));
    }
    envelope
        .result
        .ok_or_else(|| format!("{method} returned no result"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: &str = "0x6f734f6d4221c7b18667ebd5f40f19705925b062";
    const VAULT: &str = "0xbea87817c05ab529e62bead046a151cdf46d1e19";
    const PLATFORM: &str = "0x2092cd008184dd0e01c90aa14b132b468a69745e";
    const DEST: &str = "0x1111111111111111111111111111111111111111";
    const ZERO: &str = "0x0000000000000000000000000000000000000000";

    fn transfer(txid: &str, from: &str, to: &str, amount: i64) -> TransferLog {
        TransferLog {
            txid: txid.into(),
            block_number: 10,
            from: from.into(),
            to: to.into(),
            amount,
        }
    }

    fn vault(kind: VaultKind, txid: &str, amount: i64, dest: &str, dest_fee_pct: u8) -> VaultLog {
        VaultLog {
            kind,
            txid: txid.into(),
            block_number: 10,
            player: PLAYER.into(),
            amount,
            dest: dest.into(),
            dest_fee_pct,
        }
    }

    fn kinds(rows: &[StampedActivity]) -> Vec<(ChainActivityKind, i64)> {
        rows.iter().map(|row| (row.item.kind, row.item.amount_micro)).collect()
    }

    #[test]
    fn packs_balance_of() {
        let data = balance_of_data("0x2092cD008184Dd0E01C90Aa14b132B468a69745E").unwrap();
        assert!(data.starts_with("0x70a08231"));
        assert_eq!(data.len(), 2 + 8 + 64);
        assert!(data.ends_with("2092cd008184dd0e01c90aa14b132b468a69745e"));
    }

    #[test]
    fn empty_contract_is_undeployed() {
        assert!(undeployed(""));
        assert!(undeployed("0x0000000000000000000000000000000000000000"));
        assert!(!undeployed("0x2092cD008184Dd0E01C90Aa14b132B468a69745E"));
    }

    #[test]
    fn pads_address_topic() {
        assert_eq!(
            topic_address("0x2092cD008184Dd0E01C90Aa14b132B468a69745E").unwrap(),
            "0x0000000000000000000000002092cd008184dd0e01c90aa14b132b468a69745e"
        );
    }

    #[test]
    fn mint_is_deposit() {
        let rows = classify_activity(
            PLAYER,
            VAULT,
            PLATFORM,
            &[transfer("0xmint", ZERO, PLAYER, 50_000_000)],
            &[],
        );
        assert_eq!(kinds(&rows), vec![(ChainActivityKind::Deposit, 50_000_000)]);
    }

    #[test]
    fn inbound_transfer_is_deposit() {
        let rows = classify_activity(
            PLAYER,
            VAULT,
            PLATFORM,
            &[transfer("0xdep", DEST, PLAYER, 1_000_000)],
            &[],
        );
        assert_eq!(kinds(&rows), vec![(ChainActivityKind::Deposit, 1_000_000)]);
    }

    #[test]
    fn join_hides_matching_transfer() {
        let rows = classify_activity(
            PLAYER,
            VAULT,
            PLATFORM,
            &[transfer("0xjoin", PLAYER, VAULT, 1_000_000)],
            &[vault(VaultKind::Join, "0xjoin", 1_000_000, ZERO, 0)],
        );
        assert_eq!(kinds(&rows), vec![(ChainActivityKind::VaultJoin, 1_000_000)]);
    }

    #[test]
    fn withdraw_is_outbound_not_to_vault() {
        let rows = classify_activity(
            PLAYER,
            VAULT,
            PLATFORM,
            &[transfer("0xwd", PLAYER, DEST, 2_000_000)],
            &[],
        );
        assert_eq!(kinds(&rows), vec![(ChainActivityKind::Withdraw, 2_000_000)]);
    }

    #[test]
    fn claim_uses_winner_transfer_not_pot() {
        let pot = 3_000_000;
        let winner = 2_940_000;
        let rows = classify_activity(
            PLAYER,
            VAULT,
            PLATFORM,
            &[transfer("0xclaim", VAULT, PLAYER, winner)],
            &[vault(VaultKind::Claim, "0xclaim", pot, ZERO, 0)],
        );
        assert_eq!(kinds(&rows), vec![(ChainActivityKind::VaultClaim, winner)]);
    }

    #[test]
    fn dest_fee_without_winning() {
        let rows = classify_activity(
            PLAYER,
            VAULT,
            PLATFORM,
            &[transfer("0xfee", VAULT, PLAYER, 150_000)],
            &[],
        );
        assert_eq!(kinds(&rows), vec![(ChainActivityKind::VaultDevFee, 150_000)]);
    }

    #[test]
    fn leave_is_refund() {
        let rows = classify_activity(
            PLAYER,
            VAULT,
            PLATFORM,
            &[transfer("0xleave", VAULT, PLAYER, 1_000_000)],
            &[vault(VaultKind::Leave, "0xleave", 1_000_000, ZERO, 0)],
        );
        assert_eq!(kinds(&rows), vec![(ChainActivityKind::VaultLeave, 1_000_000)]);
    }

    #[test]
    fn parses_claimed_log_data() {
        let amount = format!("{:064x}", 3_000_000u64);
        let dest = format!("{:0>64}", &DEST[2..]);
        let pct = format!("{:064x}", 5u64);
        let log = RpcLog {
            topics: vec![
                CLAIMED_TOPIC.into(),
                "0xa4ed19e3cdde40271400cb997c0fa398e0dd0e7f5535d6f583ddcda32649c543".into(),
                topic_address(PLAYER).unwrap(),
            ],
            data: format!("0x{amount}{dest}{pct}"),
            block_number: Some("0xa".into()),
            transaction_hash: Some("0xclaim".into()),
        };
        let parsed = parse_vault(&log).unwrap();
        assert_eq!(parsed.kind, VaultKind::Claim);
        assert_eq!(parsed.amount, 3_000_000);
        assert!(same_addr(&parsed.dest, DEST));
        assert_eq!(parsed.dest_fee_pct, 5);
        assert_eq!(winner_amount(parsed.amount, &parsed.dest, parsed.dest_fee_pct, PLAYER, PLATFORM), 2_790_000);
    }
}
