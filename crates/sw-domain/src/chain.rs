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
    Arbitrum,
    Botchain,
}

impl ChainId {
    pub const ALL: [Self; 4] = [Self::Stacks, Self::Solana, Self::Arbitrum, Self::Botchain];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stacks => "stacks",
            Self::Solana => "solana",
            Self::Arbitrum => "arbitrum",
            Self::Botchain => "botchain",
        }
    }

    pub const fn play_token_symbol(self) -> &'static str {
        match self {
            Self::Stacks => "USDCx",
            Self::Solana => "USDC",
            Self::Arbitrum => "USDC",
            Self::Botchain => "USDT",
        }
    }

    /// Missing / junk query values fall back to the product default (Solana).
    pub fn from_optional(s: Option<&str>) -> Self {
        s.map(str::trim)
            .filter(|v| !v.is_empty())
            .and_then(|v| v.parse().ok())
            .unwrap_or_default()
    }

    /// `0x` EOAs first so they are not mistaken for Solana pubkeys (same length).
    pub fn infer_from_address(address: &str) -> Option<Self> {
        let a = address.trim();
        if looks_like_evm_address(a) {
            return Some(Self::Arbitrum);
        }
        if looks_like_stacks_address(a) {
            return Some(Self::Stacks);
        }
        if (32..=44).contains(&a.len()) {
            return Some(Self::Solana);
        }
        None
    }

    pub fn matches_address(self, address: &str) -> bool {
        match self {
            Self::Arbitrum | Self::Botchain => looks_like_evm_address(address),
            Self::Stacks | Self::Solana => Self::infer_from_address(address) == Some(self),
        }
    }
}

fn looks_like_evm_address(address: &str) -> bool {
    let rest = address.strip_prefix("0x").or_else(|| address.strip_prefix("0X"));
    let Some(rest) = rest else {
        return false;
    };
    rest.len() == 40 && rest.bytes().all(|b| b.is_ascii_hexdigit())
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
            "arbitrum" | "arb" => Ok(Self::Arbitrum),
            "botchain" | "bot" => Ok(Self::Botchain),
            other => Err(format!("unknown chain: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evm_address_is_arbitrum_before_solana_length() {
        let eoa = "0x2092cD008184Dd0E01C90Aa14b132B468a69745E";
        assert_eq!(ChainId::infer_from_address(eoa), Some(ChainId::Arbitrum));
        assert!(ChainId::Arbitrum.matches_address(eoa));
        assert!(ChainId::Botchain.matches_address(eoa));
        assert!(!ChainId::Solana.matches_address(eoa));
    }

    #[test]
    fn parses_arb_alias() {
        assert_eq!("arb".parse::<ChainId>().unwrap(), ChainId::Arbitrum);
        assert_eq!(ChainId::Arbitrum.play_token_symbol(), "USDC");
        assert_eq!("bot".parse::<ChainId>().unwrap(), ChainId::Botchain);
        assert_eq!(ChainId::Botchain.play_token_symbol(), "USDT");
        assert_eq!(ChainId::ALL.len(), 4);
    }
}
