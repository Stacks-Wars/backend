use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::UserId;

/// Platform user. Wallet-linked identity is expected later; shell keeps it generic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub display_name: String,
    /// Stacks principal (e.g. `SP…`) when linked; optional in the shell.
    pub stacks_address: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Auth session claims shape used by API / WS boundaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthClaims {
    pub sub: UserId,
    pub exp: i64,
    pub iat: i64,
}
