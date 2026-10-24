//! Noticing prey, wandering, chasing and raider tactics.

use super::*;

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

pub(super) fn wrap_angle(a: f32) -> f32 {
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

pub(super) fn wander(brain: &mut Brain, rng: &mut impl Rng, step: u16) -> Vec2 {
    if brain.turn_in == 0 {
        brain.heading = Vec2::from_angle(rng.random_range(0.0..TAU));
        brain.turn_in = rng.random_range(40..160);
    }
    brain.turn_in = brain.turn_in.saturating_sub(step);
    brain.heading
}

#[allow(clippy::type_complexity)]
pub(super) fn roam(
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
pub(super) fn raider_think(
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
