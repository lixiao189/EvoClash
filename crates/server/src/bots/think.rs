use super::build::arrange;
use super::*;

/// How far a weapon is worth firing from.
pub fn engage_range(weapon: WeaponKind) -> f32 {
    match weapon.arm() {
        Arm::Melee(club) => club.reach() + 20.0,
        Arm::Ranged(gun) => gun.range() * 0.8,
    }
}

/// How keen a bot is on a pickup, as a distance multiplier (lower is keener), or `None` if it
/// isn't worth the walk.
pub(super) fn interest(
    item: &Item,
    bag: Option<&BagContents>,
    vitals: &Vitals,
    loadout: &Loadout,
    evo: &Evolution,
) -> Option<f32> {
    if let Some(bag) = bag {
        return bag
            .0
            .iter()
            .filter_map(|i| interest(i, None, vitals, loadout, evo))
            .min_by(f32::total_cmp)
            .map(|w| w * 0.8);
    }
    if !auto_wants(item, vitals, loadout, evo) {
        return None;
    }
    match *item {
        // Rounds only matter for a gun you carry, and most when nothing can fire.
        Item::Ammo { kind, .. } => {
            let feeds = loadout.guns().any(|g| g.ammo() == Some(kind));
            let armed = loadout.weapon().ranged().is_some();
            feeds.then_some(if armed { 0.7 } else { 0.4 })
        }
        Item::Weapon(kind) => Some(if loadout.best_tier() < kind.spec().tier() {
            0.6
        } else {
            1.0
        }),
        _ => Some(1.0),
    }
}

/// Aim at a target with a fixed personal error plus a little wobble, so nobody laser-beams you.
pub fn aim_toward(to_enemy: Vec2, aim_error: f32, rng: &mut impl Rng) -> Vec2 {
    let wobble = rng.random_range(-0.05..0.05);
    Vec2::from_angle(to_enemy.to_angle() + aim_error + wobble)
}

/// Inputs only move in 8 directions: take the one closest to `dir` whose next stretch is free
/// of walls and props, so a bot slides around corners instead of grinding into them.
pub(super) fn steer(
    input: &mut PlayerInput,
    dir: Vec2,
    arena: &Arena,
    obstacles: &[(Vec2, f32)],
    at: Vec2,
) {
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
pub(super) fn think(
    arena: Res<Arena>,
    obstacles: Res<Obstacles>,
    nav: Option<Res<Nav>>,
    mut bots: Query<(
        Entity,
        &mut Bot,
        &mut CurrentInput,
        &Pos,
        &Vitals,
        &mut Loadout,
        &Evolution,
        &SkillState,
        &Aggressors,
        &Player,
    )>,
    others: Query<
        (Entity, &Pos, &Vitals, Option<&Monster>, Has<Human>),
        Or<(With<Player>, With<Monster>)>,
    >,
    items: Query<(&Pos, &Pickup, Option<&BagContents>)>,
    state: Query<&MatchState>,
    rng: Res<SimRng>,
) {
    let Some(nav) = nav else {
        return;
    };
    if state.iter().any(|s| s.phase == Phase::Recovering) {
        return;
    }
    let lobby = state.iter().any(|s| s.phase == Phase::Lobby);
    let playing = state.iter().any(|s| s.phase == Phase::Playing);
    rng.with(playing, Stream::Bots, |mut rng| {
        let mut claims: HashMap<Entity, u32> = HashMap::new();
        for (_, bot, ..) in &bots {
            if let Some(target) = bot.target {
                *claims.entry(target).or_default() += 1;
            }
        }
        let mut order: Vec<(u64, Entity)> = bots
            .iter()
            .map(|(entity, .., player)| (player.client_id, entity))
            .collect();
        order.sort_unstable_by_key(|(id, _)| *id);
        for (_, entity) in order {
            let Ok((
                entity,
                mut bot,
                mut current,
                pos,
                vitals,
                mut loadout,
                evo,
                skills,
                aggressors,
                _,
            )) = bots.get_mut(entity)
            else {
                continue;
            };
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
            let mut arranged = *loadout;
            arrange(&mut arranged);
            loadout.set_if_neq(arranged);
            let weapon = loadout.weapon();

            let previous = bot.target;
            let enemy = others
                .iter()
                .filter(|(e, p, v, _, human)| {
                    *e != entity
                    && v.alive
                    && v.shield == 0
                    // Stealthed enemies are only noticed up close.
                    && seen_from(v, p.0, pos.0)
                    // Lobby bots path, loot and fight each other, and only retaliate.
                    && !(lobby && *human && !aggressors.0.contains(e))
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
                .min_by(|a, b| {
                    a.4.total_cmp(&b.4).then(
                        a.1.x
                            .to_bits()
                            .cmp(&b.1.x.to_bits())
                            .then(a.1.y.to_bits().cmp(&b.1.y.to_bits())),
                    )
                });

            bot.target = enemy.map(|(e, ..)| e);
            if bot.target != previous {
                bot.locked_for = 0;
            }
            bot.locked_for = bot.locked_for.saturating_add(1);

            if let Some((_, enemy_pos, enemy_armor, dist, _)) = enemy {
                let to_enemy = (enemy_pos - pos.0).normalize_or(Vec2::X);
                input.aim = aim_toward(to_enemy, bot.aim_error, &mut rng);
                let range = engage_range(weapon);
                if rng.random_bool(0.03) {
                    bot.strafe = -bot.strafe;
                }
                let strafe = to_enemy.perp() * bot.strafe;
                let dir = if weapon.melee().is_some() || dist > range * 0.85 {
                    let waypoint = next_waypoint(&nav.0, &arena, pos.0, enemy_pos, PLAYER_RADIUS);
                    (waypoint - pos.0).normalize_or_zero() + strafe * 0.3
                } else if dist < range * 0.35 {
                    -to_enemy + strafe
                } else {
                    strafe
                };
                steer(&mut input, dir, &arena, &obstacles.0, pos.0);
                input.fire =
                    bot.locked_for > bot.reaction_thinks && dist < range && rng.random_bool(0.85);
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
                    .filter(|(p, ..)| {
                        p.0.distance(pos.0) < SIGHT * 1.3 && (safe(p.0) || vitals.hp > 70.0)
                    })
                    .filter_map(|(p, pickup, bag)| {
                        interest(&pickup.item, bag, vitals, &loadout, evo)
                            .map(|w| (p.0, p.0.distance(pos.0) * w))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(p, _)| p);
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
    });
}
