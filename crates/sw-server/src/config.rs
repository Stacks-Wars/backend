use std::net::IpAddr;

use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub host: IpAddr,
    pub port: u16,
    pub database_url: String,
    pub redis_url: String,
    pub internal_api_secret: String,
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

        let internal_api_secret = required_env("INTERNAL_API_SECRET")?;
        if internal_api_secret.len() < 16 {
            return Err(anyhow!(
                "INTERNAL_API_SECRET must be at least 16 characters"
            ));
        }

        Ok(Self {
            host,
            port,
            database_url,
            redis_url,
            internal_api_secret,
        })
    }
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
