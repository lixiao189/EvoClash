//! Authoritative simulation: inputs, movement, damage, respawns, loot and the match clock.

use std::collections::VecDeque;

use bevy::prelude::*;
use evoclash_game::evolution::{Evolution, Skill, SkillState, player_kill_xp};
use evoclash_game::items::{AmmoKind, ArmorKind, Item, WeaponKind, roll_loot};
use evoclash_game::map::{ArenaMap, VAULT_CENTER};
use evoclash_game::protocol::*;
use evoclash_game::rng::mix;
use evoclash_game::shop::{DeathRecap, RESPAWN_MAX_TICKS, Wallet, can_deploy, player_kill_scrap};
use evoclash_game::sim::{self, Arena, Obstacles, gear_tier, underdog_mult};
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use crate::config::ServerConfig;
use crate::connections::Human;
use crate::environment::{Blasts, PropSlots};
use crate::evolution::{Dealt, LifeLog, Recovery, XpAward, XpQueue};
use crate::inventory::{Selecting, spawn_bag, spawn_loot, take};
use crate::ops::OpsState;
use crate::rng::SimRng;
use crate::skills::HiddenFrom;

mod damage;
mod flow;
mod movement;
mod room;
mod spawning;
#[cfg(test)]
mod tests;

pub use damage::*;
pub use flow::*;
pub use movement::*;
use room::{maintain_host, private_idle_cap, read_room_commands};
pub use spawning::*;

pub const TICKS_PER_SEC: u32 = 64;
/// Lobby portion of the public 20s countdown. The last 3s are `Phase::Starting`.
pub const LOBBY_COUNTDOWN_TICKS: u32 = 17 * TICKS_PER_SEC;
pub const STARTING_TICKS: u32 = 3 * TICKS_PER_SEC;
pub const MATCH_TICKS: u32 = 8 * 60 * TICKS_PER_SEC;
pub const RESULTS_TICKS: u32 = 12 * TICKS_PER_SEC;
/// Quick start stops once fewer than this many ticks remain until Playing
/// (lobby time left plus the 3s Starting). Exactly 8s still accepts.
pub const JOIN_CUTOFF_TICKS: u32 = 8 * TICKS_PER_SEC;
/// Lobby portion of a private room's 5s start. The last 3s are `Phase::Starting`.
pub const PRIVATE_LOBBY_COUNTDOWN_TICKS: u32 = 2 * TICKS_PER_SEC;
const EMPTY_LOBBY_TICKS: u32 = 30 * TICKS_PER_SEC;
const PRIVATE_EMPTY_LOBBY_TICKS: u32 = 120 * TICKS_PER_SEC;
const EMPTY_MATCH_TICKS: u32 = 10 * TICKS_PER_SEC;
const PRIVATE_IDLE_WARN_TICKS: u32 = 28 * 60 * TICKS_PER_SEC;
const PRIVATE_IDLE_CLOSE_TICKS: u32 = 30 * 60 * TICKS_PER_SEC;
/// Lobby deaths stand back up after this, with no death bag.
pub(super) const LOBBY_RESPAWN_TICKS: u16 = (2 * TICKS_PER_SEC) as u16;
pub(crate) const FIRST_VAULT_TICKS: u32 = 60 * TICKS_PER_SEC;
const VAULT_INTERVAL_TICKS: u32 = 90 * TICKS_PER_SEC;
pub const SCORE_TO_WIN: u32 = 25;
const DROP_TTL_TICKS: u32 = 30 * TICKS_PER_SEC;
/// Fresh spawns can't be hurt for this long (or until they shoot), so nobody gets spawn-camped.
pub(crate) const SPAWN_SHIELD_TICKS: u16 = (2.5 * TICKS_PER_SEC as f32) as u16;
/// Killing someone whose gear is this many tiers above your weapon earns an extra point.
const GIANT_SLAYER_GAP: u8 = 2;
/// Upper bound on lag compensation (~220 ms) so high-ping players can't shoot far into the past.
pub const MAX_REWIND_TICKS: u32 = 14;
const HISTORY_LEN: usize = 32;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SimSet {
    Input,
    Simulate,
    Combat,
    Resolve,
    Flow,
}

pub struct GameplayPlugin;

impl Plugin for GameplayPlugin {
    fn build(&self, app: &mut App) {
        let room_seed = app
            .world()
            .get_resource::<ServerConfig>()
            .map(|config| config.room_seed)
            .unwrap_or(0);
        // First lobby of the process is match_index 0. Later rounds remix the same room seed.
        let map_seed = mix(room_seed, 0);
        app.insert_resource(Arena(ArenaMap::generate(map_seed)));
        app.insert_resource(SimRng::from_match_seed(map_seed));
        app.init_resource::<SimTick>()
            .init_resource::<Outbox>()
            .init_resource::<DamageQueue>()
            .init_resource::<LootState>()
            .init_resource::<ResetRequested>()
            .init_resource::<PendingStart>();
        app.configure_sets(
            FixedUpdate,
            (
                SimSet::Input,
                SimSet::Simulate,
                SimSet::Combat,
                SimSet::Resolve,
                SimSet::Flow,
            )
                .chain(),
        );
        app.add_systems(Startup, spawn_match_state);
        app.add_systems(
            FixedUpdate,
            (
                advance_tick.in_set(SimSet::Input).before(read_human_inputs),
                read_human_inputs.in_set(SimSet::Input),
                read_room_commands.in_set(SimSet::Input),
                (
                    drop_shield_on_attack,
                    movement,
                    record_history,
                    tick_cooldowns,
                )
                    .chain()
                    .in_set(SimSet::Simulate),
                (resolve_damage, respawn, expire_drops)
                    .chain()
                    .in_set(SimSet::Resolve),
                (
                    loot_spawner,
                    match_flow,
                    reset_match,
                    place_starters,
                    maintain_host,
                    empty_close,
                    private_idle_cap,
                    flush_outbox,
                    clear_outbox,
                )
                    .chain()
                    .in_set(SimSet::Flow),
            ),
        );
    }
}

#[derive(Resource, Default)]
pub struct SimTick(pub u32);

/// Events to broadcast this tick, flushed once at the end of the tick.
#[derive(Resource, Default)]
pub struct Outbox {
    pub reliable: Vec<GameEvent>,
    pub fast: Vec<GameEvent>,
    /// Fast events for one combatant only (dropped for bots).
    pub direct: Vec<(Entity, GameEvent)>,
}

impl Outbox {
    pub fn announce(&mut self, text: impl Into<String>) {
        self.reliable.push(GameEvent::Announce(text.into()));
    }
}

pub struct Damage {
    pub target: Entity,
    pub attacker: Option<Entity>,
    pub amount: f32,
    pub armor_mult: f32,
    /// `None` for monster bites, spores and the environment.
    pub weapon: Option<WeaponKind>,
    /// EMP blast: strips armor and slows instead of (mostly) hurting.
    pub emp: bool,
    /// What the kill feed calls it when it isn't a weapon, e.g. "油桶爆炸".
    pub cause: Option<&'static str>,
}

#[derive(Resource, Default)]
pub struct DamageQueue(pub Vec<Damage>);

#[derive(Resource, Default)]
pub struct ResetRequested(pub bool);

/// Entering Starting: after `reset_match`, stand everyone on a spawn, alive and unshielded.
#[derive(Resource, Default)]
pub struct PendingStart(pub bool);

/// Humans who have hurt this bot during the lobby. Lobby bots only fight back against these.
#[derive(Component, Default)]
pub struct Aggressors(pub Vec<Entity>);

/// Public lobby still offered by quick start. The 20s clock is lobby time plus Starting;
/// a seat is offered while that clock has not started, or still has at least 8s left.
pub fn lobby_open(state: &MatchState) -> bool {
    state.phase == Phase::Lobby
        && (!state.countdown_active
            || state.ticks_left.saturating_add(STARTING_TICKS) >= JOIN_CUTOFF_TICKS)
}

/// What the heartbeat should advertise. Private rooms close the moment the host starts;
/// public rooms stay open until the last 8s, which `lobby_open` already encodes.
pub fn current_join_mode(state: &MatchState, private_room: bool, accepting: bool) -> &'static str {
    if !accepting {
        return "none";
    }
    if private_room {
        if state.phase == Phase::Lobby && !state.countdown_active {
            "lobby"
        } else {
            "none"
        }
    } else if lobby_open(state) {
        "lobby"
    } else {
        "none"
    }
}

pub fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Lobby => "lobby",
        Phase::Starting => "starting",
        Phase::Playing => "playing",
        Phase::Results => "results",
        Phase::Recovering => "recovering",
    }
}

/// Seconds left in the phase. `None` until a lobby countdown starts.
pub fn phase_left_secs(state: &MatchState) -> Option<u32> {
    if state.phase == Phase::Lobby && !state.countdown_active {
        None
    } else {
        Some(state.ticks_left / TICKS_PER_SEC)
    }
}

/// The input applied this tick, whether it came from the network or a bot brain.
#[derive(Component, Default)]
pub struct CurrentInput(pub PlayerInput);

#[derive(Component, Default)]
pub struct Cooldowns {
    pub fire: u16,
    pub interact: u16,
    pub throw: u16,
}

/// Recent positions for lag compensation.
#[derive(Component, Default)]
pub struct PosHistory(VecDeque<(u32, Vec2)>);

impl PosHistory {
    pub fn at(&self, tick: u32) -> Option<Vec2> {
        self.0
            .iter()
            .rev()
            .find(|(t, _)| *t <= tick)
            .or(self.0.front())
            .map(|(_, p)| *p)
    }
}

/// How many ticks into the past this combatant's shots are resolved.
#[derive(Component, Default)]
pub struct Rewind(pub u32);

#[derive(Component)]
pub struct Expires(pub u32);

#[derive(Default)]
struct SpotState {
    entity: Option<Entity>,
    respawn_in: u32,
}

#[derive(Resource, Default)]
pub struct LootState {
    spots: Vec<SpotState>,
}
