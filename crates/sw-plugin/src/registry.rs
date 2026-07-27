use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use sw_domain::{GameId, GameMetadata};

use crate::{GameFactory, PluginError, PluginResult};

/// In-process map of `game_id → factory`.
#[derive(Default)]
pub struct GameRegistry {
    factories: RwLock<HashMap<String, Arc<dyn GameFactory>>>,
}

impl GameRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, factory: Arc<dyn GameFactory>) -> PluginResult<()> {
        let id = factory.game_id().as_str().to_owned();
        let mut guard = self.factories.write();
        if guard.contains_key(&id) {
            return Err(PluginError::AlreadyRegistered(id));
        }
        guard.insert(id, factory);
        Ok(())
    }

    pub fn get(&self, game_id: &GameId) -> Option<Arc<dyn GameFactory>> {
        self.factories.read().get(game_id.as_str()).cloned()
    }

    pub fn list_metadata(&self) -> Vec<GameMetadata> {
        self.factories
            .read()
            .values()
            .map(|f| f.metadata())
            .collect()
    }

    pub fn contains(&self, game_id: &GameId) -> bool {
        self.factories.read().contains_key(game_id.as_str())
    }

    pub fn len(&self) -> usize {
        self.factories.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
