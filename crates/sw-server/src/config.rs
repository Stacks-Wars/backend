use std::net::IpAddr;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use sw_domain::ChainId;

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
    pub internal_api_secret: String,
    /// Public web app origin for deep links (`https://stackswars.com`).
    pub frontend_url: String,
    /// Telegram bot token. Empty → Telegram disabled.
    pub telegram_bot_token: Option<String>,
    /// Target chat/channel id for lobby broadcasts. Required when bot token is set.
    pub telegram_chat_id: Option<i64>,
    pub vapid_public_key: Option<String>,
    pub vapid_private_key: Option<String>,
    pub vapid_subject: String,
    pub solana_rpc_url: String,
    pub solana_usdc_mint: String,
    pub solana_vault_program_id: String,
    /// Wars key pubkey. Used as Solana game-fee fallback when the plugin dev
    /// has no Solana custodial wallet. Empty → Stacks principal (frontend remaps).
    pub solana_platform_wallet: String,
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

        let hiro_api_url =
            std::env::var("HIRO_API_URL").unwrap_or_else(|_| "https://api.hiro.so".to_owned());
        let hiro_api_key = required_env("HIRO_API_KEY")?;
        let stacks_network =
            std::env::var("STACKS_NETWORK").unwrap_or_else(|_| "mainnet".to_owned());
        let sw_vault_contract = required_env("SW_VAULT_CONTRACT")?;
        if !sw_vault_contract.contains('.') {
            return Err(anyhow!("SW_VAULT_CONTRACT must be deployer.contract-name"));
        }

        let neon_auth_base_url = required_env("NEON_AUTH_BASE_URL")?;
        let jwt = NeonJwtConfig::from_auth_base_url(&neon_auth_base_url)
            .map_err(|e| anyhow!(e.to_string()))?;

        let admin_emails = parse_admin_emails(std::env::var("ADMIN").ok().as_deref());
        let internal_api_secret = required_env("INTERNAL_API_SECRET")?;

        let frontend_url = std::env::var("FRONTEND_URL")
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "https://stackswars.com".to_owned());

        let telegram_bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
            .ok()
            .map(|s| s.trim().trim_matches('"').to_owned())
            .filter(|s| !s.is_empty());
        let telegram_chat_id = match std::env::var("TELEGRAM_CHAT_ID")
            .ok()
            .map(|s| s.trim().trim_matches('"').to_owned())
            .filter(|s| !s.is_empty())
        {
            Some(raw) => Some(
                raw.parse::<i64>()
                    .context("parse TELEGRAM_CHAT_ID as i64")?,
            ),
            None => None,
        };
        if telegram_bot_token.is_some() ^ telegram_chat_id.is_some() {
            return Err(anyhow!(
                "TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID must both be set (or both unset)"
            ));
        }

        let vapid_public_key = std::env::var("VAPID_PUBLIC_KEY")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let vapid_private_key = std::env::var("VAPID_PRIVATE_KEY")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let vapid_subject = std::env::var("VAPID_SUBJECT")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "mailto:contact@mail.stackswars.com".to_owned());
        let vapid_subject =
            if vapid_subject.starts_with("mailto:") || vapid_subject.starts_with("https://") {
                vapid_subject
            } else if vapid_subject.contains('@') {
                format!("mailto:{vapid_subject}")
            } else {
                vapid_subject
            };

        let solana_rpc_url = std::env::var("SOLANA_RPC_URL")
            .unwrap_or_else(|_| "https://api.devnet.solana.com".to_owned());
        let solana_usdc_mint = std::env::var("SOLANA_USDC_MINT").unwrap_or_else(|_| {
            // Platform test USDC on devnet. Override for mainnet Circle.
            "2ztYALhLWs2Lg1bGRBje82RgiLhuH4ZbCimRWVeyxUaB".to_owned()
        });
        let solana_vault_program_id = std::env::var("SOLANA_VAULT_PROGRAM_ID")
            .unwrap_or_else(|_| "8NZHj9VH9JkqiAg19CK43ZLuK5hn5jXPBnLfbeKonqfy".to_owned());
        let solana_platform_wallet = std::env::var("SOLANA_PLATFORM_WALLET")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_default();

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
            frontend_url,
            telegram_bot_token,
            telegram_chat_id,
            vapid_public_key,
            vapid_private_key,
            vapid_subject,
            solana_rpc_url,
            solana_usdc_mint,
            solana_vault_program_id,
            solana_platform_wallet,
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

    /// Game-fee fallback when the plugin `dev_id` has no wallet on this chain.
    pub fn fallback_dev_wallet(&self, chain: ChainId) -> String {
        match chain {
            ChainId::Solana if !self.solana_platform_wallet.is_empty() => {
                self.solana_platform_wallet.clone()
            }
            _ => self.platform_wallet().to_owned(),
        }
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
