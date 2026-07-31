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
    // SIP-005: 0x0d || u32_be(len) || ascii_bytes
    let mut bytes = Vec::with_capacity(5 + s.len());
    bytes.push(0x0d); // ClarityType::StringASCII
    bytes.extend_from_slice(&(s.len() as u32).to_be_bytes());
    bytes.extend_from_slice(s.as_bytes());
    Ok(format!("0x{}", hex_encode(&bytes)))
}

fn encode_principal(address: &str) -> AppResult<String> {
    // Clarity standard principal: 0x05 || version || hash160
    Ok(serialize_principal(address)?)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(hex: &str) -> AppResult<Vec<u8>> {
    let hex = hex.trim().strip_prefix("0x").unwrap_or(hex.trim());
    if hex.len() % 2 != 0 {
        return Err(AppError::Internal(anyhow::anyhow!(
            "odd-length clarity hex: {hex}"
        )));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| {
                AppError::Internal(anyhow::anyhow!("invalid clarity hex byte: {e}"))
            })
        })
        .collect()
}

fn call_read_hex(result: &serde_json::Value) -> AppResult<&str> {
    if result.get("okay").and_then(|v| v.as_bool()) == Some(false) {
        return Err(AppError::BadRequest("vault call-read failed".into()));
    }
    result
        .get("result")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("missing call-read result")))
}

/// Hiro returns Clarity serializations as hex. BoolTrue=0x03, BoolFalse=0x04.
fn parse_bool_result(result: &serde_json::Value) -> AppResult<bool> {
    let repr = call_read_hex(result)?;
    let normalized = repr.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "0x03" | "03" | "true" | "(ok true)" => Ok(true),
        "0x04" | "04" | "false" | "(ok false)" => Ok(false),
        _ => Err(AppError::Internal(anyhow::anyhow!(
            "unexpected call-read bool: {result}"
        ))),
    }
}

/// Clarity uint: 0x01 || u128 big-endian (16 bytes).
fn parse_clarity_uint(bytes: &[u8]) -> AppResult<u128> {
    if bytes.len() != 17 || bytes[0] != 0x01 {
        return Err(AppError::Internal(anyhow::anyhow!(
            "unexpected clarity uint encoding: 0x{}",
            hex_encode(bytes)
        )));
    }
    let mut acc = 0u128;
    for b in &bytes[1..] {
        acc = (acc << 8) | u128::from(*b);
    }
    Ok(acc)
}

fn parse_uint_result(result: &serde_json::Value) -> AppResult<u128> {
    let repr = call_read_hex(result)?;
    let bytes = hex_decode(repr)?;
    parse_clarity_uint(&bytes)
}

/// Optional uint: none=0x09, some=0x0a || uint.
fn parse_optional_uint(result: &serde_json::Value) -> AppResult<Option<i64>> {
    if result.get("okay").and_then(|v| v.as_bool()) == Some(false) {
        return Err(AppError::BadRequest("vault call-read failed".into()));
    }
    let repr = result
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let normalized = repr.trim().to_ascii_lowercase();
    if normalized == "0x09" || normalized.contains("none") {
        return Ok(None);
    }
    let bytes = hex_decode(repr)?;
    if bytes.first() == Some(&0x09) {
        return Ok(None);
    }
    if bytes.first() == Some(&0x0a) {
        let value = parse_clarity_uint(&bytes[1..])?;
        return Ok(Some(value as i64));
    }
    // Bare uint (some read-onlys return uint directly).
    let value = parse_clarity_uint(&bytes)?;
    Ok(Some(value as i64))
}

/// C32 alphabet used by Stacks addresses (Crockford's Base32 variant).
const C32_CHARS: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Decode a c32-encoded string to bytes (base32 → base256 big-number conversion).
fn c32_decode(input: &str) -> AppResult<Vec<u8>> {
    let input = input.to_uppercase();
    let leading_zeros = input.chars().take_while(|&c| c == '0').count();

    let mut acc: Vec<u8> = Vec::new();
    for ch in input.chars() {
        let val = C32_CHARS
            .find(ch)
            .ok_or_else(|| AppError::BadRequest(format!("invalid c32 character: {ch}")))?;

        let mut carry = val;
        for byte in acc.iter_mut().rev() {
            let wide = (*byte as usize) * 32 + carry;
            *byte = (wide % 256) as u8;
            carry = wide / 256;
        }
        while carry > 0 {
            acc.insert(0, (carry % 256) as u8);
            carry /= 256;
        }
    }

    let mut result = vec![0u8; leading_zeros];
    result.extend(acc);
    Ok(result)
}

/// Serialize a Stacks address to Clarity principal hex.
///
/// Format: `S` + `<version_char>` + `<c32_encoded(hash160 + checksum)>`
/// Output: `0x05` + version_byte + hash160
fn serialize_principal(address: &str) -> AppResult<String> {
    let address = address.to_uppercase();
    if address.len() < 5 || !address.starts_with('S') {
        return Err(AppError::BadRequest(
            "invalid Stacks address format".into(),
        ));
    }

    let version_char = address.chars().nth(1).unwrap();
    let version = C32_CHARS
        .find(version_char)
        .ok_or_else(|| AppError::BadRequest("invalid address version character".into()))?
        as u8;

    let decoded = c32_decode(&address[2..])?;
    let hash160_with_checksum = if decoded.len() < 24 {
        let mut padded = vec![0u8; 24 - decoded.len()];
        padded.extend(&decoded);
        padded
    } else {
        decoded[decoded.len() - 24..].to_vec()
    };

    let mut principal_bytes = vec![0x05, version];
    principal_bytes.extend_from_slice(&hash160_with_checksum[..20]);
    Ok(format!("0x{}", hex_encode(&principal_bytes)))
}
