//! Scavenger bots keep small lobbies lively and step aside as humans join.

use std::collections::HashMap;

use bevy::prelude::*;
use evoclash_game::evolution::{EvoPath, Evolution, Perk, Skill, SkillState};
use evoclash_game::items::{Arm, ArmorKind, WeaponKind};
use evoclash_game::map::ARENA_HALF;
use evoclash_game::nav::{NavGrid, clear_ahead, next_waypoint};
use evoclash_game::protocol::*;
use evoclash_game::shop::{PlayerCommand, ShopItem, Wallet, can_deploy};
use evoclash_game::sim::{Arena, DT, Obstacles, PLAYER_RADIUS, PLAYER_SPEED};
use rand::Rng;

use crate::combat::auto_wants;
use crate::config::ServerConfig;
use crate::connections::Human;
use crate::evolution::{LifeLog, apply_evolution, handle_command};
use crate::gameplay::{CurrentInput, SimSet, spawn_combatant};
use evoclash_game::evolution::seen_from;

/// Bots fire their skills at enemies this close.
const SKILL_RANGE: f32 = 500.0;

const SIGHT: f32 = 820.0;
const THINK_EVERY: u16 = 4;
/// How far ahead a bot checks its heading for walls: a bit more than it covers between thinks.
const PROBE: f32 = PLAYER_SPEED * DT * (THINK_EVERY + 2) as f32;
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
    /// Evolution path whose build order this bot spends its points on.
    pub path: EvoPath,
}

/// Walkable grid for the current arena; raiders path with it too.
#[derive(Resource)]
pub struct Nav(pub NavGrid);

/// How far a weapon is worth firing from.
pub fn engage_range(weapon: WeaponKind) -> f32 {
    match weapon.arm() {
        Arm::Melee(club) => club.reach() + 20.0,
        Arm::Ranged(gun) => gun.range() * 0.8,
    }
}

/// Aim at a target with a fixed personal error plus a little wobble, so nobody laser-beams you.
pub fn aim_toward(to_enemy: Vec2, aim_error: f32, rng: &mut impl Rng) -> Vec2 {
    let wobble = rng.random_range(-0.05..0.05);
    Vec2::from_angle(to_enemy.to_angle() + aim_error + wobble)
}

pub struct BotsPlugin;

impl Plugin for BotsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (rebuild_nav, balance_bots, prepare, think)
                .chain()
                .in_set(SimSet::Input),
        );
    }
}

/// Each match has a new layout, so the grid is rebuilt whenever the arena changes.
fn rebuild_nav(mut commands: Commands, arena: Res<Arena>, nav: Option<Res<Nav>>) {
    if nav.is_none() || arena.is_changed() {
        commands.insert_resource(Nav(NavGrid::new(&arena, PLAYER_RADIUS + 2.0)));
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
            path: EvoPath::ALL[rand::rng().random_range(0..EvoPath::ALL.len())],
        });
    } else if bot_count > wanted
        && let Some((entity, player)) = bots.iter().next()
    {
        info!("bot {} leaves to make room", player.name);
        commands.entity(entity).despawn();
    }
}

/// Where each path's nine points go, in order: skill A by the 2nd point, skill B by the 5th.
pub fn build_order(path: EvoPath) -> [Perk; 9] {
    use Perk::*;
    match path {
        EvoPath::Mutant => [
            Constitution,
            Claws,
            Regeneration,
            Bloodthirst,
            Constitution,
            Claws,
            Bloodthirst,
            Constitution,
            Regeneration,
        ],
        EvoPath::Cyborg => [
            Overclock,
            CompositeArmor,
            SelfRepair,
            Overclock,
            CompositeArmor,
            Overclock,
            SelfRepair,
            CompositeArmor,
            EnergyFocus,
        ],
        EvoPath::Scavenger => [
            LightFoot,
            AmmoRecycler,
            Lockpick,
            LightFoot,
            EmpExpert,
            AmmoRecycler,
            LightFoot,
            AmmoRecycler,
            EmpExpert,
        ],
    }
}

/// The next perk `path`'s build order still wants.
fn next_perk(evo: &Evolution, path: EvoPath) -> Option<Perk> {
    let mut want = [0u8; 12];
    build_order(path).into_iter().find(|perk| {
        want[perk.index()] += 1;
        evo.rank(*perk) < want[perk.index()]
    })
}

/// Shop picks, best first.
const GUNS: [ShopItem; 3] = [ShopItem::Rifle, ShopItem::PipeShotgun, ShopItem::Bow];
const ARMORS: [ShopItem; 2] = [ShopItem::Kevlar, ShopItem::ScrapArmor];

/// A bot's death prep, through the same commands a human sends: spend points on the build,
/// and once it may deploy, buy the best gun, armor and EMPs it can afford and go.
/// Returns true if the perks changed.
pub fn prep_bot(
    path: EvoPath,
    evo: &mut Evolution,
    wallet: &mut Wallet,
    vitals: &mut Vitals,
    life: &mut LifeLog,
) -> bool {
    let mut changed = false;
    while let Some(perk) = next_perk(evo, path) {
        if !handle_command(&PlayerCommand::Spend(perk), evo, wallet, vitals, life) {
            break;
        }
        changed = true;
    }
    if vitals.alive || !can_deploy(vitals.respawn_in) {
        return changed;
    }
    for group in [&GUNS[..], &ARMORS[..]] {
        if let Some(item) = group
            .iter()
            .copied()
            .find(|item| wallet.can_buy(*item, evo))
        {
            handle_command(&PlayerCommand::Buy(item), evo, wallet, vitals, life);
        }
    }
    while wallet.can_buy(ShopItem::Emp, evo) {
        handle_command(
            &PlayerCommand::Buy(ShopItem::Emp),
            evo,
            wallet,
            vitals,
            life,
        );
    }
    handle_command(&PlayerCommand::Deploy, evo, wallet, vitals, life);
    changed
}

fn prepare(
    mut bots: Query<(
        &Bot,
        &mut Evolution,
        &mut Wallet,
        &mut Vitals,
        &mut SkillState,
        &mut LifeLog,
    )>,
) {
    for (bot, mut evo, mut wallet, mut vitals, mut skills, mut life) in &mut bots {
        if prep_bot(bot.path, &mut evo, &mut wallet, &mut vitals, &mut life) {
            apply_evolution(&evo, &mut skills, &mut vitals);
        }
    }
}

/// Inputs only move in 8 directions: take the one closest to `dir` whose next stretch is free
/// of walls and props, so a bot slides around corners instead of grinding into them.
fn steer(input: &mut PlayerInput, dir: Vec2, arena: &Arena, obstacles: &[(Vec2, f32)], at: Vec2) {
    let dir = dir.normalize_or_zero();
    if dir == Vec2::ZERO {
        input.move_x = 0;
        input.move_y = 0;
        return;
    }
    let mut axes: Vec<(i8, i8)> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |y| (x, y)))
        .filter(|&a| a != (0, 0))
        .collect();
    let along = |(x, y): (i8, i8)| Vec2::new(x as f32, y as f32).normalize().dot(dir);
    axes.sort_by(|a, b| along(*b).total_cmp(&along(*a)));
    // Never turn more than 135° away; if everything near is blocked, push on and slide.
    let (x, y) = axes
        .iter()
        .take(7)
        .copied()
        .find(|&(x, y)| {
            let d = Vec2::new(x as f32, y as f32);
            clear_ahead(arena, obstacles, at, d, PLAYER_RADIUS, PROBE)
        })
        .unwrap_or(axes[0]);
    input.move_x = x;
    input.move_y = y;
}

#[allow(clippy::type_complexity)]
fn think(
    arena: Res<Arena>,
    obstacles: Res<Obstacles>,
    nav: Option<Res<Nav>>,
    mut bots: Query<(
        Entity,
        &mut Bot,
        &mut CurrentInput,
        &Pos,
        &Vitals,
        &Loadout,
        &Evolution,
        &SkillState,
    )>,
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
    for (entity, mut bot, mut current, pos, vitals, loadout, evo, skills) in &mut bots {
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
            .filter(|(e, p, v, ..)| {
                *e != entity
                    && v.alive
                    && v.shield == 0
                    // Stealthed enemies are only noticed up close.
                    && seen_from(v, p.0, pos.0)
            })
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
            input.aim = aim_toward(to_enemy, bot.aim_error, &mut rng);
            let range = engage_range(loadout.weapon);
            if rng.random_bool(0.03) {
                bot.strafe = -bot.strafe;
            }
            let strafe = to_enemy.perp() * bot.strafe;
            let dir = if loadout.weapon.melee().is_some() || dist > range * 0.85 {
                let waypoint = next_waypoint(&nav.0, &arena, pos.0, enemy_pos, PLAYER_RADIUS);
                (waypoint - pos.0).normalize_or_zero() + strafe * 0.3
            } else if dist < range * 0.35 {
                -to_enemy + strafe
            } else {
                strafe
            };
            steer(&mut input, dir, &arena, &obstacles.0, pos.0);
            input.fire = bot.locked_for > REACTION_THINKS && dist < range && rng.random_bool(0.85);
            input.dash = dist < 260.0 && rng.random_bool(0.04);
            input.throw =
                loadout.emps > 0 && enemy_armor >= 50.0 && dist < 420.0 && rng.random_bool(0.1);
            // Hurt and hidden: slip away instead of giving yourself up.
            if vitals.stealth > 0 && vitals.hp < 0.5 * vitals.max_hp {
                input.fire = false;
                input.throw = false;
                steer(&mut input, -to_enemy, &arena, &obstacles.0, pos.0);
            }
            input.skill_b = skills.skill_b.is_some() && skills.cd_b == 0 && dist < SKILL_RANGE;
        } else {
            // Scavenge: head for the closest upgrade in sight, otherwise roam.
            let upgrade = items
                .iter()
                .filter(|(_, pickup)| auto_wants(&pickup.item, vitals, loadout, evo))
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
            steer(&mut input, dir, &arena, &obstacles.0, pos.0);
            input.aim = dir.normalize_or(Vec2::X);
        }
        // Skill A: on a nearby target, or to save yourself (shield, stealth) when hurt.
        let near = enemy.is_some_and(|(.., dist, _)| dist < SKILL_RANGE);
        let hurt = vitals.hp < 0.5 * vitals.max_hp;
        input.skill_a = skills.cd_a == 0
            && skills.skill_a.is_some_and(|skill| {
                near || (hurt && matches!(skill, Skill::EnergyShield | Skill::Stealth))
            });
        current.0 = input;
    }
}

#[cfg(test)]
mod tests {
    use evoclash_game::evolution::{MAX_RANK, SKILL_A_RANK, SKILL_B_RANK, xp_to_reach};
    use evoclash_game::shop::RESPAWN_MAX_TICKS;

    use super::*;

    #[test]
    fn build_orders_reach_both_skills_on_time() {
        for path in EvoPath::ALL {
            assert!(build_order(path).iter().all(|p| p.evo_path() == path));
            let mut evo = Evolution::default();
            let (mut wallet, mut life) = (Wallet::default(), LifeLog::default());
            let mut vitals = Vitals::default();
            for level in 2..=10 {
                evo.add_xp(xp_to_reach(level) - evo.xp);
                assert!(prep_bot(
                    path,
                    &mut evo,
                    &mut wallet,
                    &mut vitals,
                    &mut life
                ));
                let spent = level - 1;
                assert_eq!((evo.points, evo.path_rank(path)), (0, spent), "{path:?}");
                if spent == 2 {
                    assert_eq!(evo.path_rank(path), SKILL_A_RANK);
                }
                if spent == 5 {
                    assert_eq!(evo.path_rank(path), SKILL_B_RANK);
                }
            }
            assert!(Perk::ALL.iter().all(|p| evo.rank(*p) <= MAX_RANK));
            assert!(evo.skills().1.is_some());
        }
    }

    #[test]
    fn dead_bots_shop_then_deploy() {
        let mut evo = Evolution::default();
        let mut wallet = Wallet {
            scrap: 160,
            ..default()
        };
        let mut life = LifeLog::default();
        let mut vitals = Vitals {
            alive: false,
            respawn_in: RESPAWN_MAX_TICKS,
            ..default()
        };
        prep_bot(
            EvoPath::Cyborg,
            &mut evo,
            &mut wallet,
            &mut vitals,
            &mut life,
        );
        assert!(wallet.cart.is_empty(), "waits until it may deploy");
        vitals.respawn_in = RESPAWN_MAX_TICKS / 2;
        prep_bot(
            EvoPath::Cyborg,
            &mut evo,
            &mut wallet,
            &mut vitals,
            &mut life,
        );
        assert_eq!(
            wallet.cart,
            vec![
                ShopItem::Rifle,
                ShopItem::Kevlar,
                ShopItem::Emp,
                ShopItem::Emp
            ]
        );
        assert_eq!((wallet.scrap, vitals.respawn_in), (5, 1));
    }
}
