//! Register first-party game factories into the in-process registry.
//!
//! Add new games here

use anyhow::Context;
use sw_checkers::CheckersFactory;
use sw_lexi_wars::LexiWarsFactory;
use sw_ludo::LudoFactory;
use sw_ludo_rush::LudoRushFactory;
use sw_plugin::GameRegistry;

pub fn register_games(registry: &GameRegistry) -> anyhow::Result<()> {
    registry
        .register(CheckersFactory::arc())
        .context("register checkers")?;
    registry
        .register(LexiWarsFactory::arc())
        .context("register lexi-wars")?;
    registry
        .register(LudoFactory::arc())
        .context("register ludo")?;
    registry
        .register(LudoRushFactory::arc())
        .context("register ludo-rush")?;
    Ok(())
}
