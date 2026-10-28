//! Scavenger bots keep small lobbies lively and step aside as humans join.

use std::collections::HashMap;

use bevy::prelude::*;
use evoclash_game::evolution::{EvoPath, Evolution, Perk, Skill, SkillState};
use evoclash_game::items::{Arm, ArmorKind, GUN_SLOTS, HOTBAR_SLOTS, Item, WeaponKind};
use evoclash_game::map::ARENA_HALF;
use evoclash_game::nav::{NavGrid, clear_ahead, next_waypoint};
use evoclash_game::protocol::*;
use evoclash_game::shop::{PlayerCommand, ShopItem, Wallet, can_deploy};
use evoclash_game::sim::{Arena, DT, Obstacles, PLAYER_RADIUS, PLAYER_SPEED};
use rand::Rng;

use crate::config::ServerConfig;
use crate::connections::Human;
use crate::evolution::{LifeLog, apply_evolution, handle_command};
use crate::gameplay::{Aggressors, CurrentInput, SimSet, spawn_combatant, spawn_plain_combatant};
use crate::inventory::{BagContents, auto_wants};
use crate::rng::{SimRng, Stream};
use evoclash_game::evolution::seen_from;

mod build;
mod nav;
mod roster;
#[cfg(test)]
mod tests;
mod think;

use build::prepare;
use nav::rebuild_nav;
use roster::seed_match_once;
use think::think;

pub(crate) use roster::balance_bots;
pub use think::{aim_toward, engage_range};

/// Bots fire their skills at enemies this close.
const SKILL_RANGE: f32 = 500.0;

const SIGHT: f32 = 820.0;
const THINK_EVERY: u16 = 4;
/// How far ahead a bot checks its heading for walls: a bit more than it covers between thinks.
const PROBE: f32 = PLAYER_SPEED * DT * (THINK_EVERY + 2) as f32;
/// Think cycles a bot must keep a target before it opens fire (~0.3 s): a reaction time.
const REACTION_THINKS: u16 = 3;

/// Headless-only aim and reaction. Online matches never insert [`SkillMix`], so every bot stays
/// [`BotSkill::Default`] and the aim roll is the raw `±0.12` it has always been.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BotSkill {
    High,
    #[default]
    Default,
    Novice,
}

impl BotSkill {
    /// Multiplier on the `±0.12` aim roll. Default is 1, so the stored error is the roll itself.
    fn aim_scale(self) -> f32 {
        match self {
            // About ±0.03 rad, and they shoot after one extra think.
            Self::High => 0.25,
            Self::Default => 1.0,
            // About ±0.30 rad, and they wait roughly three times as long.
            Self::Novice => 2.5,
        }
    }

    fn reaction_thinks(self) -> u16 {
        match self {
            Self::High => 1,
            Self::Default => REACTION_THINKS,
            Self::Novice => 8,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Default => "default",
            Self::Novice => "novice",
        }
    }
}

/// Per-bot skill for a headless batch, in ascending `client_id` order (the same order
/// `seed_match_once` rolls personalities). Missing entries are [`BotSkill::Default`].
#[derive(Resource, Clone, Debug)]
pub struct SkillMix {
    pub tiers: Vec<BotSkill>,
}
// Target choice is "closest, weighted": below 1 is more attractive, above 1 less.
/// Monsters are easy loot, so bots go for them first.
const MONSTER_WEIGHT: f32 = 0.75;
/// Humans are slightly less attractive so bots spend more time fighting each other.
const HUMAN_WEIGHT: f32 = 1.3;
/// Each other bot already on a target makes it this much less attractive (no dogpiles).
const PILE_ON_WEIGHT: f32 = 0.8;
/// Bots stick with whoever they're already fighting rather than flicking between targets.
const STICKY_WEIGHT: f32 = 0.7;
const NAMES: [&str; 8] = [
    "拾荒者",
    "锈牙",
    "灰烬",
    "铁皮",
    "辐射鼠",
    "流浪者",
    "野狗",
    "老枪",
];

#[derive(Component)]
pub struct Bot {
    goal: Vec2,
    last_pos: Vec2,
    stuck: u16,
    strafe: f32,
    think_in: u16,
    /// Small per-bot aim error so they don't laser-beam you.
    aim_error: f32,
    target: Option<Entity>,
    /// Think cycles spent on the current target.
    locked_for: u16,
    /// Thinks on one target before this bot may fire. Default matches [`REACTION_THINKS`].
    reaction_thinks: u16,
    /// Set by [`SkillMix`] at match start. Online bots stay [`BotSkill::Default`].
    pub skill: BotSkill,
    /// Evolution path whose build order this bot spends its points on.
    pub path: EvoPath,
}

impl Bot {
    /// Placeholder brain. `seed_match_once` overwrites it when Playing starts.
    pub fn fresh() -> Self {
        Self {
            goal: Vec2::ZERO,
            last_pos: Vec2::ZERO,
            stuck: 0,
            strafe: 1.0,
            think_in: 0,
            aim_error: 0.0,
            target: None,
            locked_for: 0,
            reaction_thinks: REACTION_THINKS,
            skill: BotSkill::Default,
            path: EvoPath::Mutant,
        }
    }
}

/// Walkable grid for the current arena; raiders path with it too.
#[derive(Resource)]
pub struct Nav(pub NavGrid);

/// Unit tests insert this so a spawned body is not replicated. A match process never does.
#[derive(Resource)]
pub(crate) struct PlainBots;

pub struct BotsPlugin;

impl Plugin for BotsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (rebuild_nav, seed_match_once, balance_bots, prepare, think)
                .chain()
                .in_set(SimSet::Input)
                .in_set(crate::spectate::SeatSet::Bots),
        );
    }
}
