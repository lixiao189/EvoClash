use super::*;

const SEEDS: u64 = 300;

#[test]
fn same_seed_same_layout() {
    let (a, b) = (ArenaMap::generate(42), ArenaMap::generate(42));
    assert_eq!(a.walls, b.walls);
    assert_eq!(a.props, b.props);
    assert_eq!(a.loot_spots, b.loot_spots);
    assert_ne!(a.walls, ArenaMap::generate(43).walls);
}

#[test]
fn spawns_loot_and_props_are_clear_of_walls() {
    for seed in 0..SEEDS {
        let map = ArenaMap::generate(seed);
        for p in map
            .spawn_points
            .iter()
            .copied()
            .chain(map.loot_spots.iter().map(|s| s.pos))
        {
            assert_eq!(
                map.resolve_circle(p, 20.0),
                p,
                "seed {seed}: {p} overlaps a wall"
            );
        }
        for prop in &map.props {
            let r = prop.kind.radius();
            assert_eq!(
                map.resolve_circle(prop.pos, r),
                prop.pos,
                "seed {seed}: {prop:?} overlaps a wall"
            );
        }
        for p in &map.spawn_points {
            assert!(!map.in_hazard(*p) && map.building_at(*p).is_none());
        }
    }
}

#[test]
fn every_block_is_furnished() {
    for seed in 0..SEEDS {
        let map = ArenaMap::generate(seed);
        assert_eq!(map.blocks.len(), (COLS * ROWS) as usize);
        let rads = map
            .blocks
            .iter()
            .filter(|b| b.biome == Biome::Radiation)
            .count();
        assert!((1..=2).contains(&rads));
        assert!(
            map.loot_spots.len() >= 35,
            "seed {seed}: {}",
            map.loot_spots.len()
        );
        assert!(map.props.len() >= 25, "seed {seed}: {}", map.props.len());
        assert_eq!(map.walls.len(), map.wall_styles.len());
    }
}

/// Where a horizontal and a vertical wall meet, the square where the vertical wall's
/// column crosses the horizontal wall's row must be solid. A notch there shows as a hole
/// in the rendered wall and snags players on its inner edges.
#[test]
fn wall_corners_have_no_notches() {
    for seed in 0..SEEDS {
        let map = ArenaMap::generate(seed);
        let solid = |p: Vec2| map.walls.iter().any(|w| w.contains(p));
        let touch = |a: &Rect, b: &Rect| {
            a.min.x <= b.max.x + 0.5
                && b.min.x <= a.max.x + 0.5
                && a.min.y <= b.max.y + 0.5
                && b.min.y <= a.max.y + 0.5
        };
        for h in map.walls.iter().filter(|w| w.width() > w.height()) {
            for v in map.walls.iter().filter(|w| w.height() > w.width()) {
                if !touch(h, v) {
                    continue;
                }
                let corner = Rect::new(v.min.x, h.min.y, v.max.x, h.max.y);
                for (fx, fy) in [(0.1, 0.1), (0.9, 0.1), (0.1, 0.9), (0.9, 0.9), (0.5, 0.5)] {
                    let p = corner.min + corner.size() * Vec2::new(fx, fy);
                    assert!(
                        solid(p),
                        "seed {seed}: notch at {p} where {h:?} meets {v:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn every_building_has_an_entrance() {
    for seed in 0..SEEDS {
        let map = ArenaMap::generate(seed);
        for b in &map.buildings {
            assert!(
                map.entrances.iter().any(|e| b.rect.contains(e.center())),
                "seed {seed}: {b:?} has no door"
            );
        }
    }
}

#[test]
fn broadphase_matches_brute_force() {
    let map = ArenaMap::generate(7);
    let brute = |a: Vec2, b: Vec2| {
        map.walls
            .iter()
            .filter_map(|w| segment_rect(a, b, *w))
            .min_by(f32::total_cmp)
    };
    let mut rng = Rng(99);
    for _ in 0..2000 {
        let p = |rng: &mut Rng| {
            Vec2::new(
                rng.range(-ARENA_HALF.x, ARENA_HALF.x),
                rng.range(-ARENA_HALF.y, ARENA_HALF.y),
            )
        };
        let a = p(&mut rng);
        let b = a + (p(&mut rng) - a) * rng.unit();
        assert_eq!(map.segment_hits_wall(a, b), brute(a, b), "{a} -> {b}");
    }
}

#[test]
fn segment_tests() {
    let r = Rect::from_center_size(Vec2::ZERO, Vec2::splat(10.0));
    assert!(segment_rect(Vec2::new(-20.0, 0.0), Vec2::new(20.0, 0.0), r).is_some());
    assert!(segment_rect(Vec2::new(-20.0, 20.0), Vec2::new(20.0, 20.0), r).is_none());
    let t = segment_circle(Vec2::new(-20.0, 0.0), Vec2::new(20.0, 0.0), Vec2::ZERO, 5.0);
    assert!((t.unwrap() - 0.375).abs() < 1e-4);
}

#[test]
fn obstacles_push_out() {
    let map = ArenaMap::generate(1);
    let p = VAULT_CENTER;
    let out = map.resolve_with(p, 22.0, &[(p + Vec2::X * 10.0, 20.0)]);
    assert!(out.distance(p + Vec2::X * 10.0) >= 41.9);
}
