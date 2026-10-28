use super::*;
use evoclash_game::map::ArenaMap;
use rand::SeedableRng;
use rand::rngs::StdRng;

#[test]
fn caps_hold_while_filling() {
    let mut c = Census::default();
    loop {
        let mut spawned = false;
        for kind in MonsterKind::ALL {
            if kind == MonsterKind::Behemoth || !c.wants(kind) {
                continue;
            }
            for slot in 0..spec(kind).group {
                c.add(kind, false, slot == 0);
            }
            spawned = true;
        }
        if !spawned {
            break;
        }
    }
    assert!(c.total <= MAX_LIVE, "{c:?}");
    for kind in MonsterKind::ALL {
        let s = spec(kind);
        let i = kind_index(kind);
        assert!(c.groups[i] <= s.cap, "{kind:?}");
        assert!(c.live[i] <= s.cap * s.group, "{kind:?}");
    }
    let live = |k| c.live[kind_index(k)];
    assert_eq!(live(MonsterKind::Roach), 8);
    assert_eq!(live(MonsterKind::SporePod), 4);
    assert_eq!(live(MonsterKind::Dog), 9);
    assert_eq!(live(MonsterKind::Ghoul), 5);
    assert_eq!(live(MonsterKind::Spitter), 3);
    assert_eq!(live(MonsterKind::Crawler), 2);
    assert_eq!(live(MonsterKind::Raider), 4);
    assert_eq!(live(MonsterKind::Drone), 3);
    assert_eq!(c.groups[kind_index(MonsterKind::Dog)], 3);
    assert_eq!(c.groups[kind_index(MonsterKind::Raider)], 2);
    // Room left for the boss and its summons.
    assert!(c.total + 4 <= MAX_LIVE);
}

#[test]
fn roaches_dogs_and_pods_do_not_replenish_after_two_minutes() {
    let two = 2 * 60 * TICKS_PER_SEC;
    // `populate` reads `ticks_left` before `match_flow` decrements it. Elapsed 2:00 is the
    // first tick whose clock will show 2:00 after that decrement.
    let edge = MATCH_TICKS - two + 1;
    assert!(replenishes(MonsterKind::Roach, edge + 1));
    assert!(!replenishes(MonsterKind::Roach, edge));
    assert!(!replenishes(MonsterKind::Roach, 1));
    assert!(!replenishes(MonsterKind::Dog, edge));
    assert!(!replenishes(MonsterKind::SporePod, edge));
    for kind in [
        MonsterKind::Ghoul,
        MonsterKind::Spitter,
        MonsterKind::Crawler,
        MonsterKind::Raider,
        MonsterKind::Drone,
    ] {
        assert!(replenishes(kind, 1), "{kind:?} still refills in the finale");
    }
}

#[test]
fn census_counts_packs_once() {
    let c = census(
        [
            (MonsterKind::Dog, false, 1),
            (MonsterKind::Dog, false, 1),
            (MonsterKind::Dog, true, 2),
            (MonsterKind::Roach, false, 0),
        ]
        .into_iter(),
    );
    assert_eq!(c.groups[kind_index(MonsterKind::Dog)], 2);
    assert_eq!(c.live[kind_index(MonsterKind::Dog)], 3);
    assert_eq!((c.elites, c.total), (1, 4));
}

#[test]
fn spawn_spots_are_clear() {
    let mut rng = StdRng::seed_from_u64(7);
    let mut found = 0;
    let seeds = 60;
    for seed in 0..seeds {
        let arena = Arena(ArenaMap::generate(seed));
        let players: Vec<Vec2> = arena.spawn_points.iter().step_by(3).copied().collect();
        let obstacles: Vec<(Vec2, f32)> = arena
            .props
            .iter()
            .map(|p| (p.pos, p.kind.radius()))
            .collect();
        for kind in MonsterKind::ALL {
            if kind == MonsterKind::Behemoth {
                continue;
            }
            for _ in 0..4 {
                let Some((p, block)) = pick_spot(&arena, kind, &obstacles, &players, &mut rng)
                else {
                    continue;
                };
                found += 1;
                let r = kind.radius();
                assert_eq!(
                    arena.resolve_circle(p, r),
                    p,
                    "seed {seed} {kind:?} in a wall"
                );
                assert!(p.distance(VAULT_CENTER) > SPAWN_CLEAR_OF_VAULT);
                assert!(block.rect.contains(p) && block_suits(kind, &block));
                assert!(
                    players
                        .iter()
                        .all(|q| q.distance(p) > SPAWN_CLEAR_OF_PLAYERS)
                );
                assert!(obstacles.iter().all(|(c, cr)| c.distance(p) >= cr + r));
                if kind.flies() {
                    assert!(!over_building(&arena, p, r));
                }
            }
        }
    }
    assert!(found > seeds * 8 * 3, "too few spots found: {found}");
}

#[test]
fn crawler_shell_covers_the_front_cone() {
    assert_eq!(crawler_mult(0.0, Vec2::X), CRAWLER_SHELL_MULT);
    assert_eq!(crawler_mult(0.0, Vec2::from_angle(1.2)), CRAWLER_SHELL_MULT);
    assert_eq!(
        crawler_mult(0.0, Vec2::from_angle(-1.2)),
        CRAWLER_SHELL_MULT
    );
    assert_eq!(crawler_mult(0.0, Vec2::from_angle(1.3)), 1.0);
    assert_eq!(crawler_mult(0.0, Vec2::Y), 1.0);
    assert_eq!(crawler_mult(0.0, -Vec2::X), 1.0);
    assert_eq!(crawler_mult(PI, -Vec2::X), CRAWLER_SHELL_MULT);
}

#[test]
fn ghoul_blast_hits_each_target_once() {
    let e = |i| Entity::from_raw_u32(i).unwrap();
    let (ghoul, a, b, far) = (e(1), e(2), e(3), e(4));
    let targets = [
        (ghoul, Vec2::ZERO),
        (a, Vec2::new(10.0, 0.0)),
        (a, Vec2::new(10.0, 0.0)),
        (b, Vec2::new(0.0, GHOUL_BLAST_RADIUS)),
        (far, Vec2::new(500.0, 0.0)),
    ];
    let hits = blast_hits(
        Vec2::ZERO,
        GHOUL_BLAST,
        ghoul,
        targets.into_iter(),
        |_, _| true,
    );
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().any(|(t, d)| *t == a && *d > 43.5));
    let edge = hits.iter().find(|(t, _)| *t == b).unwrap().1;
    assert!((edge - GHOUL_BLAST * 0.75).abs() < 1e-3);
}
