//! Short-lived seat reservations so concurrent joins can't overfill a lobby
//! (e.g. checkers max 2) after both players have already paid on-chain.

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sw_domain::{LobbyId, UserId};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// Hold expires if the client abandons mid-join (pay / API).
const HOLD_TTL_SECS: i64 = 120;

fn key(lobby_id: LobbyId) -> String {
    format!("lobby:{}:seat-holds", lobby_id.as_uuid())
}

/// Atomic reserve: `participant_count + holds < max` (or already held).
const RESERVE_LUA: &str = r#"
local key = KEYS[1]
local uid = ARGV[1]
local maxp = tonumber(ARGV[2])
local part_count = tonumber(ARGV[3])
local ttl = tonumber(ARGV[4])
if redis.call('SISMEMBER', key, uid) == 1 then
  redis.call('EXPIRE', key, ttl)
  return 1
end
local holds = redis.call('SCARD', key)
if (part_count + holds) >= maxp then
  return 0
end
redis.call('SADD', key, uid)
redis.call('EXPIRE', key, ttl)
return 1
"#;

pub struct SeatHoldRepo {
    redis: ConnectionManager,
}

impl SeatHoldRepo {
    pub fn new(redis: ConnectionManager) -> Self {
        Self { redis }
    }

    /// Atomically reserve a seat when there is still capacity.
    /// `participant_count` is the current Postgres roster size (excluding this
    /// user — caller must reject "already in lobby" first).
    pub async fn try_reserve(
        &self,
        lobby_id: LobbyId,
        user_id: UserId,
        max_players: u8,
        participant_count: usize,
        participant_ids: &[UserId],
    ) -> AppResult<bool> {
        let mut redis = self.redis.clone();
        let key = key(lobby_id);

        // Holds that already converted to participants shouldn't count.
        for p in participant_ids {
            let _: Result<(), _> = redis.srem(&key, p.as_uuid().to_string()).await;
        }

        let script = redis::Script::new(RESERVE_LUA);
        let reserved: i32 = script
            .key(&key)
            .arg(user_id.as_uuid().to_string())
            .arg(max_players as i64)
            .arg(participant_count as i64)
            .arg(HOLD_TTL_SECS)
            .invoke_async(&mut redis)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(reserved == 1)
    }

    pub async fn release(&self, lobby_id: LobbyId, user_id: UserId) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let _: Result<(), _> = redis
            .srem(key(lobby_id), user_id.as_uuid().to_string())
            .await;
        Ok(())
    }

    pub async fn list(&self, lobby_id: LobbyId) -> AppResult<Vec<Uuid>> {
        let mut redis = self.redis.clone();
        let members: Vec<String> = redis
            .smembers(key(lobby_id))
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(members
            .into_iter()
            .filter_map(|s| Uuid::parse_str(&s).ok())
            .collect())
    }
}
