use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::UserId;

/// Platform user synced from Neon Auth.
///
/// On-chain keys live in `custodial_wallets`, one row per chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: UserId,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub email: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
