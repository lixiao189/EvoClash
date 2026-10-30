//! Grid A* over the static arena, used by server bots.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bevy::math::Vec2;

use crate::map::{ARENA_HALF, ArenaMap, segment_circle};

const CELL: f32 = 16.0;

pub struct NavGrid {
    width: usize,
    height: usize,
    blocked: Vec<bool>,
}

impl NavGrid {
    /// `clearance` is how far cell centers must stay from walls (agent radius plus margin).
    pub fn new(map: &ArenaMap, clearance: f32) -> Self {
        let width = (2.0 * ARENA_HALF.x / CELL).ceil() as usize;
        let height = (2.0 * ARENA_HALF.y / CELL).ceil() as usize;
        let mut blocked = vec![false; width * height];
        let mut mark = |min: Vec2, max: Vec2, hit: &dyn Fn(Vec2) -> bool| {
            let lo = ((min + ARENA_HALF) / CELL).floor().max(Vec2::ZERO);
            let hi = ((max + ARENA_HALF) / CELL).ceil();
            for y in lo.y as usize..(hi.y as usize).min(height) {
                for x in lo.x as usize..(hi.x as usize).min(width) {
                    if hit(Self::center_of(x, y)) {
                        blocked[y * width + x] = true;
                    }
                }
            }
        };
        for wall in &map.walls {
            let r = wall.inflate(clearance);
            mark(r.min, r.max, &|c| r.contains(c));
        }
        // Props come and go, but treating their spots as always solid keeps paths simple.
        for prop in &map.props {
            let reach = prop.kind.radius() + clearance;
            let at = prop.pos;
            mark(at - reach, at + reach, &|c| c.distance(at) < reach);
        }
        Self {
            width,
            height,
            blocked,
        }
    }

    fn center_of(x: usize, y: usize) -> Vec2 {
        -ARENA_HALF + Vec2::new((x as f32 + 0.5) * CELL, (y as f32 + 0.5) * CELL)
    }

    fn cell_of(&self, p: Vec2) -> (usize, usize) {
        let local = (p + ARENA_HALF) / CELL;
        (
            (local.x.max(0.0) as usize).min(self.width - 1),
            (local.y.max(0.0) as usize).min(self.height - 1),
        )
    }

    fn is_open(&self, x: usize, y: usize) -> bool {
        !self.blocked[y * self.width + x]
    }

    /// Nearest open cell to `p` in plain sight of it (spiral search), for agents hugging a
    /// wall. Never one behind a wall: walking to it would just push into that wall.
    fn open_cell_near(&self, map: &ArenaMap, p: Vec2) -> Option<(usize, usize)> {
        let (cx, cy) = self.cell_of(p);
        for r in 0..10i32 {
            let mut best: Option<((usize, usize), f32)> = None;
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                    if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                        continue;
                    }
                    let (x, y) = (x as usize, y as usize);
                    if self.is_open(x, y) && map.line_of_sight(p, Self::center_of(x, y)) {
                        let d = Self::center_of(x, y).distance_squared(p);
                        if best.is_none_or(|(_, bd)| d < bd) {
                            best = Some(((x, y), d));
                        }
                    }
                }
            }
            if let Some((cell, _)) = best {
                return Some(cell);
            }
        }
        None
    }

    /// 8-connected A* path of cell centers from `from` to `to` (without corner cutting).
    pub fn path(&self, map: &ArenaMap, from: Vec2, to: Vec2) -> Option<Vec<Vec2>> {
        let start = self.open_cell_near(map, from)?;
        let goal = self.open_cell_near(map, to)?;
        let idx = |(x, y): (usize, usize)| y * self.width + x;
        let heuristic = |(x, y): (usize, usize)| {
            let dx = (x as i32 - goal.0 as i32).unsigned_abs();
            let dy = (y as i32 - goal.1 as i32).unsigned_abs();
            // Octile distance in tenths of a cell.
            10 * dx.max(dy) + 4 * dx.min(dy)
        };

        let mut cost = vec![u32::MAX; self.width * self.height];
        let mut came_from = vec![usize::MAX; self.width * self.height];
        let mut open = BinaryHeap::new();
        cost[idx(start)] = 0;
        open.push(Reverse((heuristic(start), idx(start))));

        while let Some(Reverse((_, current))) = open.pop() {
            if current == idx(goal) {
                let mut cells = vec![current];
                let mut c = current;
                while came_from[c] != usize::MAX {
                    c = came_from[c];
                    cells.push(c);
                }
                cells.reverse();
                let mut points: Vec<Vec2> = cells
                    .into_iter()
                    .map(|i| Self::center_of(i % self.width, i / self.width))
                    .collect();
                if let Some(last) = points.last_mut() {
                    *last = to;
                }
                return Some(points);
            }
            let (x, y) = (current % self.width, current / self.width);
            for (dx, dy) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx >= self.width as i32 || ny >= self.height as i32 {
                    continue;
                }
                let (nx, ny) = (nx as usize, ny as usize);
                if !self.is_open(nx, ny) {
                    continue;
                }
                let diagonal = dx != 0 && dy != 0;
                if diagonal && (!self.is_open(nx, y) || !self.is_open(x, ny)) {
                    continue;
                }
                let next = idx((nx, ny));
                let step = if diagonal { 14 } else { 10 };
                let new_cost = cost[current] + step;
                if new_cost < cost[next] {
                    cost[next] = new_cost;
                    came_from[next] = current;
                    open.push(Reverse((new_cost + heuristic((nx, ny)), next)));
                }
            }
        }
        None
    }
}

/// Can a circle of `radius` slide straight from `a` to `b` without touching a wall or a prop?
pub fn clear_sweep(map: &ArenaMap, a: Vec2, b: Vec2, radius: f32) -> bool {
    let n = (b - a).normalize_or_zero().perp() * radius;
    map.line_of_sight(a, b)
        && map.line_of_sight(a + n, b + n)
        && map.line_of_sight(a - n, b - n)
        && map
            .props
            .iter()
            .all(|p| segment_circle(a, b, p.pos, p.kind.radius() + radius).is_none())
}

/// Extra room string-pulling keeps from corners, so agents don't clip them on the way past.
const SWEEP_MARGIN: f32 = 2.0;

/// Point to walk toward this tick to eventually reach `to`.
pub fn next_waypoint(grid: &NavGrid, map: &ArenaMap, from: Vec2, to: Vec2, radius: f32) -> Vec2 {
    let radius = radius + SWEEP_MARGIN;
    if clear_sweep(map, from, to, radius) {
        return to;
    }
    let Some(path) = grid.path(map, from, to) else {
        return to;
    };
    // String-pull: aim at the furthest path point we can reach in a straight line.
    path.iter()
        .rev()
        .find(|p| clear_sweep(map, from, **p, radius))
        .or(path.get(1))
        .copied()
        .unwrap_or(to)
}

/// Can a circle of `radius` walk `probe` along `dir` from `from` without touching a wall or
/// one of the live `obstacles`? Samples every `radius`, so no wall is stepped over.
pub fn clear_ahead(
    map: &ArenaMap,
    obstacles: &[(Vec2, f32)],
    from: Vec2,
    dir: Vec2,
    radius: f32,
    probe: f32,
) -> bool {
    let dir = dir.normalize_or_zero();
    let steps = (probe / radius).ceil().max(1.0) as u32;
    (1..=steps).all(|i| {
        let p = from + dir * (probe * i as f32 / steps as f32);
        map.resolve_with(p, radius, obstacles).distance_squared(p) < 0.25
    })
}

/// `dir` turned as little as possible (up to 135°, either way) so the next `probe` of travel
/// stays clear, keeping its length. Unchanged when it is already clear or nothing is.
pub fn steer_clear(
    map: &ArenaMap,
    obstacles: &[(Vec2, f32)],
    from: Vec2,
    dir: Vec2,
    radius: f32,
    probe: f32,
) -> Vec2 {
    if dir == Vec2::ZERO {
        return dir;
    }
    [0.0f32, 1.0, -1.0, 2.0, -2.0, 3.0, -3.0]
        .into_iter()
        .map(|eighths| Vec2::from_angle(eighths * std::f32::consts::FRAC_PI_4).rotate(dir))
        .find(|d| clear_ahead(map, obstacles, from, *d, radius, probe))
        .unwrap_or(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{DT, PLAYER_SPEED};

    #[test]
    fn finds_route_into_the_vault() {
        let map = ArenaMap::generate(3);
        let grid = NavGrid::new(&map, 24.0);
        let from = map.spawn_points[0];
        let path = grid.path(&map, from, Vec2::ZERO).expect("vault reachable");
        assert!(path.len() > 2);
        // Every hop on the path is walkable.
        for pair in path.windows(2) {
            assert!(map.line_of_sight(pair[0], pair[1]));
        }
    }

    /// Every spawn, loot spot and prop of every layout can be walked to: no room is sealed
    /// off and no doorway is blocked by clutter. Uses the bots' clearance, which is stricter
    /// than what a player needs.
    #[test]
    fn every_layout_is_connected() {
        for seed in 0..120 {
            let map = ArenaMap::generate(seed);
            let grid = NavGrid::new(&map, 24.0);
            let from = map.spawn_points[0];
            let reach = |to: Vec2| {
                let path = grid.path(&map, from, to)?;
                let end = *path.get(path.len().saturating_sub(2))?;
                (end.distance(to) < 120.0 && map.line_of_sight(end, to)).then_some(())
            };
            for to in map
                .spawn_points
                .iter()
                .chain(map.loot_spots.iter().map(|l| &l.pos))
            {
                assert!(reach(*to).is_some(), "seed {seed}: {to} unreachable");
            }
            for prop in &map.props {
                assert!(
                    reach(prop.pos).is_some(),
                    "seed {seed}: {prop:?} unreachable"
                );
            }
        }
    }

    /// Facing a wall head-on, the look-ahead sees it and turns to a free heading.
    #[test]
    fn steers_off_a_wall_ahead() {
        let map = ArenaMap::generate(3);
        let r = 22.0;
        let mut checked = 0;
        for wall in &map.walls {
            let at = Vec2::new(wall.min.x - r - 10.0, wall.center().y);
            if map.resolve_with(at, r, &[]) != at || wall.height() < 4.0 * r {
                continue;
            }
            assert!(!clear_ahead(&map, &[], at, Vec2::X, r, 2.0 * r));
            let turned = steer_clear(&map, &[], at, Vec2::X, r, 2.0 * r);
            assert!(
                turned.dot(Vec2::X) < 0.9,
                "{at}: still heading into the wall"
            );
            assert!(clear_ahead(&map, &[], at, turned, r, 2.0 * r));
            checked += 1;
        }
        assert!(checked > 5);
    }

    /// An agent following the waypoints walks from every spawn to the vault, barely
    /// touching a wall on the way (no grinding along corners).
    #[test]
    fn agents_walk_without_grinding_walls() {
        let r = 22.0;
        let speed = PLAYER_SPEED * DT;
        for seed in 0..20 {
            let map = ArenaMap::generate(seed);
            let grid = NavGrid::new(&map, r + 2.0);
            for &spawn in &map.spawn_points {
                let (mut at, mut bumps, mut ticks) = (spawn, 0, 0);
                while at.distance(Vec2::ZERO) > 40.0 {
                    ticks += 1;
                    assert!(
                        ticks < 3000,
                        "seed {seed}: stuck walking from {spawn} at {at}"
                    );
                    let dir = next_waypoint(&grid, &map, at, Vec2::ZERO, r) - at;
                    let dir = steer_clear(&map, &[], at, dir.normalize_or_zero(), r, 2.0 * r);
                    let want = at + dir * speed;
                    at = map.resolve_with(want, r, &[]);
                    if at.distance_squared(want) > 1.0 {
                        bumps += 1;
                    }
                }
                assert!(
                    bumps * 20 < ticks,
                    "seed {seed}: {bumps} bumps in {ticks} ticks from {spawn}"
                );
            }
        }
    }
}
