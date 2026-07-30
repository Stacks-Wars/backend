//! Hiro / Stacks API helpers — balance, activity, call-read, tx status.

use serde::Deserialize;
use serde_json::{json, Value};
use sw_domain::{ChainActivityItem, ChainActivityKind};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct HiroClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    usdcx_contract: String,
    usdcx_asset: String,
    vault_contract: Option<String>,
}

impl HiroClient {
    pub fn new(
        base_url: String,
        api_key: String,
        usdcx_contract: &str,
        asset_name: &str,
        vault_contract: Option<String>,
    ) -> Self {
        let usdcx_asset = format!("{usdcx_contract}::{asset_name}");
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            usdcx_contract: usdcx_contract.to_owned(),
            usdcx_asset,
            vault_contract,
        }
    }

    fn apply_key(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.api_key.is_empty() {
            req
        } else {
            req.header("x-api-key", &self.api_key)
        }
    }

    /// USDCx fungible token balance for an address (micro-units).
    pub async fn get_ft_balance(&self, address: &str) -> AppResult<i64> {
        let url = format!(
            "{}/extended/v1/address/{address}/balances",
            self.base_url
        );
        let resp = self
            .apply_key(self.http.get(&url))
            .send()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        if !resp.status().is_success() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "hiro balances failed: {}",
                resp.status()
            )));
        }
        let body: AddressBalances = resp
            .json()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        let Some(ft) = body.fungible_tokens else {
            return Ok(0);
        };
        let Some(entry) = ft.get(&self.usdcx_asset) else {
            return Ok(0);
        };
        let bal: i64 = entry.balance.parse().unwrap_or(0);
        Ok(bal.max(0))
    }

    pub async fn get_tx_status(&self, txid: &str) -> AppResult<Option<String>> {
        let url = format!("{}/extended/v1/tx/{txid}", self.base_url);
        let resp = self
            .apply_key(self.http.get(&url))
            .send()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "hiro tx lookup failed: {}",
                resp.status()
            )));
        }
        let body: TxStatusBody = resp
            .json()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(body.tx_status)
    }

    pub async fn require_tx_success(&self, txid: &str) -> AppResult<()> {
        let status = self
            .get_tx_status(txid)
            .await?
            .ok_or_else(|| AppError::BadRequest("transaction not found".into()))?;
        if is_confirmed_status(&status) {
            return Ok(());
        }
        if is_failed_status(&status) {
            return Err(AppError::BadRequest(format!(
                "transaction failed: {status}"
            )));
        }
        Err(AppError::BadRequest(format!(
            "transaction not confirmed yet: {status}"
        )))
    }

    /// Call-read a Clarity function; returns decoded JSON from Hiro.
    pub async fn call_read(
        &self,
        contract_id: &str,
        function_name: &str,
        sender: &str,
        args_hex: &[String],
    ) -> AppResult<Value> {
        let (deployer, name) = split_contract_id(contract_id)?;
        let url = format!(
            "{}/v2/contracts/call-read/{deployer}/{name}/{function_name}",
            self.base_url
        );
        let body = json!({
            "sender": sender,
            "arguments": args_hex,
        });
        let resp = self
            .apply_key(self.http.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(anyhow::anyhow!(
                "hiro call-read failed: {text}"
            )));
        }
        resp.json()
            .await
            .map_err(|e| AppError::Internal(e.into()))
    }

    /// Activity for a custodial address (FT transfers + vault contract calls).
    pub async fn list_activity(
        &self,
        address: &str,
        limit: u32,
    ) -> AppResult<Vec<ChainActivityItem>> {
        let url = format!(
            "{}/extended/v1/address/{address}/transactions_with_transfers?limit={limit}&unanchored=false",
            self.base_url
        );
        let resp = self
            .apply_key(self.http.get(&url))
            .send()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        if !resp.status().is_success() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "hiro address txs failed: {}",
                resp.status()
            )));
        }
        let body: AddressTxsResponse = resp
            .json()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;

        let vault = self.vault_contract.as_deref();
        let mut out = Vec::new();
        for item in body.results.unwrap_or_default() {
            let Some(tx) = item.tx else { continue };
            let txid = tx.tx_id.unwrap_or_default();
            if txid.is_empty() {
                continue;
            }
            let status = tx.tx_status.unwrap_or_else(|| "unknown".into());
            let block_time = tx.burn_block_time.or(tx.block_time);

            // Prefer classifying vault contract calls.
            if let Some(vault_id) = vault {
                if let Some(cc) = tx.contract_call.as_ref() {
                    let cid = format!("{}.{}", cc.contract_id.as_deref().unwrap_or(""), "");
                    let full = cc.contract_id.clone().unwrap_or_default();
                    if full == vault_id || cid.trim_end_matches('.') == vault_id {
                        let fn_name = cc.function_name.as_deref().unwrap_or("");
                        let path = extract_lobby_path_arg(cc.function_args.as_ref());
                        let amount = ft_amount_involving(
                            &item.ft_transfers,
                            address,
                            &self.usdcx_asset,
                        );
                        let kind = match fn_name {
                            "join" => ChainActivityKind::VaultJoin,
                            "leave" => ChainActivityKind::VaultLeave,
                            "kick" => ChainActivityKind::VaultKick,
                            "claim" => ChainActivityKind::VaultClaim,
                            _ => ChainActivityKind::Other,
                        };
                        out.push(ChainActivityItem {
                            txid: txid.clone(),
                            kind,
                            amount_micro: amount,
                            from_address: Some(address.to_owned()),
                            to_address: Some(vault_id.to_owned()),
                            lobby_path: path,
                            status: status.clone(),
                            block_time,
                        });
                        continue;
                    }
                }
            }

            for ft in item.ft_transfers.unwrap_or_default() {
                if ft.asset_identifier.as_deref() != Some(self.usdcx_asset.as_str()) {
                    continue;
                }
                let amount_micro: i64 = ft
                    .amount
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if amount_micro <= 0 {
                    continue;
                }
                let sender = ft.sender.unwrap_or_default();
                let recipient = ft.recipient.unwrap_or_default();
                let kind = if recipient == address {
                    // Ignore vault→user claim/leave refunds already classified? If we skipped vault calls above, inbound from vault might still show as deposit — check vault.
                    if vault.is_some_and(|v| sender.starts_with(v.split('.').next().unwrap_or("")))
                        || vault.is_some_and(|v| sender == *v)
                    {
                        continue;
                    }
                    ChainActivityKind::Deposit
                } else if sender == address {
                    if vault.is_some_and(|v| recipient == *v) {
                        continue;
                    }
                    ChainActivityKind::Withdraw
                } else {
                    continue;
                };
                out.push(ChainActivityItem {
                    txid: txid.clone(),
                    kind,
                    amount_micro,
                    from_address: Some(sender),
                    to_address: Some(recipient),
                    lobby_path: None,
                    status: status.clone(),
                    block_time,
                });
            }
        }
        Ok(out)
    }

    pub fn usdcx_contract(&self) -> &str {
        &self.usdcx_contract
    }

    pub fn vault_contract(&self) -> Option<&str> {
        self.vault_contract.as_deref()
    }
}

pub fn is_confirmed_status(status: &str) -> bool {
    status == "success"
}

pub fn is_failed_status(status: &str) -> bool {
    status.starts_with("abort_") || status == "failed" || status == "dropped_replace_by_fee"
}

fn split_contract_id(id: &str) -> AppResult<(&str, &str)> {
    let (a, b) = id
        .split_once('.')
        .ok_or_else(|| AppError::BadRequest("invalid contract id".into()))?;
    Ok((a, b))
}

fn ft_amount_involving(
    transfers: &Option<Vec<FtTransfer>>,
    address: &str,
    asset: &str,
) -> i64 {
    transfers
        .as_ref()
        .map(|v| {
            v.iter()
                .filter(|ft| ft.asset_identifier.as_deref() == Some(asset))
                .filter(|ft| {
                    ft.sender.as_deref() == Some(address)
                        || ft.recipient.as_deref() == Some(address)
                })
                .filter_map(|ft| ft.amount.as_deref()?.parse::<i64>().ok())
                .sum()
        })
        .unwrap_or(0)
}

/// Best-effort: lobby path from Clarity function args (Hiro `repr`).
fn extract_lobby_path_arg(args: Option<&Vec<FunctionArg>>) -> Option<String> {
    let args = args?;
    for a in args {
        if let Some(path) = path_from_repr(a.repr.as_deref()) {
            return Some(path);
        }
    }
    None
}

fn path_from_repr(repr: Option<&str>) -> Option<String> {
    let repr = repr?;
    let s = repr.strip_prefix('"').and_then(|r| r.strip_suffix('"'))?;
    if !s.is_empty() && s.len() <= 64 {
        Some(s.to_owned())
    } else {
        None
    }
}

#[derive(Debug, Deserialize)]
struct AddressBalances {
    fungible_tokens: Option<std::collections::HashMap<String, FtBalanceEntry>>,
}

#[derive(Debug, Deserialize)]
struct FtBalanceEntry {
    balance: String,
}

#[derive(Debug, Deserialize)]
struct AddressTxsResponse {
    results: Option<Vec<AddressTxItem>>,
}

#[derive(Debug, Deserialize)]
struct AddressTxItem {
    tx: Option<TxStub>,
    ft_transfers: Option<Vec<FtTransfer>>,
}

#[derive(Debug, Deserialize)]
struct TxStub {
    tx_id: Option<String>,
    tx_status: Option<String>,
    burn_block_time: Option<i64>,
    block_time: Option<i64>,
    contract_call: Option<ContractCallStub>,
}

#[derive(Debug, Deserialize)]
struct ContractCallStub {
    contract_id: Option<String>,
    function_name: Option<String>,
    function_args: Option<Vec<FunctionArg>>,
}

#[derive(Debug, Deserialize)]
struct FunctionArg {
    hex: Option<String>,
    repr: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FtTransfer {
    asset_identifier: Option<String>,
    amount: Option<String>,
    sender: Option<String>,
    recipient: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TxStatusBody {
    tx_status: Option<String>,
}
