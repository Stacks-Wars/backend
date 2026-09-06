use std::net::IpAddr;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use sw_domain::ChainId;

use crate::services::jwt::{JwtConfig, JwtVerifier};

pub const MIN_WITHDRAW_MICRO: i64 = 1_000_000;
pub const MAX_WITHDRAW_MICRO: i64 = 10_000_000_000;
pub const MIN_ENTRY_MICRO: i64 = 1_000_000;
pub const BALANCE_CACHE_SECS: u64 = 300;
pub const USDCX_ASSET_NAME: &str = "usdcx-token";
pub const VAPID_SUBJECT: &str = "mailto:contact@mail.stackswars.com";

pub const USDCX_CONTRACT: &str = "SP120SBRBQJ00MCWS7TM5R8WJNTTKD5K0HFRC2CNE.usdcx";
pub const DEV_USDCX_CONTRACT: &str = "ST1S3D9BTK41ST9GRT225BQFFSYT6VMX7G7MNZ5FB.usdcx-dev";
pub const DEV_VAULT_CONTRACT: &str = "ST1S3D9BTK41ST9GRT225BQFFSYT6VMX7G7MNZ5FB.sw-vault-v1";
pub const MAIN_VAULT_CONTRACT: &str =
    "SP299MBHT7FPPP2SKEY73V4DHW67467SED87A4HH4.sw-vault-v0-0-1";

const LOCAL_INTERNAL_API_SECRET: &str = "sw-dev-internal";
const LOCAL_DATABASE_URL: &str = "postgres://postgres:postgres@127.0.0.1:5433/stacks_wars";
const LOCAL_REDIS_URL: &str = "redis://127.0.0.1:6380";
const LOCAL_APP_URL: &str = "http://localhost:3000";
const DEV_HIRO_API_URL: &str = "https://api.testnet.hiro.so";
const MAIN_HIRO_API_URL: &str = "https://api.hiro.so";
const DEV_SOLANA_RPC_URL: &str = "https://api.devnet.solana.com";
const MAIN_SOLANA_USDC_MINT: &str = "2ztYALhLWs2Lg1bGRBje82RgiLhuH4ZbCimRWVeyxUaB";
const MAIN_SOLANA_VAULT_PROGRAM_ID: &str = "8NZHj9VH9JkqiAg19CK43ZLuK5hn5jXPBnLfbeKonqfy";
const MAIN_SOLANA_PLATFORM_WALLET: &str = "931LzmTuFs3k8k73mnKaZoUUYbVZu6ZNAADGVTaupiAN";

#[derive(Debug, Clone)]
pub struct Config {
    pub is_dev: bool,
    pub host: IpAddr,
    pub port: u16,
    pub database_url: String,
    pub redis_url: String,
    pub hiro_api_url: String,
    pub hiro_api_key: String,
    pub sw_vault_contract: String,
    pub usdcx_contract: String,
    pub app_url: String,
    pub jwt: JwtConfig,
    pub admin_emails: Vec<String>,
    pub internal_api_secret: String,
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<i64>,
    pub vapid_public_key: Option<String>,
    pub vapid_private_key: Option<String>,
    pub solana_rpc_url: String,
    pub solana_usdc_mint: String,
    pub solana_vault_program_id: String,
    pub solana_platform_wallet: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let is_dev = !is_main();

        let host = optional("HOST")
            .unwrap_or_else(|| "0.0.0.0".to_owned())
            .parse::<IpAddr>()
            .context("parse HOST")?;
        let port = optional("PORT")
            .unwrap_or_else(|| "8080".to_owned())
            .parse::<u16>()
            .context("parse PORT")?;

        let database_url = setting("DATABASE_URL", LOCAL_DATABASE_URL)?;
        let redis_url = setting("REDIS_URL", LOCAL_REDIS_URL)?;
        let hiro_api_url = if is_dev {
            DEV_HIRO_API_URL.to_owned()
        } else {
            MAIN_HIRO_API_URL.to_owned()
        };
        let hiro_api_key = setting("HIRO_API_KEY", "")?;
        let sw_vault_contract = if is_dev {
            DEV_VAULT_CONTRACT.to_owned()
        } else {
            MAIN_VAULT_CONTRACT.to_owned()
        };
        let usdcx_contract = if is_dev {
            DEV_USDCX_CONTRACT.to_owned()
        } else {
            USDCX_CONTRACT.to_owned()
        };

        let app_url = setting("APP_URL", LOCAL_APP_URL)?
            .trim_end_matches('/')
            .to_owned();
        let jwt = JwtConfig::from_app_url(&app_url).map_err(|e| anyhow!(e.to_string()))?;

        let telegram_bot_token = optional("TELEGRAM_BOT_TOKEN");
        let telegram_chat_id = match optional("TELEGRAM_CHAT_ID") {
            Some(raw) => Some(raw.parse::<i64>().context("parse TELEGRAM_CHAT_ID as i64")?),
            None => None,
        };
        if telegram_bot_token.is_some() ^ telegram_chat_id.is_some() {
            return Err(anyhow!(
                "TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID must both be set (or both unset)"
            ));
        }

        let helius_key = setting("HELIUS_API_KEY", "")?;
        let solana_rpc_url = solana_rpc_url(&helius_key, is_dev)?;

        Ok(Self {
            is_dev,
            host,
            port,
            database_url,
            redis_url,
            hiro_api_url,
            hiro_api_key,
            sw_vault_contract,
            usdcx_contract,
            app_url,
            jwt,
            admin_emails: parse_admin_emails(optional("ADMIN").as_deref()),
            internal_api_secret: if is_dev {
                LOCAL_INTERNAL_API_SECRET.to_owned()
            } else {
                setting("INTERNAL_API_SECRET", LOCAL_INTERNAL_API_SECRET)?
            },
            telegram_bot_token,
            telegram_chat_id,
            vapid_public_key: optional("VAPID_PUBLIC_KEY"),
            vapid_private_key: optional("VAPID_PRIVATE_KEY"),
            solana_rpc_url,
            solana_usdc_mint: MAIN_SOLANA_USDC_MINT.to_owned(),
            solana_vault_program_id: MAIN_SOLANA_VAULT_PROGRAM_ID.to_owned(),
            solana_platform_wallet: MAIN_SOLANA_PLATFORM_WALLET.to_owned(),
        })
    }

    pub fn jwt_verifier(&self) -> Arc<JwtVerifier> {
        JwtVerifier::arc(self.jwt.clone())
    }

    pub fn stacks_network(&self) -> &'static str {
        if self.is_dev {
            "testnet"
        } else {
            "mainnet"
        }
    }

    pub fn platform_wallet(&self) -> &str {
        self.sw_vault_contract
            .split_once('.')
            .map(|(a, _)| a)
            .unwrap_or(self.sw_vault_contract.as_str())
    }

    pub fn fallback_dev_wallet(&self, chain: ChainId) -> String {
        match chain {
            ChainId::Solana if !self.solana_platform_wallet.is_empty() => {
                self.solana_platform_wallet.clone()
            }
            ChainId::Solana | ChainId::Stacks => self.platform_wallet().to_owned(),
        }
    }
}

fn is_main() -> bool {
    std::env::args().any(|a| a == "--main")
        || std::env::var("NETWORK")
            .ok()
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("main"))
}

fn solana_rpc_url(key: &str, is_dev: bool) -> Result<String> {
    let key = key.trim();
    if !key.is_empty() {
        // Solana stays on Devnet in local and production.
        return Ok(format!("https://devnet.helius-rpc.com/?api-key={key}"));
    }
    if is_dev {
        return Ok(DEV_SOLANA_RPC_URL.to_owned());
    }
    Err(anyhow!("HELIUS_API_KEY must be set"))
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

fn optional(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().trim_matches('"').to_owned())
        .filter(|s| !s.is_empty())
}

fn setting(key: &str, local: &str) -> Result<String> {
    if let Some(value) = optional(key) {
        return Ok(value);
    }
    if !is_main() {
        return Ok(local.to_owned());
    }
    Err(anyhow!("{key} must be set"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helius_on_main() {
        let url = solana_rpc_url("abc", false).unwrap();
        assert_eq!(url, "https://devnet.helius-rpc.com/?api-key=abc");
    }

    #[test]
    fn helius_in_dev() {
        let url = solana_rpc_url("abc", true).unwrap();
        assert_eq!(url, "https://devnet.helius-rpc.com/?api-key=abc");
    }

    #[test]
    fn missing_key_fails_on_main() {
        let err = solana_rpc_url("  ", false).unwrap_err();
        assert!(err.to_string().contains("HELIUS_API_KEY"));
    }

    #[test]
    fn missing_key_uses_public_rpc_in_dev() {
        let url = solana_rpc_url("", true).unwrap();
        assert_eq!(url, DEV_SOLANA_RPC_URL);
    }
}
