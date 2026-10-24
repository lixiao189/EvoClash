//! The arena's interactive bits: crates to smash for loot, fuel barrels that blow up whatever
//! stands around them, safes to crack, and radiation zones that eat at anyone without power
//! armor.

use bevy::prelude::*;
use evoclash_game::items::{ArmorKind, Item, roll_loot};
use evoclash_game::map::PropKind;
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, Obstacles, PLAYER_RADIUS};
use lightyear::prelude::*;
use rand::Rng;

use crate::gameplay::{
    CurrentInput, Damage, DamageQueue, Outbox, PosHistory, SimSet, SimTick, TICKS_PER_SEC,
    resolve_damage, spawn_pickup,
};

const CRATE_RESPAWN_TICKS: u32 = 40 * TICKS_PER_SEC;
const BARREL_RESPAWN_TICKS: u32 = 45 * TICKS_PER_SEC;
const LOOT_TTL: u32 = 30 * TICKS_PER_SEC;
/// Holding interact this long next to a safe cracks it.
const CRACK_TICKS: u32 = (2.5 * TICKS_PER_SEC as f32) as u32;
const SAFE_REACH: f32 = 40.0;
/// A cracked safe stays empty this long.
const SAFE_RESEAL_TICKS: u32 = 90 * TICKS_PER_SEC;
const BLAST_RADIUS: f32 = 150.0;
const BLAST_DAMAGE: f32 = 60.0;
/// Ticks between a barrel bursting and its blast, so chain reactions ripple outward.
const BLAST_FUSE: u16 = 5;
const RAD_EVERY: u32 = TICKS_PER_SEC / 4;
const RAD_DAMAGE: f32 = 2.0;

pub struct EnvironmentPlugin;

impl Plugin for EnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PropSlots>().init_resource::<Blasts>();
        app.add_systems(
            FixedUpdate,
            (
                (spawn_props, track_obstacles).chain().in_set(SimSet::Input),
                irradiate.in_set(SimSet::Simulate),
                (detonate, crack_safes).in_set(SimSet::Combat),
                resolve_prop_damage
                    .in_set(SimSet::Resolve)
                    .before(resolve_damage),
            ),
        );
    }
}

#[derive(Default)]
struct Slot {
    entity: Option<Entity>,
    respawn_in: u32,
}

/// One slot per prop spot of the current layout. Cleared when the match resets.
#[derive(Resource, Default)]
pub struct PropSlots(Vec<Slot>);

impl PropSlots {
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

struct Blast {
    pos: Vec2,
    attacker: Option<Entity>,
    fuse: u16,
}

/// Barrels that burst and are about to explode.
#[derive(Resource, Default)]
pub struct Blasts(Vec<Blast>);

impl Blasts {
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Server-side safe state.
#[derive(Component, Default)]
struct Cracking {
    by: Option<Entity>,
    ticks: u32,
    reseal: u32,
}

fn spawn_props(
    mut commands: Commands,
    arena: Res<Arena>,
    mut slots: ResMut<PropSlots>,
    live: Query<(), With<Prop>>,
    players: Query<(&Pos, &Vitals), With<Player>>,
) {
    if slots.0.len() != arena.props.len() {
        slots.0 = arena.props.iter().map(|_| Slot::default()).collect();
    }
    for (slot, spot) in slots.0.iter_mut().zip(&arena.props) {
        if let Some(entity) = slot.entity {
            if live.contains(entity) {
                continue;
            }
            slot.entity = None;
            slot.respawn_in = match spot.kind {
                PropKind::Barrel => BARREL_RESPAWN_TICKS,
                _ => CRATE_RESPAWN_TICKS,
            };
        }
        if slot.respawn_in > 0 {
            slot.respawn_in -= 1;
            continue;
        }
        // Never pop up on top of someone.
        let clear = spot.kind.radius() + PLAYER_RADIUS + 4.0;
        if players
            .iter()
            .any(|(p, v)| v.alive && p.0.distance(spot.pos) < clear)
        {
            continue;
        }
        let mut entity = commands.spawn((
            Name::new(spot.kind.name()),
            Prop {
                kind: spot.kind,
                tier: spot.tier,
                progress: 0,
                open: false,
            },
            Pos(spot.pos),
            Vitals {
                hp: spot.kind.max_hp(),
                ..default()
            },
            PosHistory::default(),
            Replicate::to_clients(NetworkTarget::All),
        ));
        if spot.kind == PropKind::Safe {
            entity.insert(Cracking::default());
        }
        slot.entity = Some(entity.id());
    }
}

fn track_obstacles(mut obstacles: ResMut<Obstacles>, props: Query<(&Pos, &Prop, &Vitals)>) {
    obstacles.0.clear();
    obstacles.0.extend(
        props
            .iter()
            .filter(|(.., v)| v.alive)
            .map(|(p, prop, _)| (p.0, prop.kind.radius())),
    );
}

/// Radiation chews through anyone not sealed in power armor.
fn irradiate(
    tick: Res<SimTick>,
    arena: Res<Arena>,
    players: Query<(Entity, &Pos, &Vitals), With<Player>>,
    mut damage: ResMut<DamageQueue>,
) {
    if !tick.0.is_multiple_of(RAD_EVERY) || arena.hazards.is_empty() {
        return;
    }
    for (entity, pos, vitals) in &players {
        let sealed = vitals.armor == ArmorKind::PowerArmor && vitals.armor_points > 0.0;
        if vitals.alive && !sealed && arena.in_hazard(pos.0) {
            damage.0.push(Damage {
                target: entity,
                attacker: None,
                amount: RAD_DAMAGE,
                armor_mult: 1.0,
                weapon: None,
                emp: false,
                cause: Some("辐射"),
            });
        }
    }
}

#[allow(clippy::type_complexity)]
fn detonate(
    arena: Res<Arena>,
    mut blasts: ResMut<Blasts>,
    targets: Query<(Entity, &Pos, &Vitals), Or<(With<Player>, With<Monster>, With<Prop>)>>,
    mut damage: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
) {
    blasts.0.retain_mut(|blast| {
        if blast.fuse > 0 {
            blast.fuse -= 1;
            return true;
        }
        for (entity, pos, vitals) in &targets {
            let dist = pos.0.distance(blast.pos);
            if !vitals.alive || dist > BLAST_RADIUS || !arena.line_of_sight(blast.pos, pos.0) {
                continue;
            }
            damage.0.push(Damage {
                target: entity,
                attacker: blast.attacker,
                amount: BLAST_DAMAGE * (1.0 - 0.5 * dist / BLAST_RADIUS),
                armor_mult: 1.0,
                weapon: None,
                emp: false,
                cause: Some("油桶爆炸"),
            });
        }
        outbox.fast.push(GameEvent::Boom {
            pos: blast.pos,
            radius: BLAST_RADIUS,
            fire: true,
        });
        false
    });
}

/// Hold interact next to a safe, standing still, to crack it open.
fn crack_safes(
    mut commands: Commands,
    mut safes: Query<(&mut Prop, &Pos, &mut Cracking)>,
    players: Query<(Entity, &Pos, &Vitals, &CurrentInput), With<Player>>,
) {
    let mut rng = rand::rng();
    for (mut prop, pos, mut cracking) in &mut safes {
        if prop.open {
            cracking.reseal = cracking.reseal.saturating_sub(1);
            if cracking.reseal == 0 {
                prop.open = false;
            }
            continue;
        }
        let reach = prop.kind.radius() + PLAYER_RADIUS + SAFE_REACH;
        let working = |(_, p, v, input): &(Entity, &Pos, &Vitals, &CurrentInput)| {
            v.alive
                && input.0.interact
                && input.0.move_dir() == Vec2::ZERO
                && p.0.distance(pos.0) <= reach
        };
        // Whoever started keeps going; otherwise the first one to try takes over.
        let cracker = players
            .iter()
            .filter(&working)
            .find(|(e, ..)| Some(*e) == cracking.by)
            .or_else(|| players.iter().find(&working));
        match cracker {
            Some((entity, cracker_pos, ..)) => {
                if cracking.by != Some(entity) {
                    cracking.by = Some(entity);
                    cracking.ticks = 0;
                }
                cracking.ticks += 1;
                if cracking.ticks >= CRACK_TICKS {
                    let toward = (cracker_pos.0 - pos.0).normalize_or(Vec2::NEG_Y);
                    let drop_at = pos.0 + toward * (prop.kind.radius() + 18.0);
                    let tier = (prop.tier + 1).min(4);
                    spawn_pickup(
                        &mut commands,
                        roll_loot(tier, &mut rng),
                        drop_at,
                        Some(LOOT_TTL),
                    );
                    if rng.random_bool(0.5) {
                        let side = toward.perp() * 26.0;
                        spawn_pickup(&mut commands, Item::Emp, drop_at + side, Some(LOOT_TTL));
                    }
                    prop.open = true;
                    prop.progress = 0;
                    cracking.by = None;
                    cracking.ticks = 0;
                    cracking.reseal = SAFE_RESEAL_TICKS;
                    continue;
                }
            }
            None => {
                cracking.by = None;
                cracking.ticks = 0;
            }
        }
        let progress = (cracking.ticks * 100 / CRACK_TICKS) as u8;
        if prop.progress != progress {
            prop.progress = progress;
        }
    }
}

/// Props take damage here; everything aimed elsewhere is left for the other resolvers.
fn resolve_prop_damage(
    mut commands: Commands,
    mut queue: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
    mut blasts: ResMut<Blasts>,
    mut props: Query<(&Prop, &Pos, &mut Vitals)>,
    players: Query<&Player>,
) {
    let mut rng = rand::rng();
    queue.0.retain(|dmg| {
        let Ok((prop, pos, mut vitals)) = props.get_mut(dmg.target) else {
            return true;
        };
        // Safes shrug everything off; EMPs don't bother crates.
        if !vitals.alive || prop.kind == PropKind::Safe || dmg.emp {
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
        if vitals.hp > 0.0 {
            return false;
        }
        vitals.alive = false;
        commands.entity(dmg.target).despawn();
        match prop.kind {
            PropKind::Crate => {
                let item = if rng.random_bool(0.75) {
                    Some(roll_loot(prop.tier, &mut rng))
                } else if rng.random_bool(0.5) {
                    Some(Item::Medkit)
                } else {
                    None
                };
                if let Some(item) = item {
                    spawn_pickup(&mut commands, item, pos.0, Some(LOOT_TTL));
                }
            }
            PropKind::Barrel => blasts.0.push(Blast {
                pos: pos.0,
                attacker: dmg.attacker,
                fuse: BLAST_FUSE,
            }),
            PropKind::Safe => {}
        }
        false
    });
}
