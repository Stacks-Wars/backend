use std::net::IpAddr;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};

use crate::services::neon_jwt::{NeonJwtConfig, NeonJwtVerifier};

/// Withdraw floor ($1 USDCx).
pub const MIN_WITHDRAW_MICRO: i64 = 1_000_000;
/// Withdraw ceiling ($10k USDCx).
pub const MAX_WITHDRAW_MICRO: i64 = 10_000_000_000;
/// Paid lobby entry floor ($1). Free (`0`) still allowed.
pub const MIN_ENTRY_MICRO: i64 = 1_000_000;
/// Redis TTL for UI balance reads (validation always busts/refreshes).
pub const BALANCE_CACHE_SECS: u64 = 300;
/// Mainnet USDCx contract id.
pub const USDCX_CONTRACT: &str = "SP120SBRBQJ00MCWS7TM5R8WJNTTKD5K0HFRC2CNE.usdcx";
/// SIP-010 FT name inside the USDCx contract.
pub const USDCX_ASSET_NAME: &str = "usdcx-token";

#[derive(Debug, Clone)]
pub struct Config {
    pub host: IpAddr,
    pub port: u16,
    pub database_url: String,
    pub redis_url: String,
    pub hiro_api_url: String,
    pub hiro_api_key: String,
    pub stacks_network: String,
    pub sw_vault_contract: String,
    pub neon_auth_base_url: String,
    pub jwt: NeonJwtConfig,
    pub admin_emails: Vec<String>,
    pub internal_api_secret: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let host = std::env::var("HOST")
            .unwrap_or_else(|_| "0.0.0.0".to_owned())
            .parse::<IpAddr>()
            .context("parse HOST")?;

        let port = std::env::var("PORT")
            .unwrap_or_else(|_| "8080".to_owned())
            .parse::<u16>()
            .context("parse PORT")?;

        let database_url = required_env("DATABASE_URL")?;
        let redis_url = required_env("REDIS_URL")?;

        let hiro_api_url = std::env::var("HIRO_API_URL")
            .unwrap_or_else(|_| "https://api.hiro.so".to_owned());
        let hiro_api_key = required_env("HIRO_API_KEY")?;
        let stacks_network =
            std::env::var("STACKS_NETWORK").unwrap_or_else(|_| "mainnet".to_owned());
        let sw_vault_contract = required_env("SW_VAULT_CONTRACT")?;
        if !sw_vault_contract.contains('.') {
            return Err(anyhow!(
                "SW_VAULT_CONTRACT must be deployer.contract-name"
            ));
        }

        let neon_auth_base_url = required_env("NEON_AUTH_BASE_URL")?;
        let jwt = NeonJwtConfig::from_auth_base_url(&neon_auth_base_url)
            .map_err(|e| anyhow!(e.to_string()))?;

        let admin_emails = parse_admin_emails(std::env::var("ADMIN").ok().as_deref());
        let internal_api_secret = std::env::var("INTERNAL_API_SECRET")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());

        Ok(Self {
            host,
            port,
            database_url,
            redis_url,
            hiro_api_url,
            hiro_api_key,
            stacks_network,
            sw_vault_contract,
            neon_auth_base_url,
            jwt,
            admin_emails,
            internal_api_secret,
        })
    }

    pub fn jwt_verifier(&self) -> Arc<NeonJwtVerifier> {
        NeonJwtVerifier::arc(self.jwt.clone())
    }

    /// Vault deployer principal — matches Clarity `PLATFORM-WALLET`.
    pub fn platform_wallet(&self) -> &str {
        self.sw_vault_contract
            .split_once('.')
            .map(|(a, _)| a)
            .unwrap_or(self.sw_vault_contract.as_str())
    }
}

fn parse_admin_emails(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let mut emails = Vec::new();
    for part in raw.split(',') {
        let e = part.trim().to_lowercase();
        if e.is_empty() || !e.contains('@') {
            continue;
        }
        if !emails.iter().any(|x| x == &e) {
            emails.push(e);
        }
    }
    emails
}

fn required_env(key: &str) -> Result<String> {
    let value = std::env::var(key)
        .with_context(|| format!("{key} must be set"))?
        .trim()
        .to_owned();
    if value.is_empty() {
        return Err(anyhow!("{key} must not be empty"));
    }
    Ok(value)
}
