//! Scavenger bots keep small lobbies lively and step aside as humans join.

use std::collections::HashMap;

use bevy::prelude::*;
use evoclash_game::items::ArmorKind;
use evoclash_game::map::ARENA_HALF;
use evoclash_game::nav::{NavGrid, next_waypoint};
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, DT, PLAYER_RADIUS};
use rand::Rng;

use crate::combat::auto_wants;
use crate::config::ServerConfig;
use crate::connections::Human;
use crate::gameplay::{CurrentInput, SimSet, spawn_combatant};

const SIGHT: f32 = 820.0;
const THINK_EVERY: u16 = 4;
/// Think cycles a bot must keep a target before it opens fire (~0.3 s): a reaction time.
const REACTION_THINKS: u16 = 3;
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
}

#[derive(Resource)]
struct Nav(NavGrid);

pub struct BotsPlugin;

impl Plugin for BotsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (rebuild_nav, balance_bots, think)
                .chain()
                .in_set(SimSet::Input),
        );
    }
}

/// Each match has a new layout, so the grid is rebuilt whenever the arena changes.
fn rebuild_nav(mut commands: Commands, arena: Res<Arena>, nav: Option<Res<Nav>>) {
    if nav.is_none() || arena.is_changed() {
        commands.insert_resource(Nav(NavGrid::new(&arena, PLAYER_RADIUS + 6.0)));
    }
}

fn balance_bots(
    mut commands: Commands,
    config: Res<ServerConfig>,
    humans: Query<&Human>,
    bots: Query<(Entity, &Player), With<Bot>>,
    mut counter: Local<u64>,
    mut cooldown: Local<u32>,
) {
    // Adjust at most once a second so joins/leaves don't thrash.
    if *cooldown > 0 {
        *cooldown -= 1;
        return;
    }
    *cooldown = 64;
    let humans = humans.iter().count();
    let bot_count = bots.iter().count();
    let wanted = config
        .min_combatants
        .saturating_sub(humans)
        .min(config.max_players.saturating_sub(humans));
    if bot_count < wanted {
        *counter += 1;
        let n = *counter;
        let name = format!("{}{}", NAMES[(n as usize - 1) % NAMES.len()], n);
        let player = Player {
            // High ids can't collide with matchmaker-issued client ids.
            client_id: u64::MAX - n,
            name,
            color: [0.7, 0.7, 0.7],
            bot: true,
        };
        let entity = spawn_combatant(&mut commands, player, None);
        commands.entity(entity).insert(Bot {
            goal: Vec2::ZERO,
            last_pos: Vec2::ZERO,
            stuck: 0,
            strafe: 1.0,
            think_in: 0,
            aim_error: rand::rng().random_range(-0.12..0.12),
            target: None,
            locked_for: 0,
        });
    } else if bot_count > wanted
        && let Some((entity, player)) = bots.iter().next()
    {
        info!("bot {} leaves to make room", player.name);
        commands.entity(entity).despawn();
    }
}

fn to_axis(v: f32) -> i8 {
    if v > 0.38 {
        1
    } else if v < -0.38 {
        -1
    } else {
        0
    }
}

fn steer(input: &mut PlayerInput, dir: Vec2) {
    let dir = dir.normalize_or_zero();
    input.move_x = to_axis(dir.x);
    input.move_y = to_axis(dir.y);
}

#[allow(clippy::type_complexity)]
fn think(
    arena: Res<Arena>,
    nav: Option<Res<Nav>>,
    mut bots: Query<(Entity, &mut Bot, &mut CurrentInput, &Pos, &Vitals, &Loadout)>,
    others: Query<
        (Entity, &Pos, &Vitals, Option<&Monster>, Has<Human>),
        Or<(With<Player>, With<Monster>)>,
    >,
    items: Query<(&Pos, &Pickup)>,
) {
    let Some(nav) = nav else {
        return;
    };
    let mut rng = rand::rng();
    let mut claims: HashMap<Entity, u32> = HashMap::new();
    for (_, bot, ..) in &bots {
        if let Some(target) = bot.target {
            *claims.entry(target).or_default() += 1;
        }
    }
    for (entity, mut bot, mut current, pos, vitals, loadout) in &mut bots {
        if !vitals.alive {
            current.0 = PlayerInput::default();
            bot.target = None;
            continue;
        }
        // Radiation is only worth braving in power armor.
        let sealed = vitals.armor == ArmorKind::PowerArmor && vitals.armor_points > 0.0;
        let safe = |p: Vec2| sealed || !arena.in_hazard(p);
        // Stuck on a wall: pick somewhere else.
        if pos.0.distance_squared(bot.last_pos) < 0.25 && current.0.move_dir() != Vec2::ZERO {
            bot.stuck += 1;
        } else {
            bot.stuck = 0;
        }
        bot.last_pos = pos.0;

        if bot.think_in > 0 && bot.stuck < 12 {
            bot.think_in -= 1;
            // Keep holding the previous decision but never hold dash/throw across ticks.
            current.0.dash = false;
            current.0.throw = false;
            continue;
        }
        bot.think_in = THINK_EVERY;
        let mut input = PlayerInput::default();

        let previous = bot.target;
        let enemy = others
            .iter()
            .filter(|(e, _, v, ..)| *e != entity && v.alive && v.shield == 0)
            .map(|(e, p, v, monster, human)| {
                let dist = p.0.distance(pos.0);
                let others_on_it =
                    claims.get(&e).copied().unwrap_or(0) - u32::from(previous == Some(e));
                let mut weight = 1.0 + PILE_ON_WEIGHT * others_on_it as f32;
                if monster.is_some() {
                    weight *= MONSTER_WEIGHT;
                }
                if human {
                    weight *= HUMAN_WEIGHT;
                }
                if previous == Some(e) {
                    weight *= STICKY_WEIGHT;
                }
                (e, p.0, v.armor_points, dist, dist * weight)
            })
            .filter(|(_, p, _, d, _)| *d < SIGHT && arena.line_of_sight(pos.0, *p))
            .min_by(|a, b| a.4.total_cmp(&b.4));

        bot.target = enemy.map(|(e, ..)| e);
        if bot.target != previous {
            bot.locked_for = 0;
        }
        bot.locked_for = bot.locked_for.saturating_add(1);

        if let Some((_, enemy_pos, enemy_armor, dist, _)) = enemy {
            let to_enemy = (enemy_pos - pos.0).normalize_or(Vec2::X);
            let wobble = rng.random_range(-0.05..0.05);
            input.aim = Vec2::from_angle(to_enemy.to_angle() + bot.aim_error + wobble);
            let stats = loadout.weapon.stats();
            let range = if stats.melee {
                stats.range_or_speed + 20.0
            } else {
                stats.range_or_speed * stats.lifetime as f32 * DT * 0.8
            };
            if rng.random_bool(0.03) {
                bot.strafe = -bot.strafe;
            }
            let strafe = to_enemy.perp() * bot.strafe;
            let dir = if stats.melee || dist > range * 0.85 {
                let waypoint = next_waypoint(&nav.0, &arena, pos.0, enemy_pos, PLAYER_RADIUS);
                (waypoint - pos.0).normalize_or_zero() + strafe * 0.3
            } else if dist < range * 0.35 {
                -to_enemy + strafe
            } else {
                strafe
            };
            steer(&mut input, dir);
            input.fire = bot.locked_for > REACTION_THINKS && dist < range && rng.random_bool(0.85);
            input.dash = dist < 260.0 && rng.random_bool(0.04);
            input.throw =
                loadout.emps > 0 && enemy_armor >= 50.0 && dist < 420.0 && rng.random_bool(0.1);
        } else {
            // Scavenge: head for the closest upgrade in sight, otherwise roam.
            let upgrade = items
                .iter()
                .filter(|(_, pickup)| auto_wants(&pickup.item, vitals, loadout))
                .map(|(p, _)| p.0)
                .filter(|p| p.distance(pos.0) < SIGHT * 1.3 && (safe(*p) || vitals.hp > 70.0))
                .min_by(|a, b| a.distance(pos.0).total_cmp(&b.distance(pos.0)));
            if bot.stuck >= 12
                || bot.goal.distance(pos.0) < 40.0
                || !safe(bot.goal)
                || (upgrade.is_none() && rng.random_bool(0.01))
            {
                let bound = ARENA_HALF * 0.9;
                for _ in 0..8 {
                    bot.goal = Vec2::new(
                        rng.random_range(-bound.x..bound.x),
                        rng.random_range(-bound.y..bound.y),
                    );
                    if safe(bot.goal) {
                        break;
                    }
                }
                bot.stuck = 0;
            }
            let goal = upgrade.unwrap_or(bot.goal);
            let dir = next_waypoint(&nav.0, &arena, pos.0, goal, PLAYER_RADIUS) - pos.0;
            steer(&mut input, dir);
            input.aim = dir.normalize_or(Vec2::X);
        }
        current.0 = input;
    }
}
