//! Weapons, projectiles, EMPs and pickups — all resolved with lag compensation:
//! targets are rewound to where the shooter saw them on screen.

use std::collections::HashSet;

use bevy::prelude::*;
use evoclash_game::items::{ArmorKind, Item, WeaponKind};
use evoclash_game::map::segment_circle;
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, DT, MAX_HP, PLAYER_RADIUS};
use lightyear::prelude::*;

use crate::connections::Human;
use crate::gameplay::{
    Cooldowns, CurrentInput, Damage, DamageQueue, Outbox, PosHistory, Rewind, SimSet, SimTick,
    spawn_pickup,
};

const PICKUP_RADIUS: f32 = 46.0;
const MAX_EMPS: u8 = 3;
const EMP_SPEED: f32 = 760.0;
const EMP_FUSE_TICKS: u16 = 36;
const EMP_RADIUS: f32 = 180.0;
const EMP_DAMAGE: f32 = 12.0;
const MELEE_ARC_COS: f32 = 0.45;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (fire_weapons, throw_emps, step_projectiles, pickups)
                .chain()
                .in_set(SimSet::Combat),
        );
    }
}

/// Server-only data attached to a projectile.
#[derive(Component)]
pub struct Flight {
    owner: Entity,
    rewind: u32,
    ttl: u16,
    damage: f32,
    armor_mult: f32,
    piercing: bool,
    already_hit: Vec<Entity>,
}

fn combat_allowed(state: &MatchState) -> bool {
    state.phase != Phase::Intermission
}

/// Where `target` was `rewind` ticks ago, from the shooter's point of view.
fn rewound(history: &PosHistory, now: u32, rewind: u32, fallback: Vec2) -> Vec2 {
    if rewind == 0 {
        return fallback;
    }
    history.at(now.wrapping_sub(rewind)).unwrap_or(fallback)
}

fn projectile_replication(human: Option<&Human>) -> (Replicate, InterpolationTarget) {
    // The shooter already drew this projectile locally the instant they clicked,
    // so only everyone else needs the server copy.
    let target = match human {
        Some(h) => NetworkTarget::AllExceptSingle(h.peer),
        None => NetworkTarget::All,
    };
    (
        Replicate::to_clients(target.clone()),
        InterpolationTarget::to_clients(target),
    )
}

#[allow(clippy::type_complexity)]
fn fire_weapons(
    mut commands: Commands,
    tick: Res<SimTick>,
    state: Single<&MatchState>,
    arena: Res<Arena>,
    mut shooters: Query<(
        Entity,
        &Pos,
        &CurrentInput,
        &Vitals,
        &mut Loadout,
        &mut Cooldowns,
        &Rewind,
        Option<&Human>,
    )>,
    targets: Query<(Entity, &Pos, &Vitals, &PosHistory)>,
    mut damage: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
) {
    if !combat_allowed(&state) {
        return;
    }
    let mut rng = rand::rng();
    for (entity, pos, input, vitals, mut loadout, mut cd, rewind, human) in &mut shooters {
        let input = &input.0;
        if !vitals.alive || !input.fire || cd.fire > 0 {
            continue;
        }
        let aim = input.aim.normalize_or(Vec2::X);
        let stats = loadout.weapon.stats();
        cd.fire = stats.cooldown;

        if stats.melee {
            let reach = stats.range_or_speed + PLAYER_RADIUS;
            for (target, target_pos, target_vitals, history) in &targets {
                if target == entity || !target_vitals.alive {
                    continue;
                }
                let seen_at = rewound(history, tick.0, rewind.0, target_pos.0);
                let offset = seen_at - pos.0;
                let dist = offset.length();
                if dist > reach || offset.normalize_or_zero().dot(aim) < MELEE_ARC_COS {
                    continue;
                }
                if !arena.line_of_sight(pos.0, seen_at) {
                    continue;
                }
                damage.0.push(Damage {
                    target,
                    attacker: Some(entity),
                    amount: stats.damage,
                    armor_mult: stats.armor_mult,
                    weapon: loadout.weapon,
                    emp: false,
                });
            }
            outbox.fast.push(GameEvent::Melee {
                pos: pos.0,
                dir: aim,
            });
            continue;
        }

        let base = aim.to_angle();
        for i in 0..stats.pellets {
            // Shotguns fan out evenly; everything else gets a little random bloom.
            let fan = if stats.pellets > 1 {
                (i as f32 / (stats.pellets - 1) as f32 - 0.5) * stats.spread
            } else {
                0.0
            };
            let bloom = if stats.spread > 0.0 && stats.pellets == 1 {
                rand::Rng::random_range(&mut rng, -stats.spread..stats.spread)
            } else {
                0.0
            };
            let dir = Vec2::from_angle(base + fan + bloom);
            commands.spawn((
                Name::new("Projectile"),
                Projectile {
                    kind: ProjectileKind::Shot(loadout.weapon),
                    vel: dir * stats.range_or_speed,
                },
                Pos(pos.0 + dir * (PLAYER_RADIUS + 4.0)),
                Flight {
                    owner: entity,
                    rewind: rewind.0,
                    ttl: stats.lifetime,
                    damage: stats.damage,
                    armor_mult: stats.armor_mult,
                    piercing: stats.piercing,
                    already_hit: Vec::new(),
                },
                projectile_replication(human),
            ));
        }
        loadout.ammo = loadout.ammo.saturating_sub(1);
        if loadout.ammo == 0 {
            // Out of ammo: back to the stone age.
            loadout.weapon = WeaponKind::Stick;
        }
    }
}

fn throw_emps(
    mut commands: Commands,
    state: Single<&MatchState>,
    mut throwers: Query<(
        Entity,
        &Pos,
        &CurrentInput,
        &Vitals,
        &mut Loadout,
        &mut Cooldowns,
        Option<&Human>,
    )>,
) {
    if !combat_allowed(&state) {
        return;
    }
    for (entity, pos, input, vitals, mut loadout, mut cd, human) in &mut throwers {
        if !vitals.alive || !input.0.throw || cd.throw > 0 || loadout.emps == 0 {
            continue;
        }
        cd.throw = 30;
        loadout.emps -= 1;
        let dir = input.0.aim.normalize_or(Vec2::X);
        commands.spawn((
            Name::new("Emp"),
            Projectile {
                kind: ProjectileKind::Emp,
                vel: dir * EMP_SPEED,
            },
            Pos(pos.0 + dir * (PLAYER_RADIUS + 6.0)),
            Flight {
                owner: entity,
                rewind: 0,
                ttl: EMP_FUSE_TICKS,
                damage: EMP_DAMAGE,
                armor_mult: 1.0,
                piercing: false,
                already_hit: Vec::new(),
            },
            projectile_replication(human),
        ));
    }
}

fn step_projectiles(
    mut commands: Commands,
    tick: Res<SimTick>,
    arena: Res<Arena>,
    mut projectiles: Query<(Entity, &mut Pos, &Projectile, &mut Flight)>,
    targets: Query<(Entity, &Pos, &Vitals, &PosHistory), Without<Projectile>>,
    mut damage: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
) {
    for (entity, mut pos, projectile, mut flight) in &mut projectiles {
        let from = pos.0;
        let to = from + projectile.vel * DT;
        let wall_t = if flight.piercing {
            None
        } else {
            arena.segment_hits_wall(from, to)
        };
        flight.ttl = flight.ttl.saturating_sub(1);

        match projectile.kind {
            ProjectileKind::Emp => {
                if wall_t.is_some() || flight.ttl == 0 {
                    let at = from + (to - from) * wall_t.unwrap_or(1.0) * 0.95;
                    for (target, target_pos, vitals, _) in &targets {
                        if target == flight.owner || !vitals.alive {
                            continue;
                        }
                        if target_pos.0.distance(at) <= EMP_RADIUS
                            && arena.line_of_sight(at, target_pos.0)
                        {
                            damage.0.push(Damage {
                                target,
                                attacker: Some(flight.owner),
                                amount: flight.damage,
                                armor_mult: 1.0,
                                weapon: WeaponKind::Stick,
                                emp: true,
                            });
                        }
                    }
                    outbox.fast.push(GameEvent::Boom {
                        pos: at,
                        radius: EMP_RADIUS,
                    });
                    commands.entity(entity).despawn();
                } else {
                    pos.0 = to;
                }
            }
            ProjectileKind::Shot(weapon) => {
                let limit = wall_t.unwrap_or(1.0);
                let mut hits: Vec<(f32, Entity)> = targets
                    .iter()
                    .filter(|(target, _, vitals, _)| {
                        *target != flight.owner
                            && vitals.alive
                            && !flight.already_hit.contains(target)
                    })
                    .filter_map(|(target, target_pos, _, history)| {
                        let seen_at = rewound(history, tick.0, flight.rewind, target_pos.0);
                        segment_circle(from, to, seen_at, PLAYER_RADIUS + 3.0)
                            .filter(|t| *t <= limit)
                            .map(|t| (t, target))
                    })
                    .collect();
                hits.sort_by(|a, b| a.0.total_cmp(&b.0));
                if !flight.piercing {
                    hits.truncate(1);
                }
                for (_, target) in &hits {
                    flight.already_hit.push(*target);
                    damage.0.push(Damage {
                        target: *target,
                        attacker: Some(flight.owner),
                        amount: flight.damage,
                        armor_mult: flight.armor_mult,
                        weapon,
                        emp: false,
                    });
                }
                let stopped = (!flight.piercing && !hits.is_empty()) || wall_t.is_some();
                if stopped || flight.ttl == 0 {
                    commands.entity(entity).despawn();
                } else {
                    pos.0 = to;
                }
            }
        }
    }
}

/// Would a player grab this automatically just by walking over it?
pub fn auto_wants(item: &Item, vitals: &Vitals, loadout: &Loadout) -> bool {
    match *item {
        Item::Weapon { kind, .. } => {
            kind.stats().tier > loadout.weapon.stats().tier
                || (kind == loadout.weapon
                    && kind != WeaponKind::Stick
                    && loadout.ammo < kind.stats().max_ammo)
        }
        Item::Armor { points, .. } => points > vitals.armor_points,
        Item::Emp => loadout.emps < MAX_EMPS,
        Item::Medkit => vitals.hp < MAX_HP,
    }
}

/// Equip `item`, returning whatever had to be put down to make room.
fn take(item: Item, vitals: &mut Vitals, loadout: &mut Loadout) -> Option<Item> {
    match item {
        Item::Weapon { kind, ammo } => {
            if kind == loadout.weapon {
                loadout.ammo = (loadout.ammo + ammo).min(kind.stats().max_ammo);
                return None;
            }
            let old =
                (loadout.weapon != WeaponKind::Stick && loadout.ammo > 0).then_some(Item::Weapon {
                    kind: loadout.weapon,
                    ammo: loadout.ammo,
                });
            loadout.weapon = kind;
            loadout.ammo = ammo;
            old
        }
        Item::Armor { kind, points } => {
            let old = (vitals.armor != ArmorKind::None && vitals.armor_points > 0.0).then_some(
                Item::Armor {
                    kind: vitals.armor,
                    points: vitals.armor_points,
                },
            );
            vitals.armor = kind;
            vitals.armor_points = points;
            old
        }
        Item::Emp => {
            loadout.emps = (loadout.emps + 1).min(MAX_EMPS);
            None
        }
        Item::Medkit => {
            vitals.hp = (vitals.hp + 50.0).min(MAX_HP);
            None
        }
    }
}

fn pickups(
    mut commands: Commands,
    mut players: Query<(
        &Pos,
        &CurrentInput,
        &mut Vitals,
        &mut Loadout,
        &mut Cooldowns,
    )>,
    items: Query<(Entity, &Pos, &Pickup)>,
) {
    let mut taken = HashSet::new();
    for (pos, input, mut vitals, mut loadout, mut cd) in &mut players {
        if !vitals.alive {
            continue;
        }
        let mut near: Vec<(f32, Entity, Item)> = items
            .iter()
            .filter(|(e, ..)| !taken.contains(e))
            .map(|(e, p, pickup)| (p.0.distance(pos.0), e, pickup.item))
            .filter(|(d, ..)| *d <= PICKUP_RADIUS)
            .collect();
        if near.is_empty() {
            continue;
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let forced = input.0.interact && cd.interact == 0;
        for (_, entity, item) in near {
            if !forced && !auto_wants(&item, &vitals, &loadout) {
                continue;
            }
            taken.insert(entity);
            commands.entity(entity).despawn();
            if let Some(dropped) = take(item, &mut vitals, &mut loadout) {
                spawn_pickup(
                    &mut commands,
                    dropped,
                    pos.0 + Vec2::new(0.0, -18.0),
                    Some(30 * 64),
                );
            }
            if forced {
                // One deliberate swap per press.
                cd.interact = 20;
                break;
            }
        }
    }
}
