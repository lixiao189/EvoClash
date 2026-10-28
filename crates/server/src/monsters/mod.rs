//! Wasteland fauna. They give everyone — bots included — something to shoot that isn't you,
//! and they drop loot, so a fresh spawn has a way to gear up without a duel.
//!
//! Each kind lives in the city blocks that suit it (biome and tier), keeps to a cap and
//! trickles back in on its own timer. Monsters far from every player think only every
//! fourth tick, so a full arena stays cheap. The behemoth boss lives in `boss.rs`.

use std::collections::HashSet;
use std::f32::consts::{PI, TAU};

use bevy::prelude::*;
use evoclash_game::evolution::{STEALTH_SIGHT, seen_from};
use evoclash_game::items::{ArmorKind, Item, WeaponKind, roll_ammo, roll_loot};
use evoclash_game::map::{ARENA_HALF, Biome, Block, MARGIN, VAULT_CENTER};
use evoclash_game::nav::{next_waypoint, steer_clear};
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, DT, Obstacles, PLAYER_RADIUS};
use lightyear::prelude::*;
use rand::Rng;

use crate::boss::{self, BossState};
use crate::bots::{Nav, aim_toward, engage_range};
use crate::combat::{Flight, projectile_replication};
use crate::config::ServerConfig;
use crate::evolution::{Dealt, Hit, XpAward, XpQueue};
use crate::gameplay::{
    Damage, DamageQueue, MATCH_TICKS, Outbox, PosHistory, ResetRequested, SimSet, SimTick,
    TICKS_PER_SEC, match_flow, reset_match, resolve_damage,
};
use crate::inventory::spawn_loot;

mod attacks;
mod damage;
mod movement;
mod spawning;
#[cfg(test)]
mod tests;

pub use attacks::*;
pub use damage::*;
pub use movement::*;
pub use spawning::*;

/// Upper bound on live monsters (boss summons included), to keep the tick cheap.
pub const MAX_LIVE: usize = 56;
pub const MAX_ELITES: usize = 6;
/// Monsters with nobody this close think every `FAR_STRIDE` ticks.
const FAR: f32 = 900.0;
const FAR_STRIDE: u32 = 4;
/// Never spawn this close to a living player.
pub const SPAWN_CLEAR_OF_PLAYERS: f32 = 480.0;
pub const SPAWN_CLEAR_OF_VAULT: f32 = 320.0;
const LOOT_TTL: u32 = 30 * TICKS_PER_SEC;
/// Retry delay when no spot was found.
const RETRY_TICKS: u32 = 32;

const ROACH_WANDER: f32 = 120.0;
const ROACH_CHARGE: f32 = 235.0;
const ROACH_NOTICE: f32 = 300.0;
const ROACH_BITE: f32 = 8.0;
const ROACH_BITE_CD: u16 = 48;

/// Pods only spray while someone is close enough to care; saves bandwidth too.
const POD_WAKE: f32 = 680.0;
const POD_VOLLEY_EVERY: u16 = 90;
const POD_GLOBS: u32 = 10;
const SPORE_SPEED: f32 = 240.0;
const SPORE_TTL: u16 = 170;
const SPORE_DAMAGE: f32 = 9.0;

const DOG_WANDER: f32 = 150.0;
const DOG_CHARGE: f32 = 290.0;
const DOG_NOTICE: f32 = 420.0;
const DOG_BITE: f32 = 6.0;
const DOG_BITE_CD: u16 = 40;
/// Pack members come at the prey from these angles off the leader's line.
const DOG_FLANK: [f32; 3] = [0.0, PI / 3.0, -PI / 3.0];

const GHOUL_WANDER: f32 = 140.0;
const GHOUL_CHARGE: f32 = 250.0;
const GHOUL_NOTICE: f32 = 420.0;
const GHOUL_ARM_RANGE: f32 = 70.0;
const GHOUL_FUSE: u16 = 70;
pub const GHOUL_BLAST: f32 = 45.0;
/// Damage lost at the very edge of the blast.
const GHOUL_FALLOFF: f32 = 0.25;

const SPITTER_SPEED: f32 = 90.0;
const SPITTER_RANGE: f32 = 520.0;
const SPITTER_CD: u16 = 80;
const ACID_DAMAGE: f32 = 8.0;
const ACID_SPEED: f32 = 300.0;
const ACID_TTL: u16 = 90;
pub const ACID_PUDDLE_RADIUS: f32 = 55.0;
const ACID_PUDDLE_TTL: u16 = 3 * TICKS_PER_SEC as u16;
const ACID_DPS: f32 = 6.0;

const CRAWLER_WANDER: f32 = 70.0;
const CRAWLER_CHARGE: f32 = 130.0;
const CRAWLER_NOTICE: f32 = 380.0;
const CRAWLER_HIT: f32 = 22.0;
const CRAWLER_HIT_CD: u16 = 70;
const CRAWLER_TURN: f32 = 3.0;
/// Damage taken through the front shell.
pub const CRAWLER_SHELL_MULT: f32 = 0.35;

const RAIDER_SPEED: f32 = 170.0;
const RAIDER_SIGHT: f32 = 480.0;
/// Raiders fight anyone this close to their block, or anyone who shot them recently.
const RAIDER_TURF: f32 = 200.0;
const RAIDER_PROVOKED: u16 = 6 * TICKS_PER_SEC as u16;
const RAIDER_THINK: u16 = 4;

const DRONE_SPEED: f32 = 200.0;
const DRONE_RANGE: f32 = 400.0;
const DRONE_CD: u16 = 36;
const DRONE_LEASH: f32 = 600.0;
const BOLT_DAMAGE: f32 = 7.0;
const BOLT_SPEED: f32 = 560.0;
const BOLT_TTL: u16 = 50;

pub const RAD_PUDDLE_RADIUS: f32 = 150.0;
const RAD_PUDDLE_TTL: u16 = 4 * TICKS_PER_SEC as u16;
const RAD_DPS: f32 = 8.0;
/// Puddles bite four times a second.
const PUDDLE_EVERY: u32 = TICKS_PER_SEC / 4;

/// Spawning rules per kind. `cap` is for the whole map, not per block: individuals, or
/// groups when `group > 1` (a dog pack is 3, a raider team is 2).
pub struct Spec {
    /// Live individuals, or live groups when `group > 1`.
    pub cap: usize,
    pub group: usize,
    pub respawn: u32,
    /// Loot tier of a normal kill; elites drop one tier up.
    pub loot_tier: u8,
}

pub fn spec(kind: MonsterKind) -> Spec {
    let s = |cap, group, secs: u32, loot_tier| Spec {
        cap,
        group,
        respawn: secs * TICKS_PER_SEC,
        loot_tier,
    };
    match kind {
        MonsterKind::Roach => s(8, 1, 5, 1),
        MonsterKind::SporePod => s(4, 1, 5, 2),
        MonsterKind::Dog => s(3, 3, 20, 1),
        MonsterKind::Ghoul => s(5, 1, 15, 1),
        MonsterKind::Spitter => s(3, 1, 30, 2),
        MonsterKind::Crawler => s(2, 1, 45, 3),
        MonsterKind::Raider => s(2, 2, 60, 2),
        MonsterKind::Drone => s(3, 1, 30, 1),
        MonsterKind::Behemoth => s(0, 1, 0, 4),
    }
}

/// Whether `kind` lives in `block`.
pub fn block_suits(kind: MonsterKind, block: &Block) -> bool {
    use Biome::*;
    let (b, tier) = (block.biome, block.tier);
    match kind {
        _ if b == Vault => false,
        MonsterKind::Roach | MonsterKind::SporePod => true,
        MonsterKind::Dog => matches!(b, Wasteland | Urban),
        MonsterKind::Ghoul => matches!(b, Urban | Industrial) && tier >= 2,
        MonsterKind::Spitter => matches!(b, Industrial | Radiation),
        // Industrial and radiation blocks, plus the ring around the vault.
        MonsterKind::Crawler => matches!(b, Industrial | Radiation) || tier >= 2,
        MonsterKind::Raider => (b == Urban && tier >= 2) || b == Industrial,
        MonsterKind::Drone => matches!(b, Wasteland | Radiation),
        MonsterKind::Behemoth => false,
    }
}

fn can_be_elite(kind: MonsterKind) -> bool {
    !matches!(kind, MonsterKind::Raider | MonsterKind::Behemoth)
}

fn kind_index(kind: MonsterKind) -> usize {
    MonsterKind::ALL
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(0)
}

/// Server-only monster state.
#[derive(Component)]
pub struct Brain {
    heading: Vec2,
    turn_in: u16,
    attack_cd: u16,
    /// Pods rotate each ring a little so the pattern spirals.
    spiral: f32,
    /// Dog pack / raider squad id (0 = loner) and position in it.
    pack: u32,
    slot: u8,
    /// Where it spawned and the block it calls home.
    home: Vec2,
    turf: Rect,
    speed_mult: f32,
    pub(crate) damage_mult: f32,
    /// Facing angle (crawlers turn slowly; raiders face their aim).
    facing: f32,
    /// Nobody nearby: thinking at a reduced rate.
    far: bool,
    // Raider gun-play.
    weapon: WeaponKind,
    aim_error: f32,
    provoked: u16,
    goal: Vec2,
    move_dir: Vec2,
    aim: Vec2,
    fire: bool,
    locked_for: u16,
    target: Option<Entity>,
    think_in: u16,
    /// Spawn order this match. Far-stride and replay sort by this, never by entity id.
    pub serial: u32,
}

impl Brain {
    pub fn new(
        kind: MonsterKind,
        elite: bool,
        home: Vec2,
        turf: Rect,
        pack: u32,
        slot: u8,
        rng: &mut impl Rng,
    ) -> Self {
        let heading = Vec2::from_angle(rng.random_range(0.0..TAU));
        Self {
            heading,
            turn_in: 0,
            attack_cd: if kind == MonsterKind::SporePod {
                rng.random_range(0..POD_VOLLEY_EVERY)
            } else {
                0
            },
            spiral: rng.random_range(0.0..1.0),
            pack,
            slot,
            home,
            turf,
            speed_mult: if elite { ELITE_SPEED_MULT } else { 1.0 },
            damage_mult: if elite { ELITE_DAMAGE_MULT } else { 1.0 },
            facing: heading.to_angle(),
            far: false,
            weapon: if rng.random_bool(0.7) {
                WeaponKind::Rifle
            } else {
                WeaponKind::PipeShotgun
            },
            aim_error: rng.random_range(-0.1..0.1),
            provoked: 0,
            goal: home,
            move_dir: Vec2::ZERO,
            aim: heading,
            fire: false,
            locked_for: 0,
            target: None,
            think_in: 0,
            serial: 0,
        }
    }
}

/// Blown up this tick; despawned at the end of the tick so the kill feed can still name it.
#[derive(Component)]
pub struct Dying;

/// Server-side puddle lifetime.
#[derive(Component)]
struct PuddleLife {
    ttl: u16,
    dps: f32,
    owner: Option<Entity>,
    cause: &'static str,
}

pub struct PendingBlast {
    pub pos: Vec2,
    pub attacker: Option<Entity>,
    pub source: Entity,
    pub damage: f32,
}

/// Ghoul blasts waiting to go off (a ghoul shot dead blows up on the next tick).
#[derive(Resource, Default)]
pub struct GhoulBlasts(pub Vec<PendingBlast>);

#[derive(Default, Clone, Copy)]
struct KindTimer {
    /// Ticks until the next spawn while below the cap.
    wait: u32,
    /// Reached its cap once this match; until then it refills without waiting.
    filled: bool,
}

#[derive(Resource, Default)]
pub struct Spawner {
    timers: [KindTimer; MonsterKind::ALL.len()],
    next_pack: u32,
    next_serial: u32,
}

impl Spawner {
    /// Stable id for think-stride and replay ordering. Not an entity id.
    pub fn alloc_serial(&mut self) -> u32 {
        let serial = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1);
        serial
    }
}

/// Who is alive right now, per kind.
#[derive(Default, Debug, Clone)]
pub struct Census {
    pub live: [usize; MonsterKind::ALL.len()],
    pub groups: [usize; MonsterKind::ALL.len()],
    pub elites: usize,
    pub total: usize,
}

impl Census {
    pub fn add(&mut self, kind: MonsterKind, elite: bool, new_group: bool) {
        let i = kind_index(kind);
        self.live[i] += 1;
        self.groups[i] += usize::from(new_group);
        self.elites += usize::from(elite);
        self.total += 1;
    }

    /// Whether one more group of `kind` fits under its cap and the global cap.
    pub fn wants(&self, kind: MonsterKind) -> bool {
        let s = spec(kind);
        let i = kind_index(kind);
        let have = if s.group > 1 {
            self.groups[i]
        } else {
            self.live[i]
        };
        have < s.cap && self.total + s.group <= MAX_LIVE
    }
}

pub fn census(monsters: impl Iterator<Item = (MonsterKind, bool, u32)>) -> Census {
    let mut c = Census::default();
    let mut packs = HashSet::new();
    for (kind, elite, pack) in monsters {
        let new_group = pack == 0 || packs.insert(pack);
        c.add(kind, elite, new_group);
    }
    c
}

pub struct MonstersPlugin;

impl Plugin for MonstersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Spawner>()
            .init_resource::<GhoulBlasts>();
        app.add_systems(
            FixedUpdate,
            (
                (populate, roam).chain().in_set(SimSet::Simulate),
                (attack, ghoul_blasts, puddles)
                    .chain()
                    .in_set(SimSet::Combat),
                resolve_monster_damage
                    .in_set(SimSet::Resolve)
                    .before(resolve_damage),
                (reap, reset_monsters)
                    .chain()
                    .in_set(SimSet::Flow)
                    .after(match_flow)
                    .before(reset_match),
            ),
        );
    }
}
