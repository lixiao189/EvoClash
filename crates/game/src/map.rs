//! The arena: a ruined city block with a sealed pre-collapse vault in the middle.
//!
//! The layout is static and identical on server and client, so collision can be
//! predicted on the client without replicating any wall entities.

use bevy::math::{Rect, Vec2};

pub const ARENA_HALF: Vec2 = Vec2::new(1250.0, 860.0);
/// The layout below is authored at a larger size and scaled down to `ARENA_HALF`.
const LAYOUT_SCALE: f32 = 1250.0 / 1600.0;
pub const VAULT_CENTER: Vec2 = Vec2::ZERO;

/// Loot spawn point with the technology tier of the surrounding zone.
#[derive(Clone, Copy, Debug)]
pub struct LootSpot {
    pub pos: Vec2,
    pub tier: u8,
}

pub struct ArenaMap {
    pub walls: Vec<Rect>,
    pub loot_spots: Vec<LootSpot>,
    pub spawn_points: Vec<Vec2>,
}

fn wall(cx: f32, cy: f32, w: f32, h: f32) -> Rect {
    Rect::from_center_size(Vec2::new(cx, cy), Vec2::new(w, h))
}

impl ArenaMap {
    pub fn new() -> Self {
        let mut walls = Vec::new();
        let (hx, hy) = (ARENA_HALF.x / LAYOUT_SCALE, ARENA_HALF.y / LAYOUT_SCALE);
        let t = 60.0;
        // Boundary.
        walls.push(wall(0.0, hy + t / 2.0, 2.0 * hx + 2.0 * t, t));
        walls.push(wall(0.0, -hy - t / 2.0, 2.0 * hx + 2.0 * t, t));
        walls.push(wall(hx + t / 2.0, 0.0, t, 2.0 * hy));
        walls.push(wall(-hx - t / 2.0, 0.0, t, 2.0 * hy));

        // Vault: a 440x440 bunker with an opening in the middle of each side.
        let v = 220.0;
        let gap = 110.0;
        let seg = v - gap / 2.0;
        let th = 36.0;
        for sign in [-1.0, 1.0] {
            // Horizontal walls (top/bottom), split by the doorway.
            walls.push(wall(-(gap / 2.0 + seg / 2.0), sign * v, seg, th));
            walls.push(wall(gap / 2.0 + seg / 2.0, sign * v, seg, th));
            // Vertical walls (left/right).
            walls.push(wall(sign * v, -(gap / 2.0 + seg / 2.0), th, seg));
            walls.push(wall(sign * v, gap / 2.0 + seg / 2.0, th, seg));
        }

        // Ruined buildings, mirrored into all four quadrants so no spawn side is favored.
        let ruins = [
            // (center, size)
            (Vec2::new(620.0, 420.0), Vec2::new(260.0, 40.0)),
            (Vec2::new(470.0, 560.0), Vec2::new(40.0, 240.0)),
            (Vec2::new(1050.0, 300.0), Vec2::new(40.0, 320.0)),
            (Vec2::new(1200.0, 760.0), Vec2::new(300.0, 40.0)),
            (Vec2::new(760.0, 860.0), Vec2::new(40.0, 220.0)),
            (Vec2::new(300.0, 900.0), Vec2::new(220.0, 40.0)),
            (Vec2::new(1350.0, 100.0), Vec2::new(160.0, 40.0)),
            (Vec2::new(520.0, 150.0), Vec2::new(90.0, 90.0)),
        ];
        for (c, s) in ruins {
            for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                walls.push(Rect::from_center_size(Vec2::new(c.x * sx, c.y * sy), s));
            }
        }

        let mut loot_spots = Vec::new();
        let mut mirrored = |p: Vec2, tier: u8| {
            for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                loot_spots.push(LootSpot {
                    pos: Vec2::new(p.x * sx, p.y * sy),
                    tier,
                });
            }
        };
        // Outskirts: industrial scrap.
        mirrored(Vec2::new(1400.0, 950.0), 1);
        mirrored(Vec2::new(950.0, 980.0), 1);
        mirrored(Vec2::new(1450.0, 450.0), 1);
        mirrored(Vec2::new(150.0, 1000.0), 1);
        // Town ring: modern military.
        mirrored(Vec2::new(620.0, 520.0), 2);
        mirrored(Vec2::new(1150.0, 560.0), 2);
        mirrored(Vec2::new(880.0, 160.0), 2);
        // Around the vault: information age.
        mirrored(Vec2::new(330.0, 330.0), 3);

        let spawn_points = vec![
            Vec2::new(-1450.0, -950.0),
            Vec2::new(1450.0, 950.0),
            Vec2::new(-1450.0, 950.0),
            Vec2::new(1450.0, -950.0),
            Vec2::new(0.0, 1000.0),
            Vec2::new(0.0, -1000.0),
            Vec2::new(-1500.0, 0.0),
            Vec2::new(1500.0, 0.0),
        ];

        let k = LAYOUT_SCALE;
        Self {
            walls: walls
                .into_iter()
                .map(|w| Rect::from_corners(w.min * k, w.max * k))
                .collect(),
            loot_spots: loot_spots
                .into_iter()
                .map(|s| LootSpot {
                    pos: s.pos * k,
                    ..s
                })
                .collect(),
            spawn_points: spawn_points.into_iter().map(|p| p * k).collect(),
        }
    }

    /// Push a circle out of every wall it overlaps.
    pub fn resolve_circle(&self, mut pos: Vec2, radius: f32) -> Vec2 {
        for _ in 0..2 {
            for w in &self.walls {
                let closest = pos.clamp(w.min, w.max);
                let delta = pos - closest;
                let dist_sq = delta.length_squared();
                if dist_sq < radius * radius {
                    if dist_sq > 1e-6 {
                        let dist = dist_sq.sqrt();
                        pos += delta / dist * (radius - dist);
                    } else {
                        // Center is inside the wall: push out along the shallowest axis.
                        let to_min = pos - w.min;
                        let to_max = w.max - pos;
                        let candidates = [
                            (to_min.x, Vec2::new(w.min.x - radius, pos.y)),
                            (to_max.x, Vec2::new(w.max.x + radius, pos.y)),
                            (to_min.y, Vec2::new(pos.x, w.min.y - radius)),
                            (to_max.y, Vec2::new(pos.x, w.max.y + radius)),
                        ];
                        pos = candidates
                            .iter()
                            .min_by(|a, b| a.0.total_cmp(&b.0))
                            .map(|c| c.1)
                            .unwrap_or(pos);
                    }
                }
            }
        }
        pos.clamp(-ARENA_HALF, ARENA_HALF)
    }

    /// Fraction `t` in [0, 1] along `from -> to` where the segment first enters a wall.
    pub fn segment_hits_wall(&self, from: Vec2, to: Vec2) -> Option<f32> {
        self.walls
            .iter()
            .filter_map(|w| segment_rect(from, to, *w))
            .min_by(f32::total_cmp)
    }

    /// Whether there is an unobstructed straight line between two points.
    pub fn line_of_sight(&self, a: Vec2, b: Vec2) -> bool {
        self.segment_hits_wall(a, b).is_none()
    }
}

impl Default for ArenaMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Slab test: entry fraction of segment `a -> b` into `rect`, if any.
pub fn segment_rect(a: Vec2, b: Vec2, rect: Rect) -> Option<f32> {
    let d = b - a;
    let mut t_min: f32 = 0.0;
    let mut t_max: f32 = 1.0;
    for axis in 0..2 {
        let (o, dir, lo, hi) = if axis == 0 {
            (a.x, d.x, rect.min.x, rect.max.x)
        } else {
            (a.y, d.y, rect.min.y, rect.max.y)
        };
        if dir.abs() < 1e-8 {
            if o < lo || o > hi {
                return None;
            }
        } else {
            let mut t1 = (lo - o) / dir;
            let mut t2 = (hi - o) / dir;
            if t1 > t2 {
                core::mem::swap(&mut t1, &mut t2);
            }
            t_min = t_min.max(t1);
            t_max = t_max.min(t2);
            if t_min > t_max {
                return None;
            }
        }
    }
    Some(t_min)
}

/// Entry fraction of segment `a -> b` into a circle, if any.
pub fn segment_circle(a: Vec2, b: Vec2, center: Vec2, radius: f32) -> Option<f32> {
    let d = b - a;
    let f = a - center;
    let c = f.length_squared() - radius * radius;
    if c <= 0.0 {
        return Some(0.0);
    }
    let a_coef = d.length_squared();
    if a_coef < 1e-8 {
        return None;
    }
    let b_coef = 2.0 * f.dot(d);
    let disc = b_coef * b_coef - 4.0 * a_coef * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b_coef - disc.sqrt()) / (2.0 * a_coef);
    (0.0..=1.0).contains(&t).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawns_and_loot_are_not_inside_walls() {
        let map = ArenaMap::new();
        for p in map
            .spawn_points
            .iter()
            .copied()
            .chain(map.loot_spots.iter().map(|s| s.pos))
        {
            assert_eq!(map.resolve_circle(p, 20.0), p, "{p} overlaps a wall");
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
}
