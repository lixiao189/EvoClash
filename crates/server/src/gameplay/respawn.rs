//! Respawning the dead: where a life starts (by match phase) and what it carries in.

use evoclash_game::sim::PLAYER_RADIUS;

use super::*;
use crate::rng::SimRng;

/// First 3 minutes (more than 5:00 left): the old edge spawn, farthest from the living.
const EARLY_UNTIL: u32 = 3 * 60 * TICKS_PER_SEC;
/// 6:00 onward: street points inside this radius of the bunker.
const LATE_FROM: u32 = 6 * 60 * TICKS_PER_SEC;
/// 3:00 to 6:00: a spawn counts only when the nearest living player is at least this far.
const MID_CLEAR: f32 = 480.0;
const LATE_RADIUS: f32 = 480.0;
const LATE_CLEAR: f32 = 300.0;
/// Fixed grid. No RNG, so a replay that respawns on the same tick lands on the same cell.
const LATE_STEP: i32 = 40;

fn separation_sq(p: Vec2, alive: &[Vec2]) -> f32 {
    alive
        .iter()
        .map(|q| p.distance_squared(*q))
        .fold(f32::MAX, f32::min)
}

fn farther(a: Vec2, b: Vec2, alive: &[Vec2]) -> Vec2 {
    match separation_sq(a, alive).total_cmp(&separation_sq(b, alive)) {
        std::cmp::Ordering::Greater => a,
        std::cmp::Ordering::Less => b,
        std::cmp::Ordering::Equal => {
            if (a.x.to_bits(), a.y.to_bits()) <= (b.x.to_bits(), b.y.to_bits()) {
                a
            } else {
                b
            }
        }
    }
}

fn farthest(points: &[Vec2], alive: &[Vec2]) -> Option<Vec2> {
    points.iter().copied().reduce(|a, b| farther(a, b, alive))
}

fn closest_to_center(points: &[Vec2], n: usize) -> Vec<Vec2> {
    let mut idx: Vec<usize> = (0..points.len()).collect();
    idx.sort_by(|&a, &b| {
        points[a]
            .length_squared()
            .total_cmp(&points[b].length_squared())
            .then(a.cmp(&b))
    });
    idx.truncate(n.min(points.len()));
    idx.into_iter().map(|i| points[i]).collect()
}

fn pick_cleared(points: &[Vec2], alive: &[Vec2], clear: f32) -> Option<Vec2> {
    let clear_sq = clear * clear;
    let far: Vec<Vec2> = points
        .iter()
        .copied()
        .filter(|p| separation_sq(*p, alive) >= clear_sq)
        .collect();
    let pool: &[Vec2] = if far.is_empty() { points } else { &far };
    farthest(pool, alive)
}

fn on_street(map: &ArenaMap, p: Vec2, obstacles: &[(Vec2, f32)]) -> bool {
    if map.building_at(p).is_some() || map.in_hazard(p) {
        return false;
    }
    if map.resolve_circle(p, PLAYER_RADIUS) != p {
        return false;
    }
    obstacles.iter().all(|(c, r)| {
        let min = r + PLAYER_RADIUS;
        p.distance_squared(*c) >= min * min
    })
}

fn disk_points() -> Vec<Vec2> {
    let n = (LATE_RADIUS / LATE_STEP as f32) as i32;
    let radius_sq = LATE_RADIUS * LATE_RADIUS;
    let mut points = Vec::new();
    for iy in -n..=n {
        for ix in -n..=n {
            let p = Vec2::new((ix * LATE_STEP) as f32, (iy * LATE_STEP) as f32);
            if p.length_squared() <= radius_sq {
                points.push(p);
            }
        }
    }
    points
}

/// Playing respawn spot. `elapsed_ticks` is time since Playing started.
fn pick_spawn(
    map: &ArenaMap,
    alive: &[Vec2],
    elapsed_ticks: u32,
    obstacles: &[(Vec2, f32)],
) -> Vec2 {
    if elapsed_ticks >= LATE_FROM {
        let disk = disk_points();
        let street: Vec<Vec2> = disk
            .iter()
            .copied()
            .filter(|p| on_street(map, *p, obstacles))
            .collect();
        let clear_sq = LATE_CLEAR * LATE_CLEAR;
        let cleared: Vec<Vec2> = street
            .iter()
            .copied()
            .filter(|p| separation_sq(*p, alive) >= clear_sq)
            .collect();
        if let Some(spot) = farthest(&cleared, alive) {
            return spot;
        }
        // Nothing on the street is 300 clear: farthest grid point inside the disk.
        return farthest(&disk, alive).unwrap_or(Vec2::ZERO);
    }
    if elapsed_ticks < EARLY_UNTIL {
        return farthest(&map.spawn_points, alive).unwrap_or(Vec2::ZERO);
    }
    let inner = closest_to_center(&map.spawn_points, 4);
    pick_cleared(&inner, alive, MID_CLEAR).unwrap_or(Vec2::ZERO)
}

/// Bring the dead back. Playing: only after Deploy / DeployLast (`deploy_asap`); an
/// unconfirmed human sticks at `respawn_in == 1` until `deploy_in` hits 0, which sends
/// them with DeployLast. Bots are unchanged (their `deploy_in` stays 0). Lobby: the 2s
/// timer counts down to 0 on its own. Whatever was bought in the shop is equipped on the spot.
pub fn respawn(
    arena: Res<Arena>,
    obstacles: Option<Res<Obstacles>>,
    tick: Res<SimTick>,
    state: Query<&MatchState>,
    rng: Option<Res<SimRng>>,
    mut players: Query<(
        Entity,
        &mut Pos,
        &mut Vitals,
        &mut Dash,
        &mut PosHistory,
        &mut Loadout,
        &mut Evolution,
        &mut SkillState,
        &mut Wallet,
        &mut LifeLog,
        Option<&Player>,
    )>,
) {
    if state.iter().any(|s| s.phase == Phase::Recovering) {
        return;
    }
    let lobby = state.iter().any(|s| s.phase == Phase::Lobby);
    let playing = state.iter().any(|s| s.phase == Phase::Playing);
    let mut alive: Vec<(Entity, Vec2)> = players
        .iter()
        .filter(|(_, _, v, ..)| v.alive)
        .map(|(e, p, ..)| (e, p.0))
        .collect();
    let mut dead: Vec<(u64, Entity)> = players
        .iter()
        .filter(|(_, _, v, ..)| !v.alive)
        .map(|(entity, _, _, _, _, _, _, _, _, _, player)| {
            (player.map(|p| p.client_id).unwrap_or(u64::MAX), entity)
        })
        .collect();
    dead.sort_unstable_by_key(|(id, _)| *id);
    for (_, entity) in dead {
        let Ok((
            entity,
            mut pos,
            mut vitals,
            mut dash,
            mut history,
            mut loadout,
            mut evo,
            mut skills,
            mut wallet,
            mut life,
            player,
        )) = players.get_mut(entity)
        else {
            continue;
        };
        if vitals.alive {
            continue;
        }
        if playing {
            let human = player.is_some_and(|p| !p.bot);
            if human && vitals.deploy_in > 0 {
                vitals.deploy_in -= 1;
                if vitals.deploy_in == 0 && !life.deploy_asap {
                    let changed = crate::evolution::handle_command(
                        &PlayerCommand::DeployLast,
                        &mut evo,
                        &mut wallet,
                        &mut vitals,
                        &mut life,
                        true,
                    );
                    if changed {
                        crate::evolution::apply_evolution(&evo, &mut skills, &mut vitals);
                    }
                }
            }
            if life.deploy_asap && can_deploy(vitals.respawn_in) {
                vitals.respawn_in = 0;
            } else if vitals.respawn_in > 1 {
                vitals.respawn_in -= 1;
            }
        } else {
            // Lobby (and any other non-playing phase): ignore the confirm flag.
            vitals.respawn_in = vitals.respawn_in.saturating_sub(1);
        }
        if vitals.respawn_in > 0 {
            continue;
        }
        // Lobby: a random spawn. Playing: early edge, then the inner four, then the streets
        // by the bunker. Anything else (Starting) keeps the old farthest edge spawn.
        let living: Vec<Vec2> = alive
            .iter()
            .filter(|(e, _)| *e != entity)
            .map(|(_, q)| *q)
            .collect();
        let spot = if lobby && !arena.spawn_points.is_empty() {
            let index = rng.as_ref().map_or(0, |rng| {
                rng.with(false, crate::rng::Stream::Loot, |rng| {
                    use rand::Rng;
                    rng.random_range(0..arena.spawn_points.len())
                })
            });
            arena.spawn_points[index]
        } else if playing {
            let elapsed = state
                .iter()
                .find(|s| s.phase == Phase::Playing)
                .map_or(0, |s| MATCH_TICKS.saturating_sub(s.ticks_left));
            let blocks = obstacles.as_ref().map(|o| o.0.as_slice()).unwrap_or(&[]);
            pick_spawn(&arena, &living, elapsed, blocks)
        } else {
            farthest(&arena.spawn_points, &living).unwrap_or(Vec2::ZERO)
        };
        pos.0 = spot;
        alive.push((entity, spot));
        history.0.clear();
        *dash = Dash::default();
        *vitals = Vitals {
            hp: evo.max_hp(),
            max_hp: evo.max_hp(),
            shield: SPAWN_SHIELD_TICKS,
            ..default()
        };
        for item in wallet.take_cart() {
            take(item, &mut vitals, &mut loadout, &evo);
        }
        // Death already wrote a fresh log; revival only stamps this life and drops the confirm.
        life.born = tick.0;
        life.deploy_asap = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner4(map: &ArenaMap) -> Vec<Vec2> {
        let mut idx: Vec<usize> = (0..map.spawn_points.len()).collect();
        idx.sort_by(|&a, &b| {
            map.spawn_points[a]
                .length_squared()
                .total_cmp(&map.spawn_points[b].length_squared())
                .then(a.cmp(&b))
        });
        idx.truncate(4);
        idx.into_iter().map(|i| map.spawn_points[i]).collect()
    }

    #[test]
    fn playing_spawns_change_at_three_and_six_minutes() {
        let map = ArenaMap::generate(1);
        let inner = inner4(&map);
        assert_eq!(inner.len(), 4);
        let open = inner[0];
        let campers = vec![inner[1], inner[2], inner[3]];

        let early = pick_spawn(&map, &campers, EARLY_UNTIL - 1, &[]);
        let mid = pick_spawn(&map, &campers, EARLY_UNTIL, &[]);
        let still_mid = pick_spawn(&map, &campers, LATE_FROM - 1, &[]);
        assert_eq!(mid, open, "3:00 picks the inner spawn clear of the living");
        assert_eq!(still_mid, open);
        assert!(
            map.spawn_points.contains(&early),
            "before 3:00 a respawn is still an edge spawn"
        );
        assert_ne!(early, open);

        let crowded: Vec<Vec2> = inner.iter().map(|p| *p + Vec2::new(100.0, 0.0)).collect();
        let fallback = pick_spawn(&map, &crowded, EARLY_UNTIL, &[]);
        let expected = inner
            .iter()
            .copied()
            .reduce(|a, b| farther(a, b, &crowded))
            .unwrap();
        assert_eq!(fallback, expected);
        assert!(separation_sq(fallback, &crowded) < MID_CLEAR * MID_CLEAR);

        let late_alive = [Vec2::new(2000.0, 1200.0)];
        let late = pick_spawn(&map, &late_alive, LATE_FROM, &[]);
        assert!(late.length_squared() <= LATE_RADIUS * LATE_RADIUS);
        assert!(on_street(&map, late, &[]));
        assert!(separation_sq(late, &late_alive) >= LATE_CLEAR * LATE_CLEAR);
        assert_eq!(
            late,
            pick_spawn(&map, &late_alive, 8 * 60 * TICKS_PER_SEC, &[])
        );
        assert!(
            !map.spawn_points.contains(&late),
            "6:00 leaves the edge spawn list"
        );

        let blocked = pick_spawn(&map, &late_alive, LATE_FROM, &[(late, 30.0)]);
        assert_ne!(blocked, late);
        assert!(on_street(&map, blocked, &[(late, 30.0)]));
        assert!(blocked.length_squared() <= LATE_RADIUS * LATE_RADIUS);
    }
}
