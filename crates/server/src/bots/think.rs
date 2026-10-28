use super::build::arrange;
use super::engage::{
    self, MIND_RETREAT, RAIDER_TIER, Seen, choice, dangerous_monster, melee_fauna, notice,
    ranged_fauna, scavenging_now,
};
use super::scavenge::{
    self, Spec, choose, cover_point, depart_dir, follow, gather, hazard_exit, hot_barrel,
    retreat_point, shot_hits_barrel,
};
use super::*;
use crate::evolution::Recovery;
use crate::gameplay::{MATCH_TICKS, SimTick};
use evoclash_game::map::PropKind;

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
        // No separate use key: walking over a medkit is what spends it.
        Item::Medkit => Some(if vitals.hp < engage::HURT_FRAC * vitals.max_hp {
            0.35
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
    tick: Res<SimTick>,
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
        &LifeLog,
        Option<&Recovery>,
        &Player,
    )>,
    others: Query<
        (Entity, &Pos, &Vitals, Option<&Monster>, Has<Human>),
        Or<(With<Player>, With<Monster>)>,
    >,
    humans: Query<(Entity, &Loadout), (With<Player>, Without<Bot>)>,
    items: Query<(&Pos, &Pickup, Option<&BagContents>)>,
    props: Query<(Entity, &Prop, &Pos, &Vitals)>,
    faces: Query<&MonsterState>,
    state: Query<&MatchState>,
    rng: Res<SimRng>,
    mut pulse: Option<ResMut<BotPulse>>,
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
        let mut gear_tier: HashMap<Entity, u8> = HashMap::new();
        for (entity, _, _, _, _, loadout, ..) in &bots {
            gear_tier.insert(entity, loadout.weapon().spec().tier());
        }
        for (entity, loadout) in &humans {
            gear_tier.insert(entity, loadout.weapon().spec().tier());
        }
        let barrels: Vec<Vec2> = props
            .iter()
            .filter(|(_, prop, _, v)| v.alive && prop.kind == PropKind::Barrel)
            .map(|(_, _, p, _)| p.0)
            .collect();
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
                life,
                recovery,
                _,
            )) = bots.get_mut(entity)
            else {
                continue;
            };
            if !vitals.alive {
                current.0 = PlayerInput::default();
                bot.target = None;
                bot.crack = None;
                bot.mind = 0;
                bot.mind_until = 0;
                bot.hide_since = 0;
                bot.focus = None;
                continue;
            }
            let sealed = vitals.armor == ArmorKind::PowerArmor && vitals.armor_points > 0.0;
            let safe = |p: Vec2| sealed || !arena.in_hazard(p);
            // Stuck on a wall: pick somewhere else.
            if pos.0.distance_squared(bot.last_pos) < 0.25 && current.0.move_dir() != Vec2::ZERO {
                bot.stuck += 1;
            } else {
                bot.stuck = 0;
            }
            bot.last_pos = pos.0;

            if !sealed
                && vitals.hp < engage::RAD_LEAVE * vitals.max_hp
                && let Some(exit) = hazard_exit(&arena.0.hazards, pos.0)
            {
                let mut input = PlayerInput::default();
                let dir = next_waypoint(&nav.0, &arena, pos.0, exit, PLAYER_RADIUS) - pos.0;
                steer(&mut input, dir, &arena, &obstacles.0, pos.0);
                input.aim = dir.normalize_or(Vec2::X);
                current.0 = input;
                bot.target = None;
                bot.think_in = 0;
                continue;
            }
            if let Some(barrel) = hot_barrel(pos.0, &barrels) {
                let mut input = PlayerInput::default();
                steer(&mut input, pos.0 - barrel, &arena, &obstacles.0, pos.0);
                input.aim = (pos.0 - barrel).normalize_or(Vec2::X);
                current.0 = input;
                bot.think_in = 0;
                continue;
            }

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
            if let Some(gun) = loadout.guns().max_by_key(|gun| gun.spec().tier()) {
                bot.last_gun = gun;
            }
            gear_tier.insert(entity, weapon.spec().tier());

            let previous = bot.target;
            let elapsed = state
                .iter()
                .find(|s| s.phase == Phase::Playing)
                .map(|s| MATCH_TICKS.saturating_sub(s.ticks_left))
                .unwrap_or(0);
            let stick = weapon == WeaponKind::Stick;
            let scavenging = scavenging_now(
                playing,
                elapsed,
                tick.0.saturating_sub(life.born),
                stick,
                bot.aggression,
            );
            let cornered = bot.stuck >= 12;
            let me = engage::Side {
                tier: weapon.spec().tier(),
                hp: vitals.hp,
                armor: vitals.armor_points,
            };
            let hp_frac = vitals.hp / vitals.max_hp.max(1.0);
            let seen: Vec<Seen> = others
                .iter()
                .filter(|(e, p, v, _, human)| {
                    *e != entity
                        && v.alive
                        && v.shield == 0
                        && seen_from(v, p.0, pos.0)
                        && !(lobby && *human && !aggressors.0.contains(e))
                })
                .filter_map(|(e, p, v, monster, human)| {
                    let dist = p.0.distance(pos.0);
                    if dist >= SIGHT || !arena.line_of_sight(pos.0, p.0) {
                        return None;
                    }
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
                    let kind = monster.map(|m| m.kind);
                    let melee = kind.is_some_and(melee_fauna);
                    let ghoul = kind == Some(MonsterKind::Ghoul);
                    let (tier, dangerous, armed) = match kind {
                        Some(MonsterKind::Raider) if !stick => (RAIDER_TIER, false, true),
                        Some(k) => (
                            0,
                            dangerous_monster(k, monster.is_some_and(|m| m.elite)),
                            false,
                        ),
                        None => (gear_tier.get(&e).copied().unwrap_or(0), false, false),
                    };
                    Some(notice(engage::Notice {
                        me,
                        them: engage::Side {
                            tier,
                            hp: v.hp,
                            armor: v.armor_points,
                        },
                        player: monster.is_none(),
                        dist,
                        aggressor: aggressors.0.contains(&e),
                        dangerous,
                        armed,
                        stick,
                        aggression: bot.aggression,
                        scavenging,
                        cornered,
                        hp_frac,
                        playing,
                        melee,
                        ghoul,
                        crawler: kind == Some(MonsterKind::Crawler),
                        entity: e,
                        pos: p.0,
                        armor: v.armor_points,
                        score: dist * weight,
                        kind,
                    }))
                })
                .collect();
            let picked = choice(&seen);
            let focus = seen.iter().find(|s| Some(s.entity) == bot.focus);
            let just_hit =
                recovery.is_some_and(|r| r.since_hurt == 0) && tick.0.saturating_sub(life.born) > 8;
            let range = engage_range(weapon);
            let dog = seen
                .iter()
                .filter(|s| {
                    matches!(s.kind, Some(MonsterKind::Roach | MonsterKind::Dog)) && s.dist < 220.0
                })
                .min_by(|a, b| a.dist.total_cmp(&b.dist));
            let held = engage::settle(
                &picked,
                focus,
                me,
                bot.mind,
                bot.mind_until,
                bot.hide_since,
                tick.0,
                just_hit,
                focus.is_some_and(|s| arena.line_of_sight(pos.0, s.pos)),
                focus.is_some_and(|s| s.dist < range),
                cornered,
                pos.0,
                dog,
            );
            bot.mind_until = held.until;
            bot.hide_since = held.hide_since;
            let mind = held.mind;
            let leaving = held.leaving;
            let fighting = held.fighting;
            let avoiding = held.avoiding;
            let blocked = others.iter().any(|(e, p, v, monster, _)| {
                e != entity
                    && v.alive
                    && p.0.distance(pos.0) < scavenge::SAFE_THREAT
                    && arena.line_of_sight(pos.0, p.0)
                    && monster.is_none_or(|m| dangerous_monster(m.kind, m.elite))
            });
            let mut threat_dist = None;
            let mut spec = Spec {
                at: pos.0,
                our_tier: me.tier,
                smash_range: engage_range(weapon) * 0.85,
                allow_safe: false,
                hurt: true,
                sticky: None,
                sealed,
                hazard: &|p| arena.in_hazard(p),
                barrels: &barrels,
            };

            if let Some((who, enemy_pos, dist)) = leaving.or(avoiding) {
                bot.target = None;
                bot.focus = Some(who);
                bot.locked_for = 0;
                bot.crack = None;
                threat_dist = Some(dist);
                let to_enemy = (enemy_pos - pos.0).normalize_or(Vec2::X);
                input.aim = to_enemy;
                input.fire = false;
                input.throw = false;
                input.dash = dist < 320.0;
                let foe = seen.iter().find(|s| s.entity == who);
                let spitter = seen
                    .iter()
                    .filter(|s| {
                        s.kind == Some(MonsterKind::Spitter) && arena.line_of_sight(pos.0, s.pos)
                    })
                    .min_by(|a, b| a.dist.total_cmp(&b.dist));
                let hide_at = spitter.map(|s| s.pos).unwrap_or(enemy_pos);
                let break_los =
                    spitter.is_some() || foe.is_none_or(|s| s.kind.is_none_or(ranged_fauna));
                let cover = break_los
                    .then(|| cover_point(&arena.0, pos.0, hide_at))
                    .flatten();
                if let Some(cover) = cover {
                    let dir = next_waypoint(&nav.0, &arena, pos.0, cover, PLAYER_RADIUS) - pos.0;
                    steer(&mut input, dir, &arena, &obstacles.0, pos.0);
                } else if break_los {
                    let away = (pos.0 - hide_at).normalize_or(Vec2::Y).perp();
                    steer(&mut input, away, &arena, &obstacles.0, pos.0);
                } else {
                    let desires = gather(&spec, &items, &props, |item, bag| {
                        interest(item, bag, vitals, &loadout, evo)
                    });
                    let goal = choose(&desires, Some(enemy_pos), |p| {
                        !arena.line_of_sight(enemy_pos, p)
                    })
                    .map(|d| d.pos)
                    .unwrap_or_else(|| retreat_point(pos.0, enemy_pos));
                    steer(
                        &mut input,
                        depart_dir(to_enemy, goal - pos.0),
                        &arena,
                        &obstacles.0,
                        pos.0,
                    );
                }
                if mind == MIND_RETREAT
                    && just_hit
                    && dist < range
                    && !shot_hits_barrel(pos.0, enemy_pos, &barrels)
                {
                    input.aim = aim_toward(to_enemy, bot.aim_error, &mut rng);
                    input.fire = true;
                }
            } else if let Some((enemy_e, enemy_pos, enemy_armor, dist, foe_kind)) = fighting {
                bot.focus = Some(enemy_e);
                bot.crack = None;
                bot.target = Some(enemy_e);
                if bot.target != previous {
                    bot.locked_for = 0;
                }
                bot.locked_for = bot.locked_for.saturating_add(1);
                threat_dist = Some(dist);
                super::fight::press(
                    &mut input,
                    weapon,
                    stick,
                    pos.0,
                    enemy_pos,
                    dist,
                    foe_kind,
                    faces.get(enemy_e).ok().map(|s| s.facing),
                    cornered,
                    bot.locked_for,
                    bot.reaction_thinks,
                    bot.aim_error,
                    &mut bot.strafe,
                    enemy_armor,
                    loadout.emps,
                    vitals.stealth > 0 && vitals.hp < 0.5 * vitals.max_hp,
                    skills.skill_b.is_some() && skills.cd_b == 0,
                    &barrels,
                    |p| arena.in_hazard(p),
                    &nav.0,
                    &arena,
                    &obstacles.0,
                    rng,
                );
            } else {
                bot.target = None;
                bot.locked_for = 0;
                bot.focus = None;
                spec.allow_safe = !blocked && playing;
                spec.hurt = hp_frac < engage::HURT_FRAC;
                spec.sticky = bot.crack;
                let desires = gather(&spec, &items, &props, |item, bag| {
                    interest(item, bag, vitals, &loadout, evo)
                });
                let picked = choose(&desires, None, |_| false);
                let (crack, stuck) = follow(
                    &mut input,
                    &nav.0,
                    &arena,
                    &obstacles.0,
                    pos.0,
                    picked,
                    None,
                    bot.stuck,
                    &mut bot.goal,
                    safe,
                    rng,
                );
                bot.crack = crack;
                bot.stuck = stuck;
            }
            let near = threat_dist.is_some_and(|dist| dist < SKILL_RANGE);
            let hurt = vitals.hp < 0.5 * vitals.max_hp;
            input.skill_a = skills.cd_a == 0
                && skills.skill_a.is_some_and(|skill| {
                    near || (hurt && matches!(skill, Skill::EnergyShield | Skill::Stealth))
                });
            if let Some(pulse) = pulse.as_mut() {
                if mind == 2 && bot.mind != 2 {
                    pulse.retreats += 1;
                }
                if mind == 1 && bot.mind != 1 {
                    pulse.avoids += 1;
                }
            }
            bot.mind = mind;
            current.0 = input;
        }
    });
}
