//! Wasteland fauna. They give everyone — bots included — something to shoot that isn't you,
//! and they drop loot, so a fresh spawn has a way to gear up without a duel.
//!
//! Each kind lives in the city blocks that suit it (biome and tier), keeps to a cap and
//! trickles back in on its own timer. Monsters far from every player think only every
//! fourth tick, so a full arena stays cheap. The behemoth boss lives in `boss.rs`.

use std::collections::{HashMap, HashSet};
use std::f32::consts::{PI, TAU};

use bevy::prelude::*;
use evoclash_game::evolution::{STEALTH_SIGHT, seen_from};
use evoclash_game::items::{ArmorKind, Item, WeaponKind, roll_loot};
use evoclash_game::map::{ARENA_HALF, Biome, Block, MARGIN, VAULT_CENTER};
use evoclash_game::nav::next_waypoint;
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
    Damage, DamageQueue, Outbox, PosHistory, ResetRequested, SimSet, SimTick, TICKS_PER_SEC,
    match_flow, reset_match, resolve_damage, spawn_pickup,
};

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
const ROACH_BITE: f32 = 13.0;
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
const DOG_BITE: f32 = 10.0;
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
const ACID_PUDDLE_TTL: u16 = 6 * TICKS_PER_SEC as u16;
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
const RAIDER_SIGHT: f32 = 700.0;
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

/// Spawning rules per kind.
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
        MonsterKind::Roach => s(10, 1, 5, 1),
        MonsterKind::SporePod => s(5, 1, 5, 2),
        MonsterKind::Dog => s(4, 3, 20, 1),
        MonsterKind::Ghoul => s(6, 1, 15, 1),
        MonsterKind::Spitter => s(4, 1, 30, 2),
        MonsterKind::Crawler => s(3, 1, 45, 3),
        MonsterKind::Raider => s(2, 2, 60, 2),
        MonsterKind::Drone => s(4, 1, 30, 1),
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
}

impl Brain {
    pub fn new(
        kind: MonsterKind,
        elite: bool,
        home: Vec2,
        turf: Rect,
        pack: u32,
        slot: u8,
    ) -> Self {
        let mut rng = rand::rng();
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

// ---------------------------------------------------------------- spawning

pub fn over_building(arena: &Arena, p: Vec2, radius: f32) -> bool {
    arena
        .buildings
        .iter()
        .any(|b| b.rect.inflate(radius).contains(p))
}

/// Whether a `kind` may appear at `p`: clear of walls, props, the vault and living players.
pub fn spot_clear(
    arena: &Arena,
    kind: MonsterKind,
    p: Vec2,
    obstacles: &[(Vec2, f32)],
    players: &[Vec2],
) -> bool {
    let radius = kind.radius();
    arena.resolve_with(p, radius + 12.0, obstacles) == p
        && p.distance(VAULT_CENTER) > SPAWN_CLEAR_OF_VAULT
        && arena.block_at(p).is_some_and(|b| b.biome != Biome::Vault)
        && players
            .iter()
            .all(|q| q.distance(p) > SPAWN_CLEAR_OF_PLAYERS)
        && (!kind.flies() || !over_building(arena, p, radius + 8.0))
}

/// A random clear spot in a block that suits `kind`.
pub fn pick_spot(
    arena: &Arena,
    kind: MonsterKind,
    obstacles: &[(Vec2, f32)],
    players: &[Vec2],
    rng: &mut impl Rng,
) -> Option<(Vec2, Block)> {
    let blocks: Vec<&Block> = arena
        .blocks
        .iter()
        .filter(|b| block_suits(kind, b))
        .collect();
    if blocks.is_empty() {
        return None;
    }
    let inset = MARGIN + kind.radius();
    for _ in 0..24 {
        let block = blocks[rng.random_range(0..blocks.len())];
        let r = block.rect.inflate(-inset);
        let p = Vec2::new(
            rng.random_range(r.min.x..r.max.x),
            rng.random_range(r.min.y..r.max.y),
        );
        if spot_clear(arena, kind, p, obstacles, players) {
            return Some((p, *block));
        }
    }
    None
}

/// Spawn one monster. `max_hp` overrides the kind's (elite) default.
pub fn spawn_monster(
    commands: &mut Commands,
    kind: MonsterKind,
    elite: bool,
    pos: Vec2,
    brain: Brain,
    max_hp: Option<f32>,
) -> Entity {
    let hp = max_hp.unwrap_or(kind.max_hp() * if elite { ELITE_HP_MULT } else { 1.0 });
    let facing = brain.facing;
    commands
        .spawn((
            Name::new(kind.name()),
            Monster { kind, elite },
            MonsterState {
                facing,
                phase: u8::from(kind == MonsterKind::Behemoth),
                ..default()
            },
            Pos(pos),
            Vitals { hp, ..default() },
            PosHistory::default(),
            brain,
            Replicate::to_clients(NetworkTarget::All),
            InterpolationTarget::to_clients(NetworkTarget::All),
        ))
        .id()
}

fn populate(
    mut commands: Commands,
    config: Res<ServerConfig>,
    arena: Res<Arena>,
    obstacles: Res<Obstacles>,
    state: Single<&MatchState>,
    mut spawner: ResMut<Spawner>,
    monsters: Query<(&Monster, &Brain)>,
    players: Query<(&Pos, &Vitals), With<Player>>,
) {
    if !config.monsters || state.phase == Phase::Intermission {
        return;
    }
    let mut live = census(monsters.iter().map(|(m, b)| (m.kind, m.elite, b.pack)));
    let warmup = state.phase == Phase::Warmup;
    let mut candidate = None;
    for kind in MonsterKind::ALL {
        if kind == MonsterKind::Behemoth {
            continue;
        }
        let s = spec(kind);
        let timer = &mut spawner.timers[kind_index(kind)];
        if !live.wants(kind) {
            // At the cap: the respawn clock only starts once one dies.
            timer.filled = true;
            timer.wait = s.respawn;
            continue;
        }
        if warmup || !timer.filled {
            timer.wait = timer.wait.min(1);
        }
        timer.wait = timer.wait.saturating_sub(1);
        if timer.wait == 0 && candidate.is_none() {
            candidate = Some(kind);
        }
    }
    // At most one group per tick spreads the cost of a refill.
    let Some(kind) = candidate else {
        return;
    };
    let alive: Vec<Vec2> = players
        .iter()
        .filter(|(_, v)| v.alive)
        .map(|(p, _)| p.0)
        .collect();
    let mut rng = rand::rng();
    let Some((spot, block)) = pick_spot(&arena, kind, &obstacles.0, &alive, &mut rng) else {
        spawner.timers[kind_index(kind)].wait = RETRY_TICKS;
        return;
    };
    let s = spec(kind);
    spawner.timers[kind_index(kind)].wait = s.respawn;
    let pack = if s.group > 1 {
        spawner.next_pack += 1;
        spawner.next_pack
    } else {
        0
    };
    for slot in 0..s.group {
        let pos = if slot == 0 {
            spot
        } else {
            // Followers stand around the leader where there is room.
            (0..8)
                .map(|i| spot + Vec2::from_angle(i as f32 * TAU / 8.0 + slot as f32) * 48.0)
                .find(|p| spot_clear(&arena, kind, *p, &obstacles.0, &alive))
                .unwrap_or(spot)
        };
        let elite = can_be_elite(kind)
            && block.tier >= 2
            && live.elites < MAX_ELITES
            && rng.random_ratio(1, 6);
        live.add(kind, elite, slot == 0);
        let brain = Brain::new(kind, elite, spot, block.rect, pack, slot as u8);
        spawn_monster(&mut commands, kind, elite, pos, brain, None);
    }
}

pub fn spawn_puddle(commands: &mut Commands, kind: PuddleKind, pos: Vec2, owner: Option<Entity>) {
    let (radius, ttl, dps, cause) = match kind {
        PuddleKind::Acid => (ACID_PUDDLE_RADIUS, ACID_PUDDLE_TTL, ACID_DPS, "酸液"),
        PuddleKind::Radiation => (RAD_PUDDLE_RADIUS, RAD_PUDDLE_TTL, RAD_DPS, "辐射"),
    };
    commands.spawn((
        Name::new("Puddle"),
        Puddle { kind, radius, ttl },
        Pos(pos),
        PuddleLife {
            ttl,
            dps,
            owner,
            cause,
        },
        Replicate::to_clients(NetworkTarget::All),
    ));
}

/// Fire a monster projectile from `from` along `dir`.
pub fn shoot(
    commands: &mut Commands,
    kind: ProjectileKind,
    from: Vec2,
    dir: Vec2,
    speed: f32,
    flight: Flight,
) {
    commands.spawn((
        Name::new("MonsterShot"),
        Projectile {
            kind,
            vel: dir * speed,
        },
        Pos(from),
        flight,
        projectile_replication(None),
    ));
}

// ---------------------------------------------------------------- movement

/// Nearest living combatant, if any.
/// A living player a monster might go after: entity, position, and whether they are cloaked
/// (stealthed and not revealed).
pub type Prey = (Entity, Vec2, bool);

/// Every living player as [`Prey`].
pub fn prey<'a>(players: impl Iterator<Item = (Entity, &'a Pos, &'a Vitals)>) -> Vec<Prey> {
    players
        .filter(|(_, _, v)| v.alive)
        .map(|(e, p, v)| (e, p.0, !seen_from(v, p.0, Vec2::splat(f32::INFINITY))))
        .collect()
}

/// Like bots, monsters lose track of a cloaked player beyond `STEALTH_SIGHT`.
pub fn noticed(cloaked: bool, distance: f32) -> bool {
    !cloaked || distance <= STEALTH_SIGHT
}

/// The closest player this monster can notice.
pub fn nearest(players: &[Prey], from: Vec2) -> Option<(Entity, Vec2, f32)> {
    players
        .iter()
        .map(|(e, p, c)| (*e, *p, p.distance(from), *c))
        .filter(|(.., d, c)| noticed(*c, *d))
        .map(|(e, p, d, _)| (e, p, d))
        .min_by(|a, b| a.2.total_cmp(&b.2))
}

fn wrap_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}

/// Write `state` only when something visible changed (facing in ~6° steps).
pub fn set_state(state: &mut Mut<MonsterState>, new: MonsterState) {
    let turned = wrap_angle(new.facing - state.facing).abs() > 0.1;
    if turned || new.phase != state.phase || new.action != state.action || new.fuse != state.fuse {
        let facing = if turned { new.facing } else { state.facing };
        **state = MonsterState { facing, ..new };
    }
}

fn wander(brain: &mut Brain, rng: &mut impl Rng, step: u16) -> Vec2 {
    if brain.turn_in == 0 {
        brain.heading = Vec2::from_angle(rng.random_range(0.0..TAU));
        brain.turn_in = rng.random_range(40..160);
    }
    brain.turn_in = brain.turn_in.saturating_sub(step);
    brain.heading
}

#[allow(clippy::type_complexity)]
fn roam(
    tick: Res<SimTick>,
    arena: Res<Arena>,
    obstacles: Res<Obstacles>,
    nav: Option<Res<Nav>>,
    mut monsters: Query<(
        Entity,
        &Monster,
        &mut Pos,
        &mut Brain,
        &mut MonsterState,
        &Vitals,
    )>,
    players: Query<(Entity, &Pos, &Vitals), (With<Player>, Without<Monster>)>,
) {
    let alive = prey(players.iter());
    let mut rng = rand::rng();

    // Each dog pack follows its lowest-slot member: (slot, position, heading).
    let mut packs: HashMap<u32, (u8, Vec2, Vec2)> = HashMap::new();
    for (_, monster, pos, brain, _, vitals) in &monsters {
        if monster.kind != MonsterKind::Dog || brain.pack == 0 || !vitals.alive {
            continue;
        }
        let entry = packs
            .entry(brain.pack)
            .or_insert((brain.slot, pos.0, brain.heading));
        if brain.slot < entry.0 {
            *entry = (brain.slot, pos.0, brain.heading);
        }
    }
    let pack_prey: HashMap<u32, Vec2> = packs
        .iter()
        .filter_map(|(id, (_, at, _))| {
            nearest(&alive, *at)
                .filter(|(_, p, d)| *d < DOG_NOTICE && arena.line_of_sight(*at, *p))
                .map(|(_, p, _)| (*id, p))
        })
        .collect();

    for (entity, monster, mut pos, mut brain, mut state, vitals) in &mut monsters {
        let kind = monster.kind;
        if !vitals.alive || matches!(kind, MonsterKind::Behemoth | MonsterKind::SporePod) {
            continue;
        }
        let near = nearest(&alive, pos.0);
        brain.far = near.is_none_or(|(_, _, d)| d > FAR);
        let step = if brain.far {
            if !(tick.0.wrapping_add(entity.to_bits() as u32)).is_multiple_of(FAR_STRIDE) {
                continue;
            }
            FAR_STRIDE
        } else {
            1
        };
        let dt = DT * step as f32;
        let at = pos.0;
        let sees = |range: f32| near.filter(|(_, p, d)| *d < range && arena.line_of_sight(at, *p));
        let speed = brain.speed_mult;
        let s16 = step as u16;

        let velocity = match kind {
            // No pathfinding on purpose: a roach that sees you runs straight at you.
            MonsterKind::Roach => match sees(ROACH_NOTICE) {
                Some((_, p, _)) => (p - at).normalize_or_zero() * ROACH_CHARGE * speed,
                None => wander(&mut brain, &mut rng, s16) * ROACH_WANDER * speed,
            },
            MonsterKind::Dog => {
                let leader = packs.get(&brain.pack).copied();
                match (pack_prey.get(&brain.pack).copied(), leader) {
                    (Some(q), Some((_, lead_at, _))) => {
                        // Members fan out ±60° around the leader's line of attack.
                        let base = (lead_at - q).normalize_or(Vec2::X);
                        let flank = DOG_FLANK[brain.slot as usize % DOG_FLANK.len()];
                        let target = if at.distance(q) > 110.0 {
                            q + Vec2::from_angle(flank).rotate(base) * 70.0
                        } else {
                            q
                        };
                        (target - at).normalize_or_zero() * DOG_CHARGE * speed
                    }
                    (None, Some((slot, lead_at, heading))) if slot != brain.slot => {
                        // Trot after the leader.
                        brain.heading = heading;
                        let spot = lead_at + Vec2::from_angle(brain.slot as f32 * 2.1) * 45.0;
                        if spot.distance(at) > 30.0 {
                            (spot - at).normalize_or_zero() * DOG_WANDER * speed
                        } else {
                            Vec2::ZERO
                        }
                    }
                    _ => wander(&mut brain, &mut rng, s16) * DOG_WANDER * speed,
                }
            }
            MonsterKind::Ghoul => {
                let armed = state.action == MonsterAction::Arming;
                match near.filter(|_| armed).or_else(|| sees(GHOUL_NOTICE)) {
                    Some((_, p, _)) => {
                        let s = if armed { 0.6 } else { 1.0 };
                        (p - at).normalize_or_zero() * GHOUL_CHARGE * speed * s
                    }
                    None => wander(&mut brain, &mut rng, s16) * GHOUL_WANDER * speed,
                }
            }
            MonsterKind::Spitter => match sees(SPITTER_RANGE) {
                Some((_, p, d)) => {
                    let to = (p - at).normalize_or_zero();
                    if rng.random_bool(0.01) {
                        brain.heading = -brain.heading;
                    }
                    let dir = if d < 260.0 {
                        -to
                    } else if d > 480.0 {
                        to
                    } else {
                        to.perp() * brain.heading.x.signum() * 0.6
                    };
                    dir * SPITTER_SPEED * speed
                }
                None => wander(&mut brain, &mut rng, s16) * SPITTER_SPEED * speed,
            },
            MonsterKind::Crawler => {
                let (want, max) = match sees(CRAWLER_NOTICE) {
                    Some((_, p, _)) => ((p - at).normalize_or(brain.heading), CRAWLER_CHARGE),
                    None => (wander(&mut brain, &mut rng, s16), CRAWLER_WANDER),
                };
                // Heavy shell: it turns slowly, so flanking it is the way in.
                let diff = wrap_angle(want.to_angle() - brain.facing);
                let turn = CRAWLER_TURN * dt;
                brain.facing = wrap_angle(brain.facing + diff.clamp(-turn, turn));
                Vec2::from_angle(brain.facing) * max * speed * diff.cos().max(0.2)
            }
            MonsterKind::Raider => {
                raider_think(
                    &mut brain,
                    at,
                    &alive,
                    &arena,
                    nav.as_deref(),
                    &mut rng,
                    s16,
                );
                brain.move_dir * RAIDER_SPEED
            }
            MonsterKind::Drone => match sees(DRONE_RANGE + 100.0) {
                Some((_, p, d)) => {
                    let to = (p - at).normalize_or_zero();
                    let dir = if d > 300.0 {
                        to
                    } else if d < 200.0 {
                        -to
                    } else {
                        to.perp() * brain.heading.y.signum()
                    };
                    dir * DRONE_SPEED * speed
                }
                None => wander(&mut brain, &mut rng, s16) * DRONE_SPEED * speed,
            },
            MonsterKind::SporePod | MonsterKind::Behemoth => continue,
        };

        let radius = kind.radius();
        let want = at + velocity * dt;
        let next = if kind.flies() {
            // Flies over walls and props, but stays near home and never over a roof.
            let leashed = brain.home + (want - brain.home).clamp_length_max(DRONE_LEASH);
            let clamped = leashed.clamp(-ARENA_HALF + radius, ARENA_HALF - radius);
            if over_building(&arena, clamped, radius) {
                brain.turn_in = 0;
                at
            } else {
                if clamped.distance_squared(want) > 1.0 {
                    brain.turn_in = 0;
                }
                clamped
            }
        } else {
            let next = arena.resolve_with(want, radius, &obstacles.0);
            if next.distance_squared(want) > 1.0 {
                // Bumped into something: pick a new direction next time.
                brain.turn_in = 0;
            }
            next
        };
        if next != at {
            pos.0 = next;
        }
        let facing = match kind {
            MonsterKind::Crawler => brain.facing,
            MonsterKind::Raider => brain.aim.to_angle(),
            _ if velocity.length_squared() > 1.0 => velocity.to_angle(),
            _ => state.facing,
        };
        brain.facing = facing;
        let new = MonsterState { facing, ..*state };
        set_state(&mut state, new);
    }
}

/// Raiders hold their block: fight whoever comes close or shoots them, otherwise patrol.
/// Gun-play is the bots' (aim error, engage range, strafing, reaction time).
fn raider_think(
    brain: &mut Brain,
    at: Vec2,
    alive: &[Prey],
    arena: &Arena,
    nav: Option<&Nav>,
    rng: &mut impl Rng,
    step: u16,
) {
    brain.provoked = brain.provoked.saturating_sub(step);
    if brain.think_in > 0 {
        brain.think_in = brain.think_in.saturating_sub(step);
        return;
    }
    brain.think_in = RAIDER_THINK;
    let turf = brain.turf.inflate(RAIDER_TURF);
    let provoked = brain.provoked > 0;
    let enemy = alive
        .iter()
        .map(|(e, p, c)| (*e, *p, p.distance(at), *c))
        .filter(|(_, p, d, c)| {
            noticed(*c, *d)
                && *d < RAIDER_SIGHT
                && (provoked || turf.contains(*p))
                && arena.line_of_sight(at, *p)
        })
        .map(|(e, p, d, _)| (e, p, d))
        .min_by(|a, b| a.2.total_cmp(&b.2));
    let previous = brain.target;
    brain.target = enemy.map(|(e, ..)| e);
    if brain.target != previous {
        brain.locked_for = 0;
    }
    brain.locked_for = brain.locked_for.saturating_add(1);
    let route = |to: Vec2| match nav {
        Some(nav) => next_waypoint(&nav.0, arena, at, to, PLAYER_RADIUS) - at,
        None => to - at,
    };
    match enemy {
        Some((_, enemy_at, dist)) => {
            let to = (enemy_at - at).normalize_or(Vec2::X);
            brain.aim = aim_toward(to, brain.aim_error, rng);
            let range = engage_range(brain.weapon);
            if rng.random_bool(0.03) {
                brain.heading = -brain.heading;
            }
            let strafe = to.perp() * brain.heading.x.signum();
            let dir = if dist > range * 0.85 {
                route(enemy_at).normalize_or_zero() + strafe * 0.3
            } else if dist < range * 0.35 {
                -to + strafe
            } else {
                strafe
            };
            brain.move_dir = dir.normalize_or_zero();
            brain.fire = brain.locked_for > 3 && dist < range;
        }
        None => {
            brain.fire = false;
            if brain.goal.distance(at) < 40.0 || rng.random_bool(0.02) {
                let r = brain.turf.inflate(-MARGIN - 40.0);
                for _ in 0..6 {
                    let goal = Vec2::new(
                        rng.random_range(r.min.x..r.max.x),
                        rng.random_range(r.min.y..r.max.y),
                    );
                    if arena.resolve_circle(goal, PLAYER_RADIUS) == goal {
                        brain.goal = goal;
                        break;
                    }
                }
            }
            brain.move_dir = route(brain.goal).normalize_or_zero() * 0.6;
            brain.aim = brain.move_dir.normalize_or(brain.aim);
        }
    }
}

// ---------------------------------------------------------------- attacks

#[allow(clippy::type_complexity)]
fn attack(
    mut commands: Commands,
    state: Single<&MatchState>,
    arena: Res<Arena>,
    mut monsters: Query<
        (
            Entity,
            &Monster,
            &Pos,
            &mut Brain,
            &mut MonsterState,
            &mut Vitals,
        ),
        Without<Player>,
    >,
    players: Query<(Entity, &Pos, &Vitals), With<Player>>,
    mut damage: ResMut<DamageQueue>,
    mut blasts: ResMut<GhoulBlasts>,
) {
    if state.phase == Phase::Intermission {
        return;
    }
    let alive = prey(players.iter());
    let mut rng = rand::rng();
    for (entity, monster, pos, mut brain, mut mstate, mut vitals) in &mut monsters {
        let kind = monster.kind;
        if !vitals.alive || kind == MonsterKind::Behemoth {
            continue;
        }
        brain.attack_cd = brain.attack_cd.saturating_sub(1);
        if brain.far {
            continue;
        }
        let mult = brain.damage_mult;
        let at = pos.0;
        let reach = kind.radius() + PLAYER_RADIUS + 4.0;
        let bite = |amount: f32, damage: &mut DamageQueue, cause| {
            let victim = alive.iter().find(|(_, p, _)| p.distance(at) <= reach);
            if let Some((target, ..)) = victim {
                damage.0.push(Damage {
                    target: *target,
                    attacker: Some(entity),
                    amount: amount * mult,
                    armor_mult: 1.0,
                    weapon: None,
                    emp: false,
                    cause: Some(cause),
                });
            }
            victim.is_some()
        };
        if kind == MonsterKind::Ghoul && mstate.action == MonsterAction::Arming {
            let fuse = mstate.fuse.saturating_sub(1);
            if fuse == 0 {
                blasts.0.push(PendingBlast {
                    pos: at,
                    attacker: Some(entity),
                    source: entity,
                    damage: GHOUL_BLAST * mult,
                });
                vitals.alive = false;
                commands.entity(entity).insert(Dying);
            }
            let new = MonsterState { fuse, ..*mstate };
            set_state(&mut mstate, new);
            continue;
        }
        if brain.attack_cd > 0 {
            continue;
        }
        let sees = |range: f32| {
            nearest(&alive, at).filter(|(_, p, d)| *d < range && arena.line_of_sight(at, *p))
        };
        let muzzle = |dir: Vec2| at + dir * (kind.radius() + 6.0);
        match kind {
            MonsterKind::Roach => {
                if bite(ROACH_BITE, &mut damage, "撕咬") {
                    brain.attack_cd = ROACH_BITE_CD;
                }
            }
            MonsterKind::Dog => {
                if bite(DOG_BITE, &mut damage, "撕咬") {
                    brain.attack_cd = DOG_BITE_CD;
                }
            }
            MonsterKind::Crawler => {
                if bite(CRAWLER_HIT, &mut damage, "钳击") {
                    brain.attack_cd = CRAWLER_HIT_CD;
                }
            }
            MonsterKind::Ghoul => {
                let close = GHOUL_ARM_RANGE + kind.radius();
                if alive.iter().any(|(_, p, _)| p.distance(at) <= close) {
                    let new = MonsterState {
                        action: MonsterAction::Arming,
                        fuse: GHOUL_FUSE,
                        ..*mstate
                    };
                    set_state(&mut mstate, new);
                }
            }
            MonsterKind::SporePod => {
                if sees(POD_WAKE).is_none() {
                    continue;
                }
                brain.attack_cd = POD_VOLLEY_EVERY;
                brain.spiral += 0.23;
                let step = TAU / POD_GLOBS as f32;
                for i in 0..POD_GLOBS {
                    let dir = Vec2::from_angle(brain.spiral + i as f32 * step);
                    let flight = Flight::fauna(entity, SPORE_TTL, SPORE_DAMAGE * mult, "孢子");
                    let kind = ProjectileKind::Spore;
                    shoot(&mut commands, kind, muzzle(dir), dir, SPORE_SPEED, flight);
                }
            }
            MonsterKind::Spitter => {
                if let Some((_, p, _)) = sees(SPITTER_RANGE) {
                    brain.attack_cd = SPITTER_CD;
                    let dir = (p - at).normalize_or(Vec2::X);
                    let flight = Flight::fauna(entity, ACID_TTL, ACID_DAMAGE * mult, "酸液");
                    let kind = ProjectileKind::Acid;
                    shoot(&mut commands, kind, muzzle(dir), dir, ACID_SPEED, flight);
                }
            }
            MonsterKind::Drone => {
                if let Some((_, p, _)) = sees(DRONE_RANGE) {
                    brain.attack_cd = DRONE_CD;
                    let dir = (p - at).normalize_or(Vec2::X);
                    let flight = Flight::fauna(entity, BOLT_TTL, BOLT_DAMAGE * mult, "电弧");
                    let kind = ProjectileKind::Bolt;
                    shoot(&mut commands, kind, muzzle(dir), dir, BOLT_SPEED, flight);
                }
            }
            MonsterKind::Raider => {
                if !brain.fire {
                    continue;
                }
                let weapon = brain.weapon;
                let stats = weapon.stats();
                // Slower trigger finger than a player, or a rifle squad would shred everyone.
                brain.attack_cd = (stats.cooldown * 2).max(14);
                let base = brain.aim.to_angle();
                for i in 0..stats.pellets {
                    let fan = if stats.pellets > 1 {
                        (i as f32 / (stats.pellets - 1) as f32 - 0.5) * stats.spread
                    } else if stats.spread > 0.0 {
                        rng.random_range(-stats.spread..stats.spread)
                    } else {
                        0.0
                    };
                    let dir = Vec2::from_angle(base + fan);
                    let flight = Flight::raider_shot(entity, weapon);
                    let kind = ProjectileKind::Shot(weapon);
                    let speed = stats.range_or_speed;
                    shoot(&mut commands, kind, muzzle(dir), dir, speed, flight);
                }
            }
            MonsterKind::Behemoth => {}
        }
    }
}

/// Everyone caught in a ghoul blast, once each, with the damage they take.
pub fn blast_hits(
    center: Vec2,
    damage: f32,
    source: Entity,
    targets: impl Iterator<Item = (Entity, Vec2)>,
    line_of_sight: impl Fn(Vec2, Vec2) -> bool,
) -> Vec<(Entity, f32)> {
    let radius = GHOUL_BLAST_RADIUS;
    let mut seen = HashSet::new();
    targets
        .filter(|(e, p)| *e != source && p.distance(center) <= radius && line_of_sight(center, *p))
        .filter(|(e, _)| seen.insert(*e))
        .map(|(e, p)| {
            let falloff = 1.0 - GHOUL_FALLOFF * (p.distance(center) / radius);
            (e, damage * falloff)
        })
        .collect()
}

/// Ghoul blasts hurt players and monsters alike.
#[allow(clippy::type_complexity)]
fn ghoul_blasts(
    arena: Res<Arena>,
    mut blasts: ResMut<GhoulBlasts>,
    targets: Query<(Entity, &Pos, &Vitals), Or<(With<Player>, With<Monster>)>>,
    mut damage: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
) {
    for blast in blasts.0.drain(..) {
        let alive = targets
            .iter()
            .filter(|(_, _, v)| v.alive)
            .map(|(e, p, _)| (e, p.0));
        let hits = blast_hits(blast.pos, blast.damage, blast.source, alive, |a, b| {
            arena.line_of_sight(a, b)
        });
        for (target, amount) in hits {
            damage.0.push(Damage {
                target,
                attacker: blast.attacker,
                amount,
                armor_mult: 1.0,
                weapon: None,
                emp: false,
                cause: Some("尸鬼自爆"),
            });
        }
        outbox.fast.push(GameEvent::Boom {
            pos: blast.pos,
            radius: GHOUL_BLAST_RADIUS,
            fire: true,
        });
    }
}

fn puddles(
    mut commands: Commands,
    tick: Res<SimTick>,
    mut puddles: Query<(Entity, &Pos, &Puddle, &mut PuddleLife)>,
    players: Query<(Entity, &Pos, &Vitals), With<Player>>,
    mut damage: ResMut<DamageQueue>,
) {
    let bite = tick.0.is_multiple_of(PUDDLE_EVERY);
    for (entity, pos, puddle, mut life) in &mut puddles {
        life.ttl = life.ttl.saturating_sub(1);
        if life.ttl == 0 {
            commands.entity(entity).despawn();
            continue;
        }
        if !bite {
            continue;
        }
        for (target, p, v) in &players {
            if v.alive && p.0.distance(pos.0) <= puddle.radius + PLAYER_RADIUS * 0.5 {
                damage.0.push(Damage {
                    target,
                    attacker: life.owner,
                    amount: life.dps * PUDDLE_EVERY as f32 / TICKS_PER_SEC as f32,
                    armor_mult: 1.0,
                    weapon: None,
                    emp: false,
                    cause: Some(life.cause),
                });
            }
        }
    }
}

// ---------------------------------------------------------------- damage & death

/// Damage multiplier for a hit on a crawler facing `facing` (radians) from `toward_attacker`.
pub fn crawler_mult(facing: f32, toward_attacker: Vec2) -> f32 {
    let dir = toward_attacker.normalize_or_zero();
    if dir != Vec2::ZERO && dir.dot(Vec2::from_angle(facing)) >= CRAWLER_SHELL_COS {
        CRAWLER_SHELL_MULT
    } else {
        1.0
    }
}

fn drops(monster: Monster, brain: &Brain, rng: &mut impl Rng) -> Vec<Item> {
    let tier = spec(monster.kind).loot_tier;
    if monster.elite {
        return vec![roll_loot((tier + 1).min(4), rng), Item::Medkit];
    }
    match monster.kind {
        MonsterKind::Roach if rng.random_bool(0.35) => vec![roll_loot(1, rng)],
        MonsterKind::Roach if rng.random_bool(0.3) => vec![Item::Medkit],
        MonsterKind::Dog if rng.random_bool(0.25) => vec![Item::Medkit],
        MonsterKind::SporePod | MonsterKind::Spitter | MonsterKind::Crawler => {
            vec![roll_loot(tier, rng)]
        }
        MonsterKind::Drone if rng.random_bool(0.4) => {
            vec![if rng.random_bool(0.5) {
                Item::Emp
            } else {
                Item::Medkit
            }]
        }
        MonsterKind::Raider => {
            // Whatever it carried, like any fallen fighter.
            let mut items = vec![Item::Weapon {
                kind: brain.weapon,
                ammo: brain.weapon.stats().max_ammo / 2,
            }];
            if rng.random_bool(0.35) {
                items.push(Item::Armor {
                    kind: ArmorKind::Kevlar,
                    points: 40.0,
                });
            }
            items
        }
        _ => Vec::new(),
    }
}

/// Monsters take damage here; everything aimed at a combatant is left for `resolve_damage`.
#[allow(clippy::type_complexity)]
fn resolve_monster_damage(
    mut commands: Commands,
    mut xp: ResMut<XpQueue>,
    mut dealt: ResMut<Dealt>,
    mut queue: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
    mut monsters: Query<(&Monster, &Pos, &mut Vitals, &mut Brain, &MonsterState)>,
    mut players: Query<(Entity, &Player, &mut Stats)>,
    positions: Query<&Pos>,
    mut blasts: ResMut<GhoulBlasts>,
    mut boss: ResMut<BossState>,
) {
    let mut rng = rand::rng();
    queue.0.retain(|dmg| {
        let Ok((monster, pos, mut vitals, mut brain, mstate)) = monsters.get_mut(dmg.target) else {
            return true;
        };
        if !vitals.alive {
            return false;
        }
        let player_attacker = dmg.attacker.filter(|a| players.contains(*a));
        let mut amount = dmg.amount;
        match monster.kind {
            MonsterKind::Crawler => {
                if let Some(from) = dmg.attacker.and_then(|a| positions.get(a).ok()) {
                    amount *= crawler_mult(mstate.facing, from.0 - pos.0);
                }
            }
            MonsterKind::Raider => brain.provoked = RAIDER_PROVOKED,
            MonsterKind::Behemoth => {
                if let Some(attacker) = player_attacker {
                    let mine = players.get(attacker).map_or(0, |(.., s)| s.score);
                    let best_other = players
                        .iter()
                        .filter(|(e, ..)| *e != attacker)
                        .map(|(.., s)| s.score)
                        .max()
                        .unwrap_or(0);
                    amount *= boss::damage_mult(mine, best_other);
                }
            }
            _ => {}
        }
        let amount = amount.min(vitals.hp);
        vitals.hp -= amount;
        if monster.kind == MonsterKind::Behemoth
            && let Some(attacker) = player_attacker
        {
            boss.credit(attacker, amount);
        }
        outbox.fast.push(GameEvent::Hit {
            pos: pos.0,
            amount,
            armor: false,
            attacker: dmg
                .attacker
                .and_then(|a| players.get(a).ok())
                .map_or(0, |(_, p, _)| p.client_id),
        });
        if let Some(by) = dmg.attacker {
            let melee = dmg.cause.is_none() && dmg.weapon.is_some_and(|w| w.stats().melee);
            dealt.0.push(Hit {
                attacker: by,
                amount,
                melee,
            });
        }
        if vitals.hp > 0.0 {
            return false;
        }
        vitals.alive = false;
        commands.entity(dmg.target).despawn();
        if monster.kind == MonsterKind::Behemoth {
            // Rewards are handed out by `boss::reward`.
            boss.slain(pos.0);
            return false;
        }
        if monster.kind == MonsterKind::Ghoul {
            blasts.0.push(PendingBlast {
                pos: pos.0,
                attacker: dmg.attacker,
                source: dmg.target,
                damage: GHOUL_BLAST * brain.damage_mult,
            });
        }
        let items = drops(*monster, &brain, &mut rng);
        let n = items.len();
        for (i, item) in items.into_iter().enumerate() {
            let offset = if n > 1 {
                Vec2::from_angle(i as f32 * TAU / n as f32) * 16.0
            } else {
                Vec2::ZERO
            };
            spawn_pickup(&mut commands, item, pos.0 + offset, Some(LOOT_TTL));
        }
        if let Some(by) = dmg.attacker {
            // Non-players (raiders) are skipped by `grant_xp`.
            xp.0.push(XpAward {
                to: by,
                base: monster.xp(),
                pos: pos.0,
                loot: false,
            });
        }
        if let Some(killer) = player_attacker
            && let Ok((_, player, mut stats)) = players.get_mut(killer)
        {
            let points = monster.score();
            if points > 0 {
                stats.score += points;
                outbox.announce(format!(
                    "{} 击杀了{}，+{points} 分",
                    player.name,
                    monster.display_name()
                ));
            }
        }
        false
    });
}

fn reap(mut commands: Commands, dying: Query<Entity, With<Dying>>) {
    for entity in &dying {
        commands.entity(entity).despawn();
    }
}

fn reset_monsters(
    reset: Res<ResetRequested>,
    mut spawner: ResMut<Spawner>,
    mut blasts: ResMut<GhoulBlasts>,
) {
    if reset.0 {
        *spawner = Spawner::default();
        blasts.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evoclash_game::map::ArenaMap;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn caps_hold_while_filling() {
        let mut c = Census::default();
        loop {
            let mut spawned = false;
            for kind in MonsterKind::ALL {
                if kind == MonsterKind::Behemoth || !c.wants(kind) {
                    continue;
                }
                for slot in 0..spec(kind).group {
                    c.add(kind, false, slot == 0);
                }
                spawned = true;
            }
            if !spawned {
                break;
            }
        }
        assert!(c.total <= MAX_LIVE, "{c:?}");
        for kind in MonsterKind::ALL {
            let s = spec(kind);
            let i = kind_index(kind);
            assert!(c.groups[i] <= s.cap, "{kind:?}");
            assert!(c.live[i] <= s.cap * s.group, "{kind:?}");
        }
        let live = |k| c.live[kind_index(k)];
        assert_eq!(live(MonsterKind::Roach), 10);
        assert_eq!(live(MonsterKind::SporePod), 5);
        assert_eq!(live(MonsterKind::Dog), 12);
        assert_eq!(live(MonsterKind::Raider), 4);
        assert_eq!(live(MonsterKind::Crawler), 3);
        assert_eq!(c.groups[kind_index(MonsterKind::Raider)], 2);
        // Room left for the boss and its summons.
        assert!(c.total + 4 <= MAX_LIVE);
    }

    #[test]
    fn census_counts_packs_once() {
        let c = census(
            [
                (MonsterKind::Dog, false, 1),
                (MonsterKind::Dog, false, 1),
                (MonsterKind::Dog, true, 2),
                (MonsterKind::Roach, false, 0),
            ]
            .into_iter(),
        );
        assert_eq!(c.groups[kind_index(MonsterKind::Dog)], 2);
        assert_eq!(c.live[kind_index(MonsterKind::Dog)], 3);
        assert_eq!((c.elites, c.total), (1, 4));
    }

    #[test]
    fn spawn_spots_are_clear() {
        let mut rng = StdRng::seed_from_u64(7);
        let mut found = 0;
        let seeds = 60;
        for seed in 0..seeds {
            let arena = Arena(ArenaMap::generate(seed));
            let players: Vec<Vec2> = arena.spawn_points.iter().step_by(3).copied().collect();
            let obstacles: Vec<(Vec2, f32)> = arena
                .props
                .iter()
                .map(|p| (p.pos, p.kind.radius()))
                .collect();
            for kind in MonsterKind::ALL {
                if kind == MonsterKind::Behemoth {
                    continue;
                }
                for _ in 0..4 {
                    let Some((p, block)) = pick_spot(&arena, kind, &obstacles, &players, &mut rng)
                    else {
                        continue;
                    };
                    found += 1;
                    let r = kind.radius();
                    assert_eq!(
                        arena.resolve_circle(p, r),
                        p,
                        "seed {seed} {kind:?} in a wall"
                    );
                    assert!(p.distance(VAULT_CENTER) > SPAWN_CLEAR_OF_VAULT);
                    assert!(block.rect.contains(p) && block_suits(kind, &block));
                    assert!(
                        players
                            .iter()
                            .all(|q| q.distance(p) > SPAWN_CLEAR_OF_PLAYERS)
                    );
                    assert!(obstacles.iter().all(|(c, cr)| c.distance(p) >= cr + r));
                    if kind.flies() {
                        assert!(!over_building(&arena, p, r));
                    }
                }
            }
        }
        assert!(found > seeds * 8 * 3, "too few spots found: {found}");
    }

    #[test]
    fn crawler_shell_covers_the_front_cone() {
        assert_eq!(crawler_mult(0.0, Vec2::X), CRAWLER_SHELL_MULT);
        assert_eq!(crawler_mult(0.0, Vec2::from_angle(1.2)), CRAWLER_SHELL_MULT);
        assert_eq!(
            crawler_mult(0.0, Vec2::from_angle(-1.2)),
            CRAWLER_SHELL_MULT
        );
        assert_eq!(crawler_mult(0.0, Vec2::from_angle(1.3)), 1.0);
        assert_eq!(crawler_mult(0.0, Vec2::Y), 1.0);
        assert_eq!(crawler_mult(0.0, -Vec2::X), 1.0);
        assert_eq!(crawler_mult(PI, -Vec2::X), CRAWLER_SHELL_MULT);
    }

    #[test]
    fn ghoul_blast_hits_each_target_once() {
        let e = |i| Entity::from_raw_u32(i).unwrap();
        let (ghoul, a, b, far) = (e(1), e(2), e(3), e(4));
        let targets = [
            (ghoul, Vec2::ZERO),
            (a, Vec2::new(10.0, 0.0)),
            (a, Vec2::new(10.0, 0.0)),
            (b, Vec2::new(0.0, GHOUL_BLAST_RADIUS)),
            (far, Vec2::new(500.0, 0.0)),
        ];
        let hits = blast_hits(
            Vec2::ZERO,
            GHOUL_BLAST,
            ghoul,
            targets.into_iter(),
            |_, _| true,
        );
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|(t, d)| *t == a && *d > 43.5));
        let edge = hits.iter().find(|(t, _)| *t == b).unwrap().1;
        assert!((edge - GHOUL_BLAST * 0.75).abs() < 1e-3);
    }
}
