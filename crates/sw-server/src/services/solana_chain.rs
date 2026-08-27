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

    let mut signatures = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for ata in &atas {
        match fetch_signatures(&state.config.solana_rpc_url, ata, limit.clamp(1, 50)).await {
            Ok(sigs) => {
                for sig in sigs {
                    if seen.insert(sig.clone()) {
                        signatures.push(sig);
                    }
                }
            }
            Err(err) => tracing::warn!(error = %err, ata, "solana signatures read failed"),
        }
    }
    signatures.truncate(limit.clamp(1, 50) as usize);

    let vault = state.config.solana_vault_program_id.as_str();
    let mint = state.config.solana_usdc_mint.as_str();
    let mut out = Vec::new();
    for signature in signatures {
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

async fn fetch_signatures(rpc_url: &str, address: &str, limit: u32) -> Result<Vec<String>, String> {
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
    Ok(envelope
        .result
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.signature)
        .collect())
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
    if mine.is_empty() {
        return Vec::new();
    }

    let inbound: i64 = mine
        .iter()
        .filter(|m| ata_hit(m.destination.as_ref()) && !ata_hit(m.source.as_ref()))
        .map(|m| m.amount)
        .sum();
    let outbound: i64 = mine
        .iter()
        .filter(|m| ata_hit(m.source.as_ref()) && !ata_hit(m.destination.as_ref()))
        .map(|m| m.amount)
        .sum();
    let minted: i64 = mine
        .iter()
        .filter(|m| m.ty.contains("mintTo") && ata_hit(m.destination.as_ref()))
        .map(|m| m.amount)
        .sum();

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
        Some("Claim") => {
            let mut to_me: Vec<i64> = mine
                .iter()
                .filter(|m| ata_hit(m.destination.as_ref()) && !m.ty.contains("mintTo"))
                .map(|m| m.amount)
                .filter(|a| *a > 0)
                .collect();
            to_me.sort_unstable_by(|a, b| b.cmp(a));
            match to_me.as_slice() {
                [winnings, fee, ..] if *fee > 0 && *winnings > *fee => {
                    items.push(item(
                        signature,
                        ChainActivityKind::VaultClaim,
                        *winnings,
                        Some(vault),
                        Some(owner),
                        status,
                        block_time,
                    ));
                    items.push(item(
                        signature,
                        ChainActivityKind::VaultDevFee,
                        *fee,
                        Some(vault),
                        Some(owner),
                        status,
                        block_time,
                    ));
                }
                [winnings, ..] => items.push(item(
                    signature,
                    ChainActivityKind::VaultClaim,
                    *winnings,
                    Some(vault),
                    Some(owner),
                    status,
                    block_time,
                )),
                _ => {}
            }
        }
        _ => {
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
            if inbound > minted {
                items.push(item(
                    signature,
                    ChainActivityKind::Deposit,
                    inbound - minted,
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
    items
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
    let mut in_vault = false;
    for log in logs {
        let Some(line) = log.as_str() else { continue };
        if line.contains(vault) && line.contains("invoke") {
            in_vault = true;
        }
        if in_vault {
            if let Some(name) = line.strip_prefix("Program log: Instruction: ") {
                return Some(name.trim().to_owned());
            }
        }
        if line.contains(vault) && line.contains("success") {
            in_vault = false;
        }
    }
    None
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
        let amount = info
            .and_then(|i| {
                i.pointer("/tokenAmount/amount")
                    .and_then(Value::as_str)
                    .or_else(|| i.get("amount").and_then(Value::as_str))
            })
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if amount <= 0 && !ty.contains("mintTo") {
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
