//! Solana USDC balance + activity for custodial wallets.
//! Hiro stays in the Stacks adapter.

use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use sw_domain::{ChainActivityItem, ChainActivityKind, ChainId, UserId, WalletBalance};

use crate::data::users::PgUserRepo;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct RpcEnvelope<T> {
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct TokenAccounts {
    value: Vec<TokenAccount>,
}

#[derive(Debug, Deserialize)]
struct TokenAccount {
    pubkey: String,
    account: TokenAccountBody,
}

#[derive(Debug, Deserialize)]
struct TokenAccountBody {
    data: TokenAccountData,
}

#[derive(Debug, Deserialize)]
struct TokenAccountData {
    parsed: TokenParsed,
}

#[derive(Debug, Deserialize)]
struct TokenParsed {
    info: TokenInfo,
}

#[derive(Debug, Deserialize)]
struct TokenInfo {
    #[serde(rename = "tokenAmount")]
    token_amount: TokenAmount,
}

#[derive(Debug, Deserialize)]
struct TokenAmount {
    amount: String,
}

#[derive(Debug, Deserialize)]
struct SigInfo {
    signature: String,
    #[serde(rename = "blockTime")]
    block_time: Option<i64>,
}

pub async fn get_balance(state: &AppState, user_id: UserId) -> AppResult<WalletBalance> {
    let wallet = PgUserRepo::new(state.db.clone())
        .get_custodial_wallet(user_id, "solana")
        .await?
        .ok_or(AppError::NotFound("custodial wallet not found"))?;

    let available_micro = match fetch_usdc_amount(
        &state.config.solana_rpc_url,
        &state.config.solana_usdc_mint,
        &wallet.address,
    )
    .await
    {
        Ok(amount) => amount,
        Err(err) => {
            tracing::warn!(error = %err, "solana USDC balance read failed; serving 0");
            0
        }
    };

    Ok(WalletBalance {
        user_id,
        address: wallet.address,
        chain: wallet.chain.parse().unwrap_or(ChainId::Solana),
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
        .get_custodial_wallet(user_id, "solana")
        .await?
        .ok_or(AppError::NotFound("custodial wallet not found"))?;

    let atas = match fetch_atas(
        &state.config.solana_rpc_url,
        &state.config.solana_usdc_mint,
        &wallet.address,
    )
    .await
    {
        Ok(atas) => atas,
        Err(err) => {
            tracing::warn!(error = %err, "solana token accounts read failed");
            return Ok(Vec::new());
        }
    };
    if atas.is_empty() {
        return Ok(Vec::new());
    }

    let mut signatures: Vec<(String, Option<i64>)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for ata in &atas {
        match fetch_signatures(&state.config.solana_rpc_url, ata, limit.clamp(1, 50)).await {
            Ok(sigs) => {
                for sig in sigs {
                    if seen.insert(sig.signature.clone()) {
                        signatures.push((sig.signature, sig.block_time));
                    }
                }
            }
            Err(err) => tracing::warn!(error = %err, ata, "solana signatures read failed"),
        }
    }
    signatures.sort_by(|a, b| b.1.cmp(&a.1));
    signatures.truncate(limit.clamp(1, 50) as usize);

    let vault = state.config.solana_vault_program_id.as_str();
    let mint = state.config.solana_usdc_mint.as_str();
    let mut out = Vec::new();
    for (signature, _) in signatures {
        match fetch_parsed_tx(&state.config.solana_rpc_url, &signature).await {
            Ok(Some(tx)) => {
                out.extend(classify_tx(
                    &tx,
                    &signature,
                    &wallet.address,
                    &atas,
                    mint,
                    vault,
                ));
            }
            Ok(None) => {}
            Err(err) => tracing::warn!(error = %err, signature, "solana tx read failed"),
        }
    }
    out.sort_by(|a, b| b.block_time.cmp(&a.block_time));
    Ok(out)
}

async fn fetch_usdc_amount(rpc_url: &str, mint: &str, owner: &str) -> Result<i64, String> {
    let client = reqwest::Client::new();
    let envelope: RpcEnvelope<TokenAccounts> = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenAccountsByOwner",
            "params": [owner, { "mint": mint }, { "encoding": "jsonParsed" }]
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let Some(result) = envelope.result else {
        return Ok(0);
    };
    let mut total: i64 = 0;
    for account in result.value {
        let raw = account.account.data.parsed.info.token_amount.amount;
        total = total.saturating_add(raw.parse().unwrap_or(0));
    }
    Ok(total)
}

async fn fetch_atas(rpc_url: &str, mint: &str, owner: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::new();
    let envelope: RpcEnvelope<TokenAccounts> = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenAccountsByOwner",
            "params": [owner, { "mint": mint }, { "encoding": "jsonParsed" }]
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(envelope
        .result
        .map(|r| r.value.into_iter().map(|a| a.pubkey).collect())
        .unwrap_or_default())
}

async fn fetch_signatures(
    rpc_url: &str,
    address: &str,
    limit: u32,
) -> Result<Vec<SigInfo>, String> {
    let client = reqwest::Client::new();
    let envelope: RpcEnvelope<Vec<SigInfo>> = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSignaturesForAddress",
            "params": [address, { "limit": limit }]
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(envelope.result.unwrap_or_default())
}

async fn fetch_parsed_tx(rpc_url: &str, signature: &str) -> Result<Option<Value>, String> {
    let client = reqwest::Client::new();
    let envelope: RpcEnvelope<Value> = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTransaction",
            "params": [signature, {
                "encoding": "jsonParsed",
                "commitment": "confirmed",
                "maxSupportedTransactionVersion": 1
            }]
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(envelope.result)
}

struct TokenMove {
    ty: String,
    amount: i64,
    source: Option<String>,
    destination: Option<String>,
}

fn classify_tx(
    tx: &Value,
    signature: &str,
    owner: &str,
    atas: &[String],
    mint: &str,
    vault: &str,
) -> Vec<ChainActivityItem> {
    let meta = tx.get("meta").cloned().unwrap_or(Value::Null);
    let failed = meta.get("err").map(|e| !e.is_null()).unwrap_or(false);
    let status = if failed { "failed" } else { "confirmed" };
    let block_time = tx.get("blockTime").and_then(Value::as_i64);
    let logs = meta
        .get("logMessages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let vault_ix = instruction_name_from_logs(&logs, vault);
    let vault_touched = vault_program_in_logs(&logs, vault);
    let delta = owner_mint_delta(&meta, owner, mint);

    let mut moves = Vec::new();
    collect_token_moves(
        tx.pointer("/transaction/message/instructions")
            .and_then(Value::as_array),
        mint,
        &mut moves,
    );
    if let Some(inner) = meta.get("innerInstructions").and_then(Value::as_array) {
        for group in inner {
            collect_token_moves(
                group.get("instructions").and_then(Value::as_array),
                mint,
                &mut moves,
            );
        }
    }

    let ata_hit = |addr: Option<&String>| addr.is_some_and(|a| atas.iter().any(|x| x == a));
    let mine: Vec<&TokenMove> = moves
        .iter()
        .filter(|m| ata_hit(m.source.as_ref()) || ata_hit(m.destination.as_ref()))
        .collect();

    let mut inbound: i64 = mine
        .iter()
        .filter(|m| ata_hit(m.destination.as_ref()) && !ata_hit(m.source.as_ref()))
        .map(|m| m.amount)
        .sum();
    let mut outbound: i64 = mine
        .iter()
        .filter(|m| ata_hit(m.source.as_ref()) && !ata_hit(m.destination.as_ref()))
        .map(|m| m.amount)
        .sum();
    let minted: i64 = mine
        .iter()
        .filter(|m| m.ty.to_ascii_lowercase().contains("mintto") && ata_hit(m.destination.as_ref()))
        .map(|m| m.amount)
        .sum();

    // Parsed inner transfers are often missing on v0 / ALT txs. Balance
    // deltas still see the $50 mint and lobby joins.
    if inbound == 0 && delta > 0 {
        inbound = delta;
    }
    if outbound == 0 && delta < 0 {
        outbound = -delta;
    }

    if inbound <= 0 && outbound <= 0 && minted <= 0 {
        return Vec::new();
    }

    let mut items = Vec::new();
    match vault_ix.as_deref() {
        Some("Join") if outbound > 0 => items.push(item(
            signature,
            ChainActivityKind::VaultJoin,
            outbound,
            Some(owner),
            Some(vault),
            status,
            block_time,
        )),
        Some("Leave") if inbound > 0 => items.push(item(
            signature,
            ChainActivityKind::VaultLeave,
            inbound,
            Some(vault),
            Some(owner),
            status,
            block_time,
        )),
        Some("Kick") if inbound > 0 => items.push(item(
            signature,
            ChainActivityKind::VaultKick,
            inbound,
            Some(vault),
            Some(owner),
            status,
            block_time,
        )),
        Some("Claim") if inbound > 0 => {
            let max_out = moves
                .iter()
                .filter(|m| !m.ty.to_ascii_lowercase().contains("mintto") && m.amount > 0)
                .map(|m| m.amount)
                .max()
                .unwrap_or(0)
                .max(max_positive_owner_delta(&meta, mint));
            let mut to_me: Vec<i64> = mine
                .iter()
                .filter(|m| {
                    ata_hit(m.destination.as_ref()) && !m.ty.to_ascii_lowercase().contains("mintto")
                })
                .map(|m| m.amount)
                .filter(|a| *a > 0)
                .collect();
            if to_me.is_empty() {
                to_me.push(inbound);
            }
            for (kind, amount) in classify_claim_inflows(to_me, max_out) {
                items.push(item(
                    signature,
                    kind,
                    amount,
                    Some(vault),
                    Some(owner),
                    status,
                    block_time,
                ));
            }
        }
        _ => {
            if outbound > 0 && vault_touched {
                items.push(item(
                    signature,
                    ChainActivityKind::VaultJoin,
                    outbound,
                    Some(owner),
                    Some(vault),
                    status,
                    block_time,
                ));
            } else if inbound > 0 && vault_touched {
                let max_out = max_positive_owner_delta(&meta, mint).max(inbound);
                for (kind, amount) in classify_claim_inflows(vec![inbound], max_out) {
                    items.push(item(
                        signature,
                        kind,
                        amount,
                        Some(vault),
                        Some(owner),
                        status,
                        block_time,
                    ));
                }
            } else {
                if minted > 0 {
                    items.push(item(
                        signature,
                        ChainActivityKind::Deposit,
                        minted,
                        None,
                        Some(owner),
                        status,
                        block_time,
                    ));
                }
                let rest = if minted > 0 {
                    inbound.saturating_sub(minted)
                } else {
                    inbound
                };
                if rest > 0 {
                    items.push(item(
                        signature,
                        ChainActivityKind::Deposit,
                        rest,
                        mine.iter().find_map(|m| m.source.as_deref()),
                        Some(owner),
                        status,
                        block_time,
                    ));
                }
                if outbound > 0 {
                    items.push(item(
                        signature,
                        ChainActivityKind::Withdraw,
                        outbound,
                        Some(owner),
                        mine.iter().find_map(|m| m.destination.as_deref()),
                        status,
                        block_time,
                    ));
                }
            }
        }
    }
    items.retain(|row| row.amount_micro > 0);
    items
}

/// One claim tx pays winner + platform (2%) + optional game fee (≤5%).
/// A lone inbound used to be labeled Winnings, so dest-fee-only receipts
/// (game author, not winner) showed up in the wrong filter.
fn classify_claim_inflows(mut to_me: Vec<i64>, max_out: i64) -> Vec<(ChainActivityKind, i64)> {
    to_me.retain(|amount| *amount > 0);
    to_me.sort_unstable_by(|a, b| b.cmp(a));
    match to_me.as_slice() {
        [winnings, fee, rest @ ..] if *fee > 0 && *winnings > *fee => {
            let extra: i64 = rest.iter().copied().sum();
            vec![
                (ChainActivityKind::VaultClaim, *winnings),
                (ChainActivityKind::VaultDevFee, *fee + extra),
            ]
        }
        [only] if max_out > *only => {
            vec![(ChainActivityKind::VaultDevFee, *only)]
        }
        [only, ..] => vec![(ChainActivityKind::VaultClaim, *only)],
        [] => Vec::new(),
    }
}

fn item(
    txid: &str,
    kind: ChainActivityKind,
    amount_micro: i64,
    from: Option<&str>,
    to: Option<&str>,
    status: &str,
    block_time: Option<i64>,
) -> ChainActivityItem {
    ChainActivityItem {
        txid: txid.to_owned(),
        kind,
        amount_micro,
        from_address: from.map(str::to_owned),
        to_address: to.map(str::to_owned),
        lobby_path: None,
        status: status.to_owned(),
        block_time,
    }
}

fn instruction_name_from_logs(logs: &[Value], vault: &str) -> Option<String> {
    if vault.is_empty() {
        return None;
    }
    let invoke = format!("Program {vault} invoke");
    let mut in_vault = false;
    for log in logs {
        let Some(line) = log.as_str() else { continue };
        if line.starts_with(&invoke) {
            in_vault = true;
            continue;
        }
        if in_vault && let Some(name) = line.strip_prefix("Program log: Instruction: ") {
            return Some(name.trim().to_owned());
        }
        if line.starts_with(&format!("Program {vault} success"))
            || line.starts_with(&format!("Program {vault} failed"))
        {
            in_vault = false;
        }
    }
    None
}

fn vault_program_in_logs(logs: &[Value], vault: &str) -> bool {
    !vault.is_empty()
        && logs.iter().any(|log| {
            log.as_str()
                .is_some_and(|line| line.starts_with(&format!("Program {vault} invoke")))
        })
}

fn json_amount(value: Option<&Value>) -> i64 {
    let Some(value) = value else {
        return 0;
    };
    if let Some(s) = value.as_str() {
        return s.parse().unwrap_or(0);
    }
    if let Some(n) = value.as_i64() {
        return n;
    }
    if let Some(n) = value.as_u64() {
        return n.min(i64::MAX as u64) as i64;
    }
    0
}

fn token_balance_sum(meta: &Value, key: &str, owner: &str, mint: &str) -> i64 {
    meta.get(key)
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter(|row| {
                    row.get("mint").and_then(Value::as_str) == Some(mint)
                        && row.get("owner").and_then(Value::as_str) == Some(owner)
                })
                .map(|row| json_amount(row.pointer("/uiTokenAmount/amount")))
                .sum()
        })
        .unwrap_or(0)
}

fn owner_mint_delta(meta: &Value, owner: &str, mint: &str) -> i64 {
    token_balance_sum(meta, "postTokenBalances", owner, mint)
        - token_balance_sum(meta, "preTokenBalances", owner, mint)
}

fn max_positive_owner_delta(meta: &Value, mint: &str) -> i64 {
    use std::collections::HashMap;
    let mut pre = HashMap::<String, i64>::new();
    let mut post = HashMap::<String, i64>::new();
    let add = |map: &mut HashMap<String, i64>, key: &str| {
        if let Some(rows) = meta.get(key).and_then(Value::as_array) {
            for row in rows {
                if row.get("mint").and_then(Value::as_str) != Some(mint) {
                    continue;
                }
                let Some(owner) = row.get("owner").and_then(Value::as_str) else {
                    continue;
                };
                *map.entry(owner.to_owned()).or_insert(0) +=
                    json_amount(row.pointer("/uiTokenAmount/amount"));
            }
        }
    };
    add(&mut pre, "preTokenBalances");
    add(&mut post, "postTokenBalances");
    post.iter()
        .map(|(owner, after)| after - pre.get(owner).copied().unwrap_or(0))
        .filter(|delta| *delta > 0)
        .max()
        .unwrap_or(0)
}

fn collect_token_moves(instructions: Option<&Vec<Value>>, mint: &str, out: &mut Vec<TokenMove>) {
    let Some(instructions) = instructions else {
        return;
    };
    for ix in instructions {
        let parsed = ix.get("parsed");
        let ty = parsed
            .and_then(|p| p.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if ty.is_empty() {
            continue;
        }
        let info = parsed.and_then(|p| p.get("info"));
        let ix_mint = info
            .and_then(|i| i.get("mint"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !ix_mint.is_empty() && ix_mint != mint {
            continue;
        }
        let amount = json_amount(
            info.and_then(|i| i.pointer("/tokenAmount/amount"))
                .or_else(|| info.and_then(|i| i.get("amount"))),
        );
        if amount <= 0 && !ty.to_ascii_lowercase().contains("mintto") {
            continue;
        }
        out.push(TokenMove {
            ty,
            amount,
            source: info
                .and_then(|i| i.get("source").or(i.get("multisigSource")))
                .and_then(Value::as_str)
                .map(str::to_owned),
            destination: info
                .and_then(|i| i.get("destination").or(i.get("account")))
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dest_fee_only_claim_is_game_fee() {
        let got = classify_claim_inflows(vec![30_000], 6_510_000);
        assert_eq!(got, vec![(ChainActivityKind::VaultDevFee, 30_000)]);
    }

    #[test]
    fn winner_inbound_is_winnings() {
        let got = classify_claim_inflows(vec![6_510_000], 6_510_000);
        assert_eq!(got, vec![(ChainActivityKind::VaultClaim, 6_510_000)]);
    }

    #[test]
    fn winner_who_is_also_dev_splits_two_legs() {
        let got = classify_claim_inflows(vec![6_510_000, 350_000], 6_510_000);
        assert_eq!(
            got,
            vec![
                (ChainActivityKind::VaultClaim, 6_510_000),
                (ChainActivityKind::VaultDevFee, 350_000),
            ]
        );
    }

    fn balance_tx(
        logs: &[&str],
        owner: &str,
        mint: &str,
        pre: i64,
        post: i64,
        others: &[(&str, i64, i64)],
    ) -> Value {
        let mut pre_rows = vec![json!({
            "mint": mint,
            "owner": owner,
            "uiTokenAmount": { "amount": pre.to_string() }
        })];
        let mut post_rows = vec![json!({
            "mint": mint,
            "owner": owner,
            "uiTokenAmount": { "amount": post.to_string() }
        })];
        for (who, before, after) in others {
            pre_rows.push(json!({
                "mint": mint,
                "owner": who,
                "uiTokenAmount": { "amount": before.to_string() }
            }));
            post_rows.push(json!({
                "mint": mint,
                "owner": who,
                "uiTokenAmount": { "amount": after.to_string() }
            }));
        }
        json!({
            "blockTime": 1,
            "meta": {
                "err": null,
                "logMessages": logs,
                "preTokenBalances": pre_rows,
                "postTokenBalances": post_rows,
                "innerInstructions": []
            },
            "transaction": { "message": { "instructions": [] } }
        })
    }

    #[test]
    fn mint_deposit_shows_without_parsed_transfer() {
        let owner = "Owner1111111111111111111111111111111111111";
        let mint = "Mint11111111111111111111111111111111111111";
        let tx = balance_tx(
            &[
                "Program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA invoke [1]",
                "Program log: Instruction: MintToChecked",
                "Program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA success",
            ],
            owner,
            mint,
            0,
            50_000_000,
            &[],
        );
        let got = classify_tx(
            &tx,
            "sig",
            owner,
            &[],
            mint,
            "Vault1111111111111111111111111111111111111",
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, ChainActivityKind::Deposit);
        assert_eq!(got[0].amount_micro, 50_000_000);
    }

    #[test]
    fn join_debit_is_lobby_entry_even_without_parsed_cpi() {
        let owner = "Owner1111111111111111111111111111111111111";
        let mint = "Mint11111111111111111111111111111111111111";
        let vault = "Vault1111111111111111111111111111111111111";
        let invoke = format!("Program {vault} invoke [1]");
        let success = format!("Program {vault} success");
        let tx = balance_tx(
            &[
                &invoke,
                "Program log: Instruction: Join",
                "Program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA invoke [2]",
                "Program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA success",
                &success,
            ],
            owner,
            mint,
            50_000_000,
            45_000_000,
            &[],
        );
        let got = classify_tx(&tx, "sig", owner, &[], mint, vault);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, ChainActivityKind::VaultJoin);
        assert_eq!(got[0].amount_micro, 5_000_000);
    }

    #[test]
    fn dest_fee_claim_uses_balance_delta_when_moves_missing() {
        let owner = "Dev111111111111111111111111111111111111111";
        let winner = "Win111111111111111111111111111111111111111";
        let mint = "Mint11111111111111111111111111111111111111";
        let vault = "Vault1111111111111111111111111111111111111";
        let invoke = format!("Program {vault} invoke [1]");
        let success = format!("Program {vault} success");
        let tx = balance_tx(
            &[&invoke, "Program log: Instruction: Claim", &success],
            owner,
            mint,
            0,
            140_000,
            &[(winner, 0, 6_510_000)],
        );
        let got = classify_tx(&tx, "sig", owner, &[], mint, vault);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, ChainActivityKind::VaultDevFee);
        assert_eq!(got[0].amount_micro, 140_000);
    }
}
