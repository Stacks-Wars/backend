//! Confirm Solana vault signatures via JSON-RPC. Hiro stays on the Stacks path.

use serde::Deserialize;
use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct RpcEnvelope<T> {
    result: Option<T>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct TxResult {
    meta: Option<TxMeta>,
}

#[derive(Debug, Deserialize)]
struct TxMeta {
    err: Option<serde_json::Value>,
}

pub async fn assert_tx_ok(state: &AppState, signature: &str) -> AppResult<()> {
    if signature.trim().is_empty() {
        return Err(AppError::BadRequest("vault txid required".into()));
    }
    let client = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": [
            signature,
            {
                "encoding": "json",
                "commitment": "confirmed",
                "maxSupportedTransactionVersion": 1
            }
        ]
    });
    let envelope: RpcEnvelope<TxResult> = client
        .post(&state.config.solana_rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("solana rpc: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::BadRequest(format!("solana rpc decode: {e}")))?;

    if let Some(err) = envelope.error {
        return Err(AppError::BadRequest(format!("solana rpc error: {err}")));
    }
    let Some(result) = envelope.result else {
        return Err(AppError::BadRequest(
            "solana vault transaction not found".into(),
        ));
    };
    if result.meta.and_then(|m| m.err).is_some() {
        return Err(AppError::BadRequest(
            "solana vault transaction failed on-chain".into(),
        ));
    }
    Ok(())
}
