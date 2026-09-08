//! Confirm BOT Chain vault transactions via eth_getTransactionReceipt.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct RpcEnvelope<T> {
    result: Option<T>,
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct Receipt {
    status: Option<String>,
}

pub async fn assert_tx_ok(state: &AppState, txid: &str) -> AppResult<()> {
    let hash = normalize_tx_hash(txid)?;
    let client = reqwest::Client::new();
    let envelope: RpcEnvelope<Receipt> = client
        .post(&state.config.botchain_rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getTransactionReceipt",
            "params": [hash],
        }))
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("botchain rpc: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::BadRequest(format!("botchain rpc decode: {e}")))?;

    if let Some(err) = envelope.error {
        return Err(AppError::BadRequest(format!("botchain rpc error: {err}")));
    }
    let Some(receipt) = envelope.result else {
        return Err(AppError::BadRequest(
            "botchain vault transaction not found".into(),
        ));
    };
    let status = receipt.status.unwrap_or_default();
    if status != "0x1" && status != "1" {
        return Err(AppError::BadRequest(
            "botchain vault transaction failed on-chain".into(),
        ));
    }
    Ok(())
}

fn normalize_tx_hash(txid: &str) -> AppResult<String> {
    let trimmed = txid.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("vault txid required".into()));
    }
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest("invalid BOT Chain transaction hash".into()));
    }
    Ok(format!("0x{}", hex.to_ascii_lowercase()))
}
