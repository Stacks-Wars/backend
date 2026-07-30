use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sw_domain::{LobbyId, LobbyState, PlayerState, UserId};

use crate::error::{AppError, AppResult};

fn lobby_state_key(lobby_id: LobbyId) -> String {
    format!("lobby:{}:state", lobby_id.as_uuid())
}

fn player_key(lobby_id: LobbyId, user_id: UserId) -> String {
    format!(
        "lobby:{}:player:{}",
        lobby_id.as_uuid(),
        user_id.as_uuid()
    )
}

fn players_set_key(lobby_id: LobbyId) -> String {
    format!("lobby:{}:players", lobby_id.as_uuid())
}

pub struct LobbyStateRepo {
    redis: ConnectionManager,
}

impl LobbyStateRepo {
    pub fn new(redis: ConnectionManager) -> Self {
        Self { redis }
    }

    pub async fn set(&self, state: &LobbyState) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let key = lobby_state_key(state.lobby_id);
        let payload =
            serde_json::to_string(state).map_err(|e| AppError::Internal(e.into()))?;
        redis
            .set::<_, _, ()>(key, payload)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn get(&self, lobby_id: LobbyId) -> AppResult<Option<LobbyState>> {
        let mut redis = self.redis.clone();
        let key = lobby_state_key(lobby_id);
        let raw: Option<String> = redis
            .get(key)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        match raw {
            Some(raw) => {
                let state = serde_json::from_str(&raw).map_err(|e| AppError::Internal(e.into()))?;
                Ok(Some(state))
            }
            None => Ok(None),
        }
    }
}

pub struct PlayerStateRepo {
    redis: ConnectionManager,
}

impl PlayerStateRepo {
    pub fn new(redis: ConnectionManager) -> Self {
        Self { redis }
    }

    pub async fn set(&self, lobby_id: LobbyId, player: &PlayerState) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let key = player_key(lobby_id, player.user_id);
        let set_key = players_set_key(lobby_id);
        let payload =
            serde_json::to_string(player).map_err(|e| AppError::Internal(e.into()))?;
        redis
            .set::<_, _, ()>(key, payload)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        redis
            .sadd::<_, _, ()>(set_key, player.user_id.as_uuid().to_string())
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn list(&self, lobby_id: LobbyId) -> AppResult<Vec<PlayerState>> {
        let mut redis = self.redis.clone();
        let set_key = players_set_key(lobby_id);
        let ids: Vec<String> = redis
            .smembers(set_key)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;

        let mut players = Vec::with_capacity(ids.len());
        for id in ids {
            let Ok(uuid) = uuid::Uuid::parse_str(&id) else {
                continue;
            };
            let key = player_key(lobby_id, UserId::from(uuid));
            let raw: Option<String> = redis
                .get(key)
                .await
                .map_err(|e| AppError::Internal(e.into()))?;
            if let Some(raw) = raw {
                let player: PlayerState =
                    serde_json::from_str(&raw).map_err(|e| AppError::Internal(e.into()))?;
                players.push(player);
            }
        }
        Ok(players)
    }

    pub async fn delete(&self, lobby_id: LobbyId, user_id: UserId) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let key = player_key(lobby_id, user_id);
        let set_key = players_set_key(lobby_id);
        redis
            .del::<_, ()>(key)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        redis
            .srem::<_, _, ()>(set_key, user_id.as_uuid().to_string())
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }
}
