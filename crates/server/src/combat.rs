//! Weapons, projectiles, EMPs and pickups — all resolved with lag compensation:
//! targets are rewound to where the shooter saw them on screen.

use std::collections::HashSet;

use bevy::prelude::*;
use evoclash_game::evolution::{Evolution, SkillState};
use evoclash_game::items::{ArmorKind, Item, WeaponKind};
use evoclash_game::map::segment_circle;
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, DT, PLAYER_RADIUS};
use lightyear::prelude::*;

use crate::connections::Human;
use crate::gameplay::{
    Cooldowns, CurrentInput, Damage, DamageQueue, Outbox, PosHistory, Rewind, SimSet, SimTick,
    spawn_pickup,
};
use crate::monsters::spawn_puddle;
use crate::skills::TurretBrain;

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
    /// Fired by a monster: passes through other monsters and props.
    fauna: bool,
    /// Kill-feed name when it isn't the weapon's (e.g. "自动炮塔", "酸液").
    cause: Option<&'static str>,
    /// EMP blast radius.
    radius: f32,
}

impl Flight {
    /// A plain, non-rewound monster projectile (spore, acid, bolt).
    pub fn fauna(owner: Entity, ttl: u16, damage: f32, cause: &'static str) -> Self {
        Self {
            owner,
            rewind: 0,
            ttl,
            damage,
            armor_mult: 1.0,
            piercing: false,
            already_hit: Vec::new(),
            fauna: true,
            cause: Some(cause),
            radius: 0.0,
        }
    }

    /// A raider's gunshot: weapon stats, but it never hits other monsters.
    pub fn raider_shot(owner: Entity, weapon: WeaponKind) -> Self {
        let stats = weapon.stats();
        Self {
            owner,
            rewind: 0,
            ttl: stats.lifetime,
            damage: stats.damage,
            armor_mult: stats.armor_mult,
            piercing: stats.piercing,
            already_hit: Vec::new(),
            fauna: true,
            cause: None,
            radius: 0.0,
        }
    }

    /// A shot fired on someone's behalf (a turret): hits are credited to `owner`.
    pub fn turret_shot(owner: Entity, ttl: u16, damage: f32, cause: &'static str) -> Self {
        Self {
            owner,
            rewind: 0,
            ttl,
            damage,
            armor_mult: 1.0,
            piercing: false,
            already_hit: Vec::new(),
            fauna: false,
            cause: Some(cause),
            radius: 0.0,
        }
    }
}

/// Hit radius of a combatant, monster or prop.
fn body_radius(monster: Option<&Monster>, prop: Option<&Prop>) -> f32 {
    match (monster, prop) {
        (Some(m), _) => m.kind.radius(),
        (_, Some(p)) => p.kind.radius(),
        _ => PLAYER_RADIUS,
    }
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

pub fn projectile_replication(human: Option<&Human>) -> (Replicate, InterpolationTarget) {
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
        &Evolution,
        &SkillState,
    )>,
    targets: Query<(
        Entity,
        &Pos,
        &Vitals,
        &PosHistory,
        Option<&Monster>,
        Option<&Prop>,
    )>,
    mut damage: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
) {
    if !combat_allowed(&state) {
        return;
    }
    let mut rng = rand::rng();
    for (entity, pos, input, vitals, mut loadout, mut cd, rewind, human, evo, skills) in
        &mut shooters
    {
        let input = &input.0;
        if !vitals.alive || !input.fire || cd.fire > 0 {
            continue;
        }
        let aim = input.aim.normalize_or(Vec2::X);
        let stats = loadout.weapon.stats();
        cd.fire = evo.fire_cooldown(stats.cooldown);
        // Perks (利爪, 能量聚焦) and 狂暴.
        let damage_amount =
            stats.damage * evo.weapon_damage_mult(loadout.weapon) * skills.damage_mult();
        let armor_mult = stats.armor_mult + evo.armor_mult_bonus(loadout.weapon);

        if stats.melee {
            for (target, target_pos, target_vitals, history, monster, prop) in &targets {
                if target == entity || !target_vitals.alive {
                    continue;
                }
                let reach =
                    stats.range_or_speed + evo.melee_reach_bonus() + body_radius(monster, prop);
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
                    amount: damage_amount,
                    armor_mult,
                    weapon: Some(loadout.weapon),
                    emp: false,
                    cause: None,
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
                    damage: damage_amount,
                    armor_mult,
                    piercing: stats.piercing,
                    already_hit: Vec::new(),
                    fauna: false,
                    cause: None,
                    radius: 0.0,
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
        &Evolution,
    )>,
) {
    if !combat_allowed(&state) {
        return;
    }
    for (entity, pos, input, vitals, mut loadout, mut cd, human, evo) in &mut throwers {
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
                fauna: false,
                cause: None,
                radius: EMP_RADIUS * evo.emp_radius_mult(),
            },
            projectile_replication(human),
        ));
    }
}

pub(crate) fn step_projectiles(
    mut commands: Commands,
    tick: Res<SimTick>,
    arena: Res<Arena>,
    mut projectiles: Query<(Entity, &mut Pos, &Projectile, &mut Flight)>,
    targets: Query<
        (
            Entity,
            &Pos,
            &Vitals,
            &PosHistory,
            Option<&Monster>,
            Option<&Prop>,
            Option<&TurretBrain>,
        ),
        Without<Projectile>,
    >,
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
                    for (target, target_pos, vitals, ..) in &targets {
                        if target == flight.owner || !vitals.alive {
                            continue;
                        }
                        if target_pos.0.distance(at) <= flight.radius
                            && arena.line_of_sight(at, target_pos.0)
                        {
                            damage.0.push(Damage {
                                target,
                                attacker: Some(flight.owner),
                                amount: flight.damage,
                                armor_mult: 1.0,
                                weapon: Some(WeaponKind::Stick),
                                emp: true,
                                cause: Some("电磁脉冲"),
                            });
                        }
                    }
                    outbox.fast.push(GameEvent::Boom {
                        pos: at,
                        radius: flight.radius,
                        fire: false,
                    });
                    commands.entity(entity).despawn();
                } else {
                    pos.0 = to;
                }
            }
            ProjectileKind::Shot(_)
            | ProjectileKind::Spore
            | ProjectileKind::Acid
            | ProjectileKind::Bolt => {
                let weapon = match projectile.kind {
                    ProjectileKind::Shot(weapon) => Some(weapon),
                    _ => None,
                };
                let fauna = flight.fauna;
                let limit = wall_t.unwrap_or(1.0);
                let mut hits: Vec<(f32, Entity)> = targets
                    .iter()
                    .filter(|(target, _, vitals, _, monster, prop, turret)| {
                        *target != flight.owner
                            && vitals.alive
                            // Your own turret doesn't soak up your shots.
                            && turret.is_none_or(|t| t.owner() != flight.owner)
                            // Monsters don't hurt each other, or set off the scenery.
                            && !(fauna && (monster.is_some() || prop.is_some()))
                            && !flight.already_hit.contains(target)
                    })
                    .filter_map(|(target, target_pos, _, history, monster, prop, _)| {
                        let seen_at = rewound(history, tick.0, flight.rewind, target_pos.0);
                        segment_circle(from, to, seen_at, body_radius(monster, prop) + 3.0)
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
                        cause: flight.cause,
                    });
                }
                let stopped = (!flight.piercing && !hits.is_empty()) || wall_t.is_some();
                if stopped || flight.ttl == 0 {
                    if projectile.kind == ProjectileKind::Acid {
                        let t = hits.first().map_or(limit, |h| h.0);
                        // Land just short of the wall, not inside it.
                        let at = from + (to - from) * t * 0.9;
                        spawn_puddle(&mut commands, PuddleKind::Acid, at, Some(flight.owner));
                    }
                    commands.entity(entity).despawn();
                } else {
                    pos.0 = to;
                }
            }
        }
    }
}

/// Would a player grab this automatically just by walking over it?
pub fn auto_wants(item: &Item, vitals: &Vitals, loadout: &Loadout, evo: &Evolution) -> bool {
    match *item {
        Item::Weapon { kind, .. } => {
            kind.stats().tier > loadout.weapon.stats().tier
                || (kind == loadout.weapon
                    && kind != WeaponKind::Stick
                    && loadout.ammo < evo.max_ammo(kind))
        }
        Item::Armor { points, .. } => points > vitals.armor_points,
        Item::Emp => loadout.emps < evo.emp_cap(MAX_EMPS),
        Item::Medkit => vitals.hp < vitals.max_hp,
    }
}

/// Equip `item`, returning whatever had to be put down to make room.
fn take(item: Item, vitals: &mut Vitals, loadout: &mut Loadout, evo: &Evolution) -> Option<Item> {
    match item {
        Item::Weapon { kind, ammo } => {
            if kind == loadout.weapon {
                loadout.ammo = (loadout.ammo + ammo).min(evo.max_ammo(kind));
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
            // 复合装甲 raises the cap and tops fresh armor up to it.
            let bonus = evo.armor_cap(kind) - kind.max_points();
            vitals.armor_points = (points + bonus).min(evo.armor_cap(kind)).max(points);
            old
        }
        Item::Emp => {
            loadout.emps = (loadout.emps + 1).min(evo.emp_cap(MAX_EMPS));
            None
        }
        Item::Medkit => {
            vitals.hp = (vitals.hp + 50.0).min(vitals.max_hp);
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
        &Evolution,
    )>,
    items: Query<(Entity, &Pos, &Pickup)>,
) {
    let mut taken = HashSet::new();
    for (pos, input, mut vitals, mut loadout, mut cd, evo) in &mut players {
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
            if !forced && !auto_wants(&item, &vitals, &loadout, evo) {
                continue;
            }
            taken.insert(entity);
            commands.entity(entity).despawn();
            if let Some(dropped) = take(item, &mut vitals, &mut loadout, evo) {
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
