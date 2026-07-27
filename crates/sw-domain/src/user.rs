use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::UserId;

/// Platform user synced from Neon Auth.
///
/// `wallet_address` is the user's personal Stacks address for receiving rewards
/// (linked later). Server automation uses a separate `custodial_wallets` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub email: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub wallet_address: Option<String>,
    pub wallet_verified_at: Option<DateTime<Utc>>,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
