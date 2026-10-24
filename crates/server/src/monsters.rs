//! Mindless wasteland fauna. They give everyone — bots included — something to shoot that
//! isn't you, and they drop loot, so a fresh spawn has a way to gear up without a duel.

use bevy::prelude::*;
use evoclash_game::items::{Item, roll_loot};
use evoclash_game::map::{ARENA_HALF, VAULT_CENTER};
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, DT, Obstacles, PLAYER_RADIUS};
use lightyear::prelude::*;
use rand::Rng;

use crate::combat::{Flight, projectile_replication};
use crate::config::ServerConfig;
use crate::gameplay::{
    Damage, DamageQueue, Outbox, PosHistory, SimSet, TICKS_PER_SEC, resolve_damage, spawn_pickup,
};

const ROACHES: usize = 14;
const PODS: usize = 6;
/// Ticks between top-ups, so a cleared area stays quiet for a while.
const SPAWN_EVERY: u32 = 5 * TICKS_PER_SEC;

const ROACH_WANDER_SPEED: f32 = 120.0;
const ROACH_CHARGE_SPEED: f32 = 235.0;
const ROACH_NOTICE: f32 = 300.0;
const ROACH_BITE: f32 = 13.0;
const ROACH_BITE_COOLDOWN: u16 = 48;

/// Pods only spray while someone is close enough to care; saves bandwidth too.
const POD_WAKE: f32 = 680.0;
const POD_VOLLEY_EVERY: u16 = 90;
const POD_GLOBS: u32 = 10;
const SPORE_SPEED: f32 = 240.0;
const SPORE_TTL: u16 = 170;
const SPORE_DAMAGE: f32 = 9.0;
const LOOT_TTL: u32 = 30 * TICKS_PER_SEC;

/// Server-only monster state.
#[derive(Component)]
struct Brain {
    heading: Vec2,
    turn_in: u16,
    attack_cd: u16,
    /// Pods rotate each ring a little so the pattern spirals.
    spiral: f32,
}

pub struct MonstersPlugin;

impl Plugin for MonstersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                (populate, roam).chain().in_set(SimSet::Simulate),
                attack.in_set(SimSet::Combat),
                resolve_monster_damage
                    .in_set(SimSet::Resolve)
                    .before(resolve_damage),
            ),
        );
    }
}

fn populate(
    mut commands: Commands,
    config: Res<ServerConfig>,
    arena: Res<Arena>,
    state: Single<&MatchState>,
    monsters: Query<&Monster>,
    players: Query<(&Pos, &Vitals), With<Player>>,
    mut cooldown: Local<u32>,
) {
    if !config.monsters || state.phase == Phase::Intermission {
        return;
    }
    if *cooldown > 0 {
        *cooldown -= 1;
        return;
    }
    let count = |kind| monsters.iter().filter(|m| m.kind == kind).count();
    let kind = if count(MonsterKind::Roach) < ROACHES {
        MonsterKind::Roach
    } else if count(MonsterKind::SporePod) < PODS {
        MonsterKind::SporePod
    } else {
        return;
    };
    // During warmup fill the arena straight away; mid-match trickle them back in.
    *cooldown = if state.phase == Phase::Warmup {
        0
    } else {
        SPAWN_EVERY
    };
    let alive: Vec<Vec2> = players
        .iter()
        .filter(|(_, v)| v.alive)
        .map(|(p, _)| p.0)
        .collect();
    let mut rng = rand::rng();
    let radius = kind.radius();
    let bound = ARENA_HALF * 0.88;
    let spot = (0..40)
        .map(|_| {
            Vec2::new(
                rng.random_range(-bound.x..bound.x),
                rng.random_range(-bound.y..bound.y),
            )
        })
        .find(|p| {
            // Stay out of walls, out of the vault, and never pop up on top of someone.
            arena.resolve_circle(*p, radius + 12.0) == *p
                && p.distance(VAULT_CENTER) > 320.0
                && alive.iter().all(|q| q.distance(*p) > 480.0)
        });
    let Some(spot) = spot else {
        return;
    };
    commands.spawn((
        Name::new(kind.name()),
        Monster { kind },
        Pos(spot),
        Vitals {
            hp: kind.max_hp(),
            ..default()
        },
        PosHistory::default(),
        Brain {
            heading: Vec2::from_angle(rng.random_range(0.0..std::f32::consts::TAU)),
            turn_in: 0,
            attack_cd: rng.random_range(0..POD_VOLLEY_EVERY),
            spiral: rng.random_range(0.0..1.0),
        },
        Replicate::to_clients(NetworkTarget::All),
        InterpolationTarget::to_clients(NetworkTarget::All),
    ));
}

/// Nearest living combatant, if any.
fn nearest(players: &[Vec2], from: Vec2) -> Option<(Vec2, f32)> {
    players
        .iter()
        .map(|p| (*p, p.distance(from)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

fn roam(
    arena: Res<Arena>,
    obstacles: Res<Obstacles>,
    mut monsters: Query<(&Monster, &mut Pos, &mut Brain)>,
    players: Query<(&Pos, &Vitals), (With<Player>, Without<Monster>)>,
) {
    let alive: Vec<Vec2> = players
        .iter()
        .filter(|(_, v)| v.alive)
        .map(|(p, _)| p.0)
        .collect();
    let mut rng = rand::rng();
    for (monster, mut pos, mut brain) in &mut monsters {
        brain.attack_cd = brain.attack_cd.saturating_sub(1);
        if monster.kind != MonsterKind::Roach {
            continue;
        }
        // No pathfinding on purpose: a roach that sees you runs straight at you, otherwise it drifts.
        let prey = nearest(&alive, pos.0)
            .filter(|(p, d)| *d < ROACH_NOTICE && arena.line_of_sight(pos.0, *p));
        let velocity = match prey {
            Some((p, _)) => (p - pos.0).normalize_or_zero() * ROACH_CHARGE_SPEED,
            None => {
                if brain.turn_in == 0 {
                    brain.heading = Vec2::from_angle(rng.random_range(0.0..std::f32::consts::TAU));
                    brain.turn_in = rng.random_range(40..160);
                }
                brain.turn_in -= 1;
                brain.heading * ROACH_WANDER_SPEED
            }
        };
        let want = pos.0 + velocity * DT;
        let next = arena.resolve_with(want, monster.kind.radius(), &obstacles.0);
        if next.distance_squared(want) > 1.0 {
            // Bumped into a wall: pick a new direction next tick.
            brain.turn_in = 0;
        }
        if next != pos.0 {
            pos.0 = next;
        }
    }
}

fn attack(
    mut commands: Commands,
    state: Single<&MatchState>,
    arena: Res<Arena>,
    mut monsters: Query<(Entity, &Monster, &Pos, &mut Brain)>,
    players: Query<(Entity, &Pos, &Vitals), With<Player>>,
    mut damage: ResMut<DamageQueue>,
) {
    if state.phase == Phase::Intermission {
        return;
    }
    for (entity, monster, pos, mut brain) in &mut monsters {
        if brain.attack_cd > 0 {
            continue;
        }
        match monster.kind {
            MonsterKind::Roach => {
                let reach = monster.kind.radius() + PLAYER_RADIUS + 4.0;
                let victim = players
                    .iter()
                    .find(|(_, p, v)| v.alive && p.0.distance(pos.0) <= reach);
                if let Some((target, ..)) = victim {
                    brain.attack_cd = ROACH_BITE_COOLDOWN;
                    damage.0.push(Damage {
                        target,
                        attacker: Some(entity),
                        amount: ROACH_BITE,
                        armor_mult: 1.0,
                        weapon: None,
                        emp: false,
                        cause: Some("撕咬"),
                    });
                }
            }
            MonsterKind::SporePod => {
                let awake = players.iter().any(|(_, p, v)| {
                    v.alive && p.0.distance(pos.0) < POD_WAKE && arena.line_of_sight(pos.0, p.0)
                });
                if !awake {
                    continue;
                }
                brain.attack_cd = POD_VOLLEY_EVERY;
                brain.spiral += 0.23;
                let step = std::f32::consts::TAU / POD_GLOBS as f32;
                for i in 0..POD_GLOBS {
                    let dir = Vec2::from_angle(brain.spiral + i as f32 * step);
                    commands.spawn((
                        Name::new("Spore"),
                        Projectile {
                            kind: ProjectileKind::Spore,
                            vel: dir * SPORE_SPEED,
                        },
                        Pos(pos.0 + dir * (monster.kind.radius() + 6.0)),
                        Flight::simple(entity, SPORE_TTL, SPORE_DAMAGE),
                        projectile_replication(None),
                    ));
                }
            }
        }
    }
}

/// Monsters take damage here; everything aimed at a combatant is left for `resolve_damage`.
fn resolve_monster_damage(
    mut commands: Commands,
    mut queue: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
    mut monsters: Query<(&Monster, &Pos, &mut Vitals)>,
    players: Query<&Player>,
) {
    let mut rng = rand::rng();
    queue.0.retain(|dmg| {
        let Ok((monster, pos, mut vitals)) = monsters.get_mut(dmg.target) else {
            return true;
        };
        if !vitals.alive {
            return false;
        }
        let amount = dmg.amount.min(vitals.hp);
        vitals.hp -= amount;
        outbox.fast.push(GameEvent::Hit {
            pos: pos.0,
            amount,
            armor: false,
            attacker: dmg
                .attacker
                .and_then(|a| players.get(a).ok())
                .map_or(0, |p| p.client_id),
        });
        if vitals.hp <= 0.0 {
            vitals.alive = false;
            commands.entity(dmg.target).despawn();
            let loot = match monster.kind {
                MonsterKind::Roach if rng.random_bool(0.35) => Some(roll_loot(1, &mut rng)),
                MonsterKind::Roach if rng.random_bool(0.3) => Some(Item::Medkit),
                MonsterKind::Roach => None,
                MonsterKind::SporePod => Some(roll_loot(2, &mut rng)),
            };
            if let Some(item) = loot {
                spawn_pickup(&mut commands, item, pos.0, Some(LOOT_TTL));
            }
        }
        false
    });
}
