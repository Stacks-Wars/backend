//! Chain identity. Vault, RPC, and address format stay in per-chain adapters.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Supported settlement chains. Stored as lowercase text in Postgres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChainId {
    Stacks,
    Solana,
}

impl ChainId {
    pub const ALL: [Self; 2] = [Self::Stacks, Self::Solana];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stacks => "stacks",
            Self::Solana => "solana",
        }
    }

    pub const fn play_token_symbol(self) -> &'static str {
        match self {
            Self::Stacks => "USDCx",
            Self::Solana => "USDC",
        }
    }

    /// Missing / junk query values fall back to the product default (Solana).
    pub fn from_optional(s: Option<&str>) -> Self {
        s.map(str::trim)
            .filter(|v| !v.is_empty())
            .and_then(|v| v.parse().ok())
            .unwrap_or_default()
    }

    /// Stacks principals are c32 (`SP`/`ST`, no lowercase). Solana pubkeys mix case.
    pub fn infer_from_address(address: &str) -> Option<Self> {
        let a = address.trim();
        if looks_like_stacks_address(a) {
            return Some(Self::Stacks);
        }
        if (32..=44).contains(&a.len()) {
            return Some(Self::Solana);
        }
        None
    }

    pub fn matches_address(self, address: &str) -> bool {
        Self::infer_from_address(address) == Some(self)
    }
}

fn looks_like_stacks_address(address: &str) -> bool {
    let prefix_ok = address.starts_with("SP") || address.starts_with("ST");
    prefix_ok
        && address.len() >= 39
        && address
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

impl Default for ChainId {
    fn default() -> Self {
        Self::Solana
    }
}

impl fmt::Display for ChainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ChainId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "stacks" | "stx" => Ok(Self::Stacks),
            "solana" | "sol" => Ok(Self::Solana),
            other => Err(format!("unknown chain: {other}")),
        }
    }
}
