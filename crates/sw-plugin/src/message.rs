use serde::{Deserialize, Serialize};
use sw_domain::UserId;

/// Outcome reported when an engine finishes a match.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchResult {
    pub winners: Vec<UserId>,
    pub rankings: Vec<UserId>,
    pub stats: serde_json::Value,
}
