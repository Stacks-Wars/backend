//! Solana USDC balance + activity for custodial wallets.
//! Hiro stays in the Stacks adapter.

use std::str::FromStr;

use base64::Engine;
use chrono::Utc;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use solana_pubkey::Pubkey;
use sw_domain::{ChainActivityItem, ChainActivityKind, ChainId, UserId, WalletBalance};

use crate::data::users::PgUserRepo;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

#[derive(Debug, Deserialize)]
struct RpcEnvelope<T> {
    result: Option<T>,
    error: Option<RpcErrorBody>,
}

#[derive(Debug, Deserialize)]
struct RpcErrorBody {
    code: Option<i64>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HeliusTxPage {
    data: Vec<Value>,
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

    let available_micro = fetch_usdc_amount(
        &state.config.solana_rpc_url,
        &state.config.solana_usdc_mint,
        &wallet.address,
    )
    .await
    .map_err(|err| {
        tracing::error!(error = %err, "solana USDC balance read failed");
        AppError::BadRequest(format!("unable to query wallet balance ({err})"))
    })?;

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

    let ata = match usdc_ata(&wallet.address, &state.config.solana_usdc_mint) {
        Ok(ata) => ata,
        Err(err) => {
            tracing::warn!(error = %err, "solana USDC ATA derive failed");
            return Ok(Vec::new());
        }
    };
    let cap = limit.clamp(1, 50);
    let txs =
        match fetch_activity_txs(&state.config.solana_rpc_url, &wallet.address, &ata, cap).await {
            Ok(txs) => txs,
            Err(err) => {
                tracing::warn!(error = %err, ata, "solana activity read failed");
                return Ok(Vec::new());
            }
        };

    let vault = state.config.solana_vault_program_id.as_str();
    let mint = state.config.solana_usdc_mint.as_str();
    let atas = [ata];
    let mut out = Vec::new();
    for (signature, tx) in txs {
        out.extend(classify_tx(
            &tx,
            &signature,
            &wallet.address,
            &atas,
            mint,
            vault,
        ));
    }
    out.sort_by(|a, b| b.block_time.cmp(&a.block_time));
    Ok(out)
}

fn usdc_ata(owner: &str, mint: &str) -> Result<String, String> {
    let owner = Pubkey::from_str(owner).map_err(|e| e.to_string())?;
    let mint = Pubkey::from_str(mint).map_err(|e| e.to_string())?;
    let token = Pubkey::from_str(TOKEN_PROGRAM).map_err(|e| e.to_string())?;
    let ata_program = Pubkey::from_str(ATA_PROGRAM).map_err(|e| e.to_string())?;
    let (ata, _) = Pubkey::find_program_address(
        &[owner.as_ref(), token.as_ref(), mint.as_ref()],
        &ata_program,
    );
    Ok(ata.to_string())
}

fn rpc_error_message(err: &RpcErrorBody) -> String {
    let msg = err.message.as_deref().unwrap_or("rpc error");
    match err.code {
        Some(code) => format!("{msg} ({code})"),
        None => msg.to_owned(),
    }
}

async fn rpc_call<T: DeserializeOwned>(
    rpc_url: &str,
    method: &str,
    params: Value,
) -> Result<T, String> {
    let envelope = rpc_envelope::<T>(
        rpc_url,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }),
    )
    .await?;
    if let Some(err) = envelope.error {
        return Err(format!("{method}: {}", rpc_error_message(&err)));
    }
    envelope
        .result
        .ok_or_else(|| format!("{method} returned no result"))
}

async fn rpc_envelope<T: DeserializeOwned>(
    rpc_url: &str,
    body: &Value,
) -> Result<RpcEnvelope<T>, String> {
    let client = reqwest::Client::new();
    client
        .post(rpc_url)
        .json(body)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

/// SPL amount for `mint`. Missing ATA is 0, not an RPC failure.
async fn fetch_usdc_amount(rpc_url: &str, mint: &str, owner: &str) -> Result<i64, String> {
    let result: Value = rpc_call(
        rpc_url,
        "getTokenAccountsByOwner",
        json!([
            owner,
            { "mint": mint },
            { "encoding": "jsonParsed", "commitment": "confirmed" }
        ]),
    )
    .await?;
    let Some(accounts) = result.get("value").and_then(Value::as_array) else {
        return Ok(0);
    };
    let Some(first) = accounts.first() else {
        return Ok(0);
    };
    let amount = first
        .pointer("/account/data/parsed/info/tokenAmount/amount")
        .and_then(Value::as_str)
        .unwrap_or("0");
    amount.parse().map_err(|e| format!("token amount: {e}"))
}

async fn fetch_signatures(
    rpc_url: &str,
    address: &str,
    limit: u32,
) -> Result<Vec<SigInfo>, String> {
    rpc_call(
        rpc_url,
        "getSignaturesForAddress",
        json!([address, { "limit": limit }]),
    )
    .await
}

fn tx_rpc_opts() -> Value {
    json!({
        "encoding": "jsonParsed",
        "commitment": "confirmed",
        "maxSupportedTransactionVersion": 1
    })
}

async fn fetch_activity_txs(
    rpc_url: &str,
    owner: &str,
    ata: &str,
    limit: u32,
) -> Result<Vec<(String, Value)>, String> {
    match fetch_helius_full_txs(rpc_url, owner, limit).await {
        Ok(txs) => return Ok(txs),
        Err(err) => tracing::warn!(
            error = %err,
            "helius getTransactionsForAddress unavailable; using signatures"
        ),
    }
    fetch_txs_by_signatures(rpc_url, ata, limit).await
}

async fn fetch_helius_full_txs(
    rpc_url: &str,
    owner: &str,
    limit: u32,
) -> Result<Vec<(String, Value)>, String> {
    let page: HeliusTxPage = rpc_call(
        rpc_url,
        "getTransactionsForAddress",
        json!([
            owner,
            {
                "transactionDetails": "full",
                "sortOrder": "desc",
                "limit": limit,
                "encoding": "jsonParsed",
                "maxSupportedTransactionVersion": 1,
                "commitment": "confirmed",
                "filters": {
                    "tokenAccounts": "balanceChanged"
                }
            }
        ]),
    )
    .await?;
    let mut out = Vec::new();
    for item in page.data {
        let signature = item
            .get("signature")
            .and_then(Value::as_str)
            .or_else(|| {
                item.pointer("/transaction/signatures/0")
                    .and_then(Value::as_str)
            })
            .unwrap_or("")
            .to_owned();
        if signature.is_empty() || item.get("meta").is_none() {
            continue;
        }
        out.push((
            signature,
            json!({
                "blockTime": item.get("blockTime"),
                "meta": item.get("meta"),
                "transaction": item.get("transaction"),
            }),
        ));
    }
    Ok(out)
}

async fn fetch_txs_by_signatures(
    rpc_url: &str,
    ata: &str,
    limit: u32,
) -> Result<Vec<(String, Value)>, String> {
    let sigs = fetch_signatures(rpc_url, ata, limit).await?;
    if sigs.is_empty() {
        return Ok(Vec::new());
    }
    match fetch_parsed_txs_batch(rpc_url, &sigs).await {
        Ok(txs) => Ok(txs),
        Err(err) => {
            tracing::warn!(error = %err, "solana batched getTransaction failed");
            let mut out = Vec::new();
            for sig in sigs {
                match fetch_parsed_tx(rpc_url, &sig.signature).await {
                    Ok(Some(tx)) => out.push((sig.signature, tx)),
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, signature = sig.signature, "solana tx read failed")
                    }
                }
            }
            Ok(out)
        }
    }
}

async fn fetch_parsed_txs_batch(
    rpc_url: &str,
    sigs: &[SigInfo],
) -> Result<Vec<(String, Value)>, String> {
    let body: Vec<Value> = sigs
        .iter()
        .enumerate()
        .map(|(i, sig)| {
            json!({
                "jsonrpc": "2.0",
                "id": i,
                "method": "getTransaction",
                "params": [sig.signature, tx_rpc_opts()]
            })
        })
        .collect();
    let client = reqwest::Client::new();
    let envelopes: Vec<RpcEnvelope<Value>> = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(err) = envelopes.iter().find_map(|e| e.error.as_ref()) {
        return Err(format!("getTransaction: {}", rpc_error_message(err)));
    }
    let mut out = Vec::new();
    for (i, envelope) in envelopes.into_iter().enumerate() {
        if let Some(tx) = envelope.result {
            if let Some(sig) = sigs.get(i) {
                out.push((sig.signature.clone(), tx));
            }
        }
    }
    Ok(out)
}

async fn fetch_parsed_tx(rpc_url: &str, signature: &str) -> Result<Option<Value>, String> {
    let envelope = rpc_envelope::<Value>(
        rpc_url,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTransaction",
            "params": [signature, {
                "encoding": "jsonParsed",
                "commitment": "confirmed",
                "maxSupportedTransactionVersion": 1
            }]
        }),
    )
    .await?;
    if let Some(err) = envelope.error {
        return Err(format!("getTransaction: {}", rpc_error_message(&err)));
    }
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
    let mine_events: Vec<VaultEvent> = parse_vault_claim_events(&logs)
        .into_iter()
        .filter(|event| event.recipient == owner)
        .collect();
    if !failed && !mine_events.is_empty() {
        return mine_events
            .into_iter()
            .map(|event| {
                item(
                    signature,
                    event.kind,
                    event.amount,
                    Some(vault),
                    Some(owner),
                    status,
                    block_time,
                )
            })
            .filter(|row| row.amount_micro > 0)
            .collect();
    }
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
            } else if inbound <= 0 || !vault_touched {
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

struct VaultEvent {
    kind: ChainActivityKind,
    recipient: String,
    amount: i64,
}

fn event_discriminator(name: &str) -> [u8; 8] {
    let hash = Sha256::digest(format!("event:{name}").as_bytes());
    hash[..8].try_into().expect("sha256 prefix")
}

fn parse_vault_claim_events(logs: &[Value]) -> Vec<VaultEvent> {
    let claim = event_discriminator("VaultClaim");
    let dev = event_discriminator("VaultDevFee");
    let mut out = Vec::new();
    for log in logs {
        let Some(line) = log.as_str() else { continue };
        let Some(payload) = line.strip_prefix("Program data: ") else {
            continue;
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(payload.trim()) else {
            continue;
        };
        if bytes.len() < 48 {
            continue;
        }
        let disc = &bytes[..8];
        let kind = if disc == claim {
            ChainActivityKind::VaultClaim
        } else if disc == dev {
            ChainActivityKind::VaultDevFee
        } else {
            continue;
        };
        let Ok(recipient_bytes) = <[u8; 32]>::try_from(&bytes[8..40]) else {
            continue;
        };
        let Ok(amount_bytes) = <[u8; 8]>::try_from(&bytes[40..48]) else {
            continue;
        };
        let amount = u64::from_le_bytes(amount_bytes) as i64;
        if amount <= 0 {
            continue;
        }
        out.push(VaultEvent {
            kind,
            recipient: Pubkey::new_from_array(recipient_bytes).to_string(),
            amount,
        });
    }
    out
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

    fn balance_tx(logs: &[&str], owner: &str, mint: &str, pre: i64, post: i64) -> Value {
        json!({
            "blockTime": 1,
            "meta": {
                "err": null,
                "logMessages": logs,
                "preTokenBalances": [{
                    "mint": mint,
                    "owner": owner,
                    "uiTokenAmount": { "amount": pre.to_string() }
                }],
                "postTokenBalances": [{
                    "mint": mint,
                    "owner": owner,
                    "uiTokenAmount": { "amount": post.to_string() }
                }],
                "innerInstructions": []
            },
            "transaction": { "message": { "instructions": [] } }
        })
    }

    #[test]
    fn rpc_envelope_surfaces_helius_unauthorized() {
        let raw = r#"{"jsonrpc":"2.0","error":{"code":-32401,"message":"Unauthorized"},"id":1}"#;
        let envelope: RpcEnvelope<Value> = serde_json::from_str(raw).unwrap();
        assert!(envelope.result.is_none());
        let err = envelope.error.expect("error body");
        assert_eq!(err.code, Some(-32401));
        assert_eq!(rpc_error_message(&err), "Unauthorized (-32401)");
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
        );
        let got = classify_tx(&tx, "sig", owner, &[], mint, vault);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, ChainActivityKind::VaultJoin);
        assert_eq!(got[0].amount_micro, 5_000_000);
    }

    fn program_data_log(name: &str, recipient: Pubkey, amount: u64) -> String {
        let mut bytes = Vec::with_capacity(48);
        bytes.extend_from_slice(&event_discriminator(name));
        bytes.extend_from_slice(recipient.as_ref());
        bytes.extend_from_slice(&amount.to_le_bytes());
        format!(
            "Program data: {}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }

    #[test]
    fn claim_events_label_winner_and_dev() {
        let winner = Pubkey::new_from_array([1u8; 32]);
        let dev = Pubkey::new_from_array([3u8; 32]);
        let winner_s = winner.to_string();
        let dev_s = dev.to_string();
        let mint = "Mint11111111111111111111111111111111111111";
        let vault = "Vault1111111111111111111111111111111111111";
        let invoke = format!("Program {vault} invoke [1]");
        let success = format!("Program {vault} success");
        let logs = [
            invoke.as_str(),
            "Program log: Instruction: Claim",
            &program_data_log("VaultClaim", winner, 6_510_000),
            &program_data_log("VaultDevFee", dev, 350_000),
            success.as_str(),
        ];

        let winner_tx = balance_tx(&logs, &winner_s, mint, 0, 6_510_000);
        let got = classify_tx(&winner_tx, "sig", &winner_s, &[], mint, vault);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, ChainActivityKind::VaultClaim);
        assert_eq!(got[0].amount_micro, 6_510_000);

        let dev_tx = balance_tx(&logs, &dev_s, mint, 0, 350_000);
        let got = classify_tx(&dev_tx, "sig", &dev_s, &[], mint, vault);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, ChainActivityKind::VaultDevFee);
        assert_eq!(got[0].amount_micro, 350_000);
    }
}
