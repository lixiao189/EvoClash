//! Authoritative simulation: inputs, movement, damage, respawns, loot and the match clock.

use std::collections::VecDeque;

use bevy::prelude::*;
use evoclash_game::items::{ArmorKind, Item, WeaponKind, roll_loot};
use evoclash_game::map::VAULT_CENTER;
use evoclash_game::protocol::*;
use evoclash_game::sim::{self, Arena, MAX_HP};
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use crate::config::ServerConfig;
use crate::connections::Human;
use crate::ops::OpsState;

pub const TICKS_PER_SEC: u32 = 64;
const WARMUP_TICKS: u32 = 4 * TICKS_PER_SEC;
const MATCH_TICKS: u32 = 4 * 60 * TICKS_PER_SEC;
const INTERMISSION_TICKS: u32 = 8 * TICKS_PER_SEC;
const FIRST_VAULT_TICKS: u32 = 40 * TICKS_PER_SEC;
const VAULT_INTERVAL_TICKS: u32 = 60 * TICKS_PER_SEC;
pub const SCORE_TO_WIN: u32 = 15;
const RESPAWN_TICKS: u16 = (2.5 * TICKS_PER_SEC as f32) as u16;
const DROP_TTL_TICKS: u32 = 30 * TICKS_PER_SEC;
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
        app.init_resource::<SimTick>()
            .init_resource::<Outbox>()
            .init_resource::<DamageQueue>()
            .init_resource::<LootState>()
            .init_resource::<ResetRequested>();
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
                (movement, record_history, tick_cooldowns)
                    .chain()
                    .in_set(SimSet::Simulate),
                (resolve_damage, respawn, expire_drops)
                    .chain()
                    .in_set(SimSet::Resolve),
                (loot_spawner, match_flow, reset_match, flush_outbox)
                    .chain()
                    .in_set(SimSet::Flow),
            ),
        );
    }
}

// ---------------------------------------------------------------- resources & components

#[derive(Resource, Default)]
pub struct SimTick(pub u32);

/// Events to broadcast this tick, flushed once at the end of the tick.
#[derive(Resource, Default)]
pub struct Outbox {
    pub reliable: Vec<GameEvent>,
    pub fast: Vec<GameEvent>,
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
    pub weapon: WeaponKind,
    /// EMP blast: strips armor and slows instead of (mostly) hurting.
    pub emp: bool,
}

#[derive(Resource, Default)]
pub struct DamageQueue(pub Vec<Damage>);

#[derive(Resource, Default)]
pub struct ResetRequested(pub bool);

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
struct LootState {
    spots: Vec<SpotState>,
}

// ---------------------------------------------------------------- spawning helpers

/// Spawn a fresh combatant. `peer` is set for humans so they predict themselves.
pub fn spawn_combatant(commands: &mut Commands, player: Player, peer: Option<PeerId>) -> Entity {
    let interpolate = match peer {
        Some(peer) => NetworkTarget::AllExceptSingle(peer),
        None => NetworkTarget::All,
    };
    let mut entity = commands.spawn((
        Name::new(player.name.clone()),
        player,
        Pos(Vec2::ZERO),
        Aim(Vec2::X),
        Dash::default(),
        // Spawned dead with a short timer so `respawn` picks a safe spot.
        Vitals {
            alive: false,
            respawn_in: 1,
            ..default()
        },
        Loadout::default(),
        Stats::default(),
        (
            CurrentInput::default(),
            Cooldowns::default(),
            PosHistory::default(),
            Rewind::default(),
        ),
        Replicate::to_clients(NetworkTarget::All),
        InterpolationTarget::to_clients(interpolate),
    ));
    if let Some(peer) = peer {
        entity.insert(PredictionTarget::to_clients(NetworkTarget::Single(peer)));
    }
    entity.id()
}

pub fn spawn_pickup(commands: &mut Commands, item: Item, pos: Vec2, ttl: Option<u32>) -> Entity {
    let mut entity = commands.spawn((
        Name::new("Pickup"),
        Pickup { item },
        Pos(pos),
        Replicate::to_clients(NetworkTarget::All),
    ));
    if let Some(ttl) = ttl {
        entity.insert(Expires(ttl));
    }
    entity.id()
}

fn spawn_match_state(mut commands: Commands) {
    commands.spawn((
        Name::new("Match"),
        MatchState {
            phase: Phase::Warmup,
            ticks_left: WARMUP_TICKS,
            vault_in: FIRST_VAULT_TICKS,
            winner: None,
        },
        Replicate::to_clients(NetworkTarget::All),
    ));
}

// ---------------------------------------------------------------- input & movement

fn advance_tick(mut tick: ResMut<SimTick>) {
    tick.0 = tick.0.wrapping_add(1);
}

fn read_human_inputs(
    mut humans: Query<
        (
            &mut CurrentInput,
            &mut Rewind,
            &Human,
            Option<&ActionState<PlayerInput>>,
            Option<&ControlledBy>,
        ),
        With<Human>,
    >,
    delays: Query<&InterpolationDelay>,
) {
    for (mut current, mut rewind, human, action, control) in &mut humans {
        if !human.connected() {
            // Disconnected bodies stand still instead of replaying the last held keys.
            current.0 = PlayerInput::default();
            continue;
        }
        if let Some(action) = action {
            current.0 = action.0.clone();
        }
        rewind.0 = control
            .and_then(|c| delays.get(c.owner).ok())
            .map(|d| (d.delay.tick_diff() + 1).min(MAX_REWIND_TICKS))
            .unwrap_or(0);
    }
}

fn movement(
    arena: Res<Arena>,
    mut players: Query<(&mut Pos, &mut Dash, &mut Aim, &CurrentInput, &Vitals)>,
) {
    for (pos, dash, mut aim, input, vitals) in &mut players {
        sim::step_movement(pos, dash, &input.0, vitals, &arena);
        let dir = input.0.aim.normalize_or_zero();
        if vitals.alive && dir != Vec2::ZERO && dir.distance_squared(aim.0) > 1e-4 {
            aim.0 = dir;
        }
    }
}

fn record_history(tick: Res<SimTick>, mut players: Query<(&Pos, &mut PosHistory)>) {
    for (pos, mut history) in &mut players {
        history.0.push_back((tick.0, pos.0));
        if history.0.len() > HISTORY_LEN {
            history.0.pop_front();
        }
    }
}

fn tick_cooldowns(mut players: Query<(&mut Cooldowns, &mut Vitals)>) {
    for (mut cd, mut vitals) in &mut players {
        cd.fire = cd.fire.saturating_sub(1);
        cd.interact = cd.interact.saturating_sub(1);
        cd.throw = cd.throw.saturating_sub(1);
        if vitals.emp_slow > 0 {
            vitals.emp_slow -= 1;
        }
    }
}

// ---------------------------------------------------------------- damage, death, respawn

/// Apply damage through armor. Returns (hp lost, armor lost).
fn absorb(vitals: &mut Vitals, amount: f32, armor_mult: f32) -> (f32, f32) {
    let armor_hit = (amount * armor_mult).min(vitals.armor_points);
    vitals.armor_points -= armor_hit;
    let absorbed = armor_hit / armor_mult;
    let hp_hit = (amount - absorbed).max(0.0);
    vitals.hp -= hp_hit;
    if vitals.armor_points <= 0.0 {
        vitals.armor_points = 0.0;
        vitals.armor = ArmorKind::None;
    }
    (hp_hit, armor_hit)
}

fn resolve_damage(
    mut commands: Commands,
    mut queue: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
    mut players: Query<(&Player, &Pos, &mut Vitals, &mut Loadout, &mut Stats)>,
    state: Single<&MatchState>,
) {
    if state.phase == Phase::Intermission {
        queue.0.clear();
        return;
    }
    let mut deaths: Vec<(Entity, Option<Entity>, WeaponKind)> = Vec::new();
    for dmg in queue.0.drain(..) {
        let attacker_id = dmg
            .attacker
            .and_then(|a| players.get(a).ok())
            .map(|(p, ..)| p.client_id)
            .unwrap_or(0);
        let Ok((_, pos, mut vitals, mut loadout, _)) = players.get_mut(dmg.target) else {
            continue;
        };
        if !vitals.alive {
            continue;
        }
        if dmg.emp {
            vitals.armor_points = match vitals.armor {
                ArmorKind::PowerArmor => 0.0,
                _ => vitals.armor_points * 0.5,
            };
            if vitals.armor_points < 1.0 {
                vitals.armor_points = 0.0;
                vitals.armor = ArmorKind::None;
            }
            vitals.emp_slow = 3 * TICKS_PER_SEC as u16;
            // Energy weapons short out too.
            if matches!(loadout.weapon, WeaponKind::Laser | WeaponKind::Railgun) {
                loadout.ammo = loadout.ammo / 2;
            }
        }
        let (hp_hit, armor_hit) = absorb(&mut vitals, dmg.amount, dmg.armor_mult);
        outbox.fast.push(GameEvent::Hit {
            pos: pos.0,
            amount: hp_hit + armor_hit,
            armor: armor_hit > hp_hit,
            attacker: attacker_id,
        });
        if vitals.hp <= 0.0 {
            vitals.hp = 0.0;
            vitals.alive = false;
            vitals.respawn_in = RESPAWN_TICKS;
            deaths.push((dmg.target, dmg.attacker, dmg.weapon));
        }
    }

    for (victim, attacker, weapon) in deaths {
        let Ok((player, pos, mut vitals, mut loadout, mut stats)) = players.get_mut(victim) else {
            continue;
        };
        let victim_name = player.name.clone();
        let had_bounty = stats.has_bounty();
        stats.deaths += 1;
        stats.streak = 0;

        // Everything you carried is up for grabs: killing the strong is how the weak catch up.
        let drop_at = pos.0;
        if loadout.weapon != WeaponKind::Stick && loadout.ammo > 0 {
            let item = Item::Weapon {
                kind: loadout.weapon,
                ammo: loadout.ammo,
            };
            spawn_pickup(&mut commands, item, drop_at + Vec2::new(-14.0, 0.0), Some(DROP_TTL_TICKS));
        }
        if vitals.armor != ArmorKind::None && vitals.armor_points > 0.0 {
            let item = Item::Armor {
                kind: vitals.armor,
                points: vitals.armor_points,
            };
            spawn_pickup(&mut commands, item, drop_at + Vec2::new(14.0, 0.0), Some(DROP_TTL_TICKS));
        }
        if loadout.emps > 0 {
            spawn_pickup(&mut commands, Item::Emp, drop_at + Vec2::new(0.0, 14.0), Some(DROP_TTL_TICKS));
        }
        *loadout = Loadout::default();
        vitals.armor = ArmorKind::None;
        vitals.armor_points = 0.0;

        let killer = attacker.filter(|a| *a != victim);
        let killer_name = match killer.and_then(|k| players.get_mut(k).ok()) {
            Some((killer_player, _, _, _, mut killer_stats)) => {
                killer_stats.kills += 1;
                killer_stats.streak += 1;
                killer_stats.score += if had_bounty { 3 } else { 1 };
                if killer_stats.streak == sim::BOUNTY_STREAK {
                    outbox.announce(format!("{} 连杀 {} 人，已被悬赏！", killer_player.name, killer_stats.streak));
                }
                killer_player.name.clone()
            }
            None => "废土".to_string(),
        };
        info!("{killer_name} killed {victim_name} with {weapon:?}");
        outbox.reliable.push(GameEvent::Kill {
            killer: killer_name,
            victim: victim_name,
            weapon,
            bounty: had_bounty,
        });
    }
}

fn respawn(
    arena: Res<Arena>,
    mut players: Query<(Entity, &mut Pos, &mut Vitals, &mut Dash, &mut PosHistory)>,
) {
    let mut alive: Vec<(Entity, Vec2)> = players
        .iter()
        .filter(|(_, _, v, ..)| v.alive)
        .map(|(e, p, ..)| (e, p.0))
        .collect();
    for (entity, mut pos, mut vitals, mut dash, mut history) in &mut players {
        if vitals.alive {
            continue;
        }
        vitals.respawn_in = vitals.respawn_in.saturating_sub(1);
        if vitals.respawn_in > 0 {
            continue;
        }
        // Spawn as far as possible from everyone else.
        let spot = arena
            .spawn_points
            .iter()
            .copied()
            .max_by(|a, b| {
                let score = |p: &Vec2| {
                    alive
                        .iter()
                        .filter(|(e, _)| *e != entity)
                        .map(|(_, q)| p.distance(*q))
                        .fold(f32::MAX, f32::min)
                };
                score(a).total_cmp(&score(b))
            })
            .unwrap_or(Vec2::ZERO);
        pos.0 = spot;
        alive.push((entity, spot));
        history.0.clear();
        *dash = Dash::default();
        *vitals = Vitals {
            hp: MAX_HP,
            ..default()
        };
    }
}

fn expire_drops(mut commands: Commands, mut drops: Query<(Entity, &mut Expires)>) {
    for (entity, mut expires) in &mut drops {
        expires.0 = expires.0.saturating_sub(1);
        if expires.0 == 0 {
            commands.entity(entity).despawn();
        }
    }
}

// ---------------------------------------------------------------- loot & match flow

fn loot_spawner(
    mut commands: Commands,
    arena: Res<Arena>,
    mut loot: ResMut<LootState>,
    pickups: Query<(), With<Pickup>>,
) {
    if loot.spots.len() != arena.loot_spots.len() {
        loot.spots = arena.loot_spots.iter().map(|_| SpotState::default()).collect();
    }
    let mut rng = rand::rng();
    for (state, spot) in loot.spots.iter_mut().zip(&arena.loot_spots) {
        if let Some(entity) = state.entity {
            if pickups.contains(entity) {
                continue;
            }
            // Taken: higher tiers take longer to come back.
            state.entity = None;
            state.respawn_in = (10 + 6 * spot.tier as u32) * TICKS_PER_SEC;
        }
        if state.respawn_in > 0 {
            state.respawn_in -= 1;
            continue;
        }
        let item = roll_loot(spot.tier, &mut rng);
        state.entity = Some(spawn_pickup(&mut commands, item, spot.pos, None));
    }
}

fn match_flow(
    mut commands: Commands,
    mut state: Single<&mut MatchState>,
    players: Query<(&Player, &Stats)>,
    mut outbox: ResMut<Outbox>,
    mut reset: ResMut<ResetRequested>,
    config: Res<ServerConfig>,
    ops: Res<OpsState>,
    mut exit: MessageWriter<AppExit>,
) {
    state.ticks_left = state.ticks_left.saturating_sub(1);
    match state.phase {
        Phase::Warmup => {
            if players.iter().count() < 2 {
                state.ticks_left = WARMUP_TICKS;
            } else if state.ticks_left == 0 {
                state.phase = Phase::Playing;
                state.ticks_left = MATCH_TICKS;
                state.vault_in = FIRST_VAULT_TICKS;
                state.winner = None;
                reset.0 = true;
                info!("match started with {} combatants", players.iter().count());
                outbox.announce(format!("对局开始！先拿到 {SCORE_TO_WIN} 分者获胜"));
            }
        }
        Phase::Playing => {
            state.vault_in = state.vault_in.saturating_sub(1);
            if state.vault_in == 10 * TICKS_PER_SEC {
                outbox.announce("前文明地堡将在 10 秒后开启");
            }
            if state.vault_in == 0 {
                state.vault_in = VAULT_INTERVAL_TICKS;
                let mut rng = rand::rng();
                for offset in [Vec2::new(-45.0, 0.0), Vec2::new(45.0, 0.0)] {
                    let item = roll_loot(4, &mut rng);
                    spawn_pickup(&mut commands, item, VAULT_CENTER + offset, Some(DROP_TTL_TICKS * 2));
                }
                outbox.announce("地堡开启：尖端科技已出土！");
                outbox.fast.push(GameEvent::Boom {
                    pos: VAULT_CENTER,
                    radius: 220.0,
                });
            }
            let leader = players.iter().max_by_key(|(_, s)| s.score);
            let won = leader.is_some_and(|(_, s)| s.score >= SCORE_TO_WIN);
            if won || state.ticks_left == 0 {
                let winner = leader.map(|(p, _)| p.name.clone());
                outbox.announce(match &winner {
                    Some(name) => format!("{name} 赢得了本局！"),
                    None => "本局结束".to_string(),
                });
                info!("match over, winner {winner:?}");
                state.phase = Phase::Intermission;
                state.ticks_left = INTERMISSION_TICKS;
                state.winner = winner;
            }
        }
        Phase::Intermission => {
            if state.ticks_left == 0 {
                if config.single_match || ops.is_draining() {
                    info!("match finished, shutting down");
                    exit.write(AppExit::Success);
                    return;
                }
                state.phase = Phase::Warmup;
                state.ticks_left = WARMUP_TICKS;
                state.winner = None;
            }
        }
    }
}

/// Start every combatant over from a stick and clear the arena.
fn reset_match(
    mut commands: Commands,
    mut reset: ResMut<ResetRequested>,
    mut players: Query<(&mut Vitals, &mut Loadout, &mut Stats)>,
    clutter: Query<Entity, Or<(With<Pickup>, With<Projectile>)>>,
    mut loot: ResMut<LootState>,
) {
    if !std::mem::take(&mut reset.0) {
        return;
    }
    for (mut vitals, mut loadout, mut stats) in &mut players {
        *vitals = Vitals {
            alive: false,
            respawn_in: 1,
            ..default()
        };
        *loadout = Loadout::default();
        *stats = Stats::default();
    }
    for entity in &clutter {
        commands.entity(entity).despawn();
    }
    loot.spots.clear();
}

fn flush_outbox(
    mut outbox: ResMut<Outbox>,
    mut sender: ServerMultiMessageSender,
    server: Single<&Endpoint, With<Server>>,
) {
    let server = server.into_inner();
    for event in outbox.reliable.drain(..) {
        if let Err(e) = sender.send::<_, ReliableChannel>(&event, server, &NetworkTarget::All) {
            warn!("failed to send event: {e:?}");
        }
    }
    for event in outbox.fast.drain(..) {
        let _ = sender.send::<_, FastChannel>(&event, server, &NetworkTarget::All);
    }
}
