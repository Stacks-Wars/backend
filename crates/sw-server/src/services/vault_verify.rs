//! Vault on-chain verification via Hiro tx lookup + call-read.

use crate::error::{AppError, AppResult};
use crate::services::hiro::HiroClient;

pub struct VaultReader<'a> {
    hiro: &'a HiroClient,
    vault_contract: &'a str,
}

impl<'a> VaultReader<'a> {
    pub fn new(hiro: &'a HiroClient, vault_contract: &'a str) -> Self {
        Self {
            hiro,
            vault_contract,
        }
    }

    pub async fn has_joined(&self, path: &str, player: &str) -> AppResult<bool> {
        let args = vec![
            encode_string_ascii(path)?,
            encode_principal(player)?,
        ];
        let res = self
            .hiro
            .call_read(self.vault_contract, "has-joined", player, &args)
            .await?;
        parse_bool_result(&res)
    }

    pub async fn get_paid(&self, path: &str, player: &str) -> AppResult<Option<i64>> {
        let args = vec![
            encode_string_ascii(path)?,
            encode_principal(player)?,
        ];
        let res = self
            .hiro
            .call_read(self.vault_contract, "get-paid", player, &args)
            .await?;
        parse_optional_uint(&res)
    }

    pub async fn get_pot(&self, path: &str, sender: &str) -> AppResult<i64> {
        let args = vec![encode_string_ascii(path)?];
        let res = self
            .hiro
            .call_read(self.vault_contract, "get-pot", sender, &args)
            .await?;
        Ok(parse_uint_result(&res)? as i64)
    }

    /// Confirm tx succeeded and player is joined with expected paid amount.
    pub async fn assert_joined(
        &self,
        path: &str,
        player: &str,
        expected_paid: i64,
        vault_txid: &str,
    ) -> AppResult<()> {
        self.hiro.require_tx_success(vault_txid).await?;
        if !self.has_joined(path, player).await? {
            return Err(AppError::BadRequest(
                "vault join not confirmed on-chain".into(),
            ));
        }
        let paid = self.get_paid(path, player).await?.unwrap_or(-1);
        if paid != expected_paid {
            return Err(AppError::BadRequest(format!(
                "vault paid amount mismatch: on-chain {paid}, expected {expected_paid}"
            )));
        }
        Ok(())
    }

    pub async fn assert_not_joined(
        &self,
        path: &str,
        player: &str,
        vault_txid: &str,
    ) -> AppResult<()> {
        self.hiro.require_tx_success(vault_txid).await?;
        if self.has_joined(path, player).await? {
            return Err(AppError::BadRequest(
                "vault leave/kick not confirmed on-chain".into(),
            ));
        }
        Ok(())
    }

    pub async fn assert_claim_tx(&self, vault_txid: &str) -> AppResult<()> {
        self.hiro.require_tx_success(vault_txid).await
    }
}

fn encode_string_ascii(s: &str) -> AppResult<String> {
    if !s.is_ascii() || s.len() > 64 {
        return Err(AppError::BadRequest("invalid string-ascii".into()));
    }
    let mut bytes = Vec::with_capacity(2 + s.len());
    bytes.push(0x0d); // ClarityType::StringASCII
    bytes.push(s.len() as u8);
    bytes.extend_from_slice(s.as_bytes());
    Ok(format!("0x{}", hex_encode(&bytes)))
}

fn encode_principal(address: &str) -> AppResult<String> {
    let (version, hash160) = decode_c32_address(address)?;
    let mut bytes = Vec::with_capacity(22);
    bytes.push(0x05); // Standard principal
    bytes.push(version);
    bytes.extend_from_slice(&hash160);
    Ok(format!("0x{}", hex_encode(&bytes)))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_bool_result(result: &serde_json::Value) -> AppResult<bool> {
    if result.get("okay").and_then(|v| v.as_bool()) == Some(false) {
        return Err(AppError::BadRequest("vault call-read failed".into()));
    }
    let repr = result
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if repr.contains("true") {
        return Ok(true);
    }
    if repr.contains("false") {
        return Ok(false);
    }
    Err(AppError::Internal(anyhow::anyhow!(
        "unexpected call-read bool: {result}"
    )))
}

fn parse_uint_result(result: &serde_json::Value) -> AppResult<u128> {
    if result.get("okay").and_then(|v| v.as_bool()) == Some(false) {
        return Err(AppError::BadRequest("vault call-read failed".into()));
    }
    let repr = result
        .get("result")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("missing call-read result")))?;
    let digits: String = repr.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        if repr.contains("none") {
            return Ok(0);
        }
        return Err(AppError::Internal(anyhow::anyhow!(
            "unexpected call-read uint: {repr}"
        )));
    }
    digits
        .parse()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("parse uint: {e}")))
}

fn parse_optional_uint(result: &serde_json::Value) -> AppResult<Option<i64>> {
    let repr = result
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if repr.contains("none") {
        return Ok(None);
    }
    Ok(Some(parse_uint_result(result)? as i64))
}

/// Decode a Stacks C32 address into (version, hash160).
fn decode_c32_address(address: &str) -> AppResult<(u8, [u8; 20])> {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    if address.len() < 5 || !(address.starts_with('S') || address.starts_with('T')) {
        return Err(AppError::BadRequest("invalid stacks address".into()));
    }
    // Skip leading S/T network char used in display — c32check payload is after first char
    let body = &address[1..];
    let mut values = Vec::with_capacity(body.len());
    for ch in body.chars() {
        let c = ch.to_ascii_uppercase() as u8;
        let idx = ALPHABET
            .iter()
            .position(|&a| a == c)
            .ok_or_else(|| AppError::BadRequest("invalid c32 character".into()))?;
        values.push(idx as u8);
    }
    // Convert base32 to bytes
    let mut acc: u128 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::new();
    for v in values {
        acc = (acc << 5) | u128::from(v);
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if out.len() < 21 {
        return Err(AppError::BadRequest("address too short".into()));
    }
    // version (1) + hash160 (20) + checksum (4) — c32check includes checksum in decoded form
    // Stacks c32 addresses decode to version + 20-byte hash + 4-byte checksum
    if out.len() < 25 {
        // Some decoders yield 21 without verifying checksum separately
        let version = out[0];
        let mut hash = [0u8; 20];
        if out.len() >= 21 {
            hash.copy_from_slice(&out[1..21]);
            return Ok((version, hash));
        }
        return Err(AppError::BadRequest("bad address length".into()));
    }
    let version = out[0];
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&out[1..21]);
    Ok((version, hash))
}
