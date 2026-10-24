//! The arena: a ruined city assembled fresh every match from hand-authored blocks, around a
//! sealed pre-collapse vault in the middle.
//!
//! The layout is a pure function of a seed (replicated in `MatchState`), so server and client
//! build identical walls and collision can still be predicted without replicating any walls.

use bevy::math::{Rect, Vec2};
use bevy::reflect::Reflect;
use serde::{Deserialize, Serialize};

mod geometry;
mod index;
mod lot;
mod prefabs;
#[cfg(test)]
mod tests;

pub use geometry::*;
use index::*;
use lot::*;
use prefabs::*;

pub const ARENA_HALF: Vec2 = Vec2::new(2200.0, 1500.0);
pub const VAULT_CENTER: Vec2 = Vec2::ZERO;
/// The arena is a 5x3 grid of city blocks; the vault plaza is the middle one.
const COLS: i32 = 5;
const ROWS: i32 = 3;
pub const BLOCK: Vec2 = Vec2::new(880.0, 1000.0);
/// Block contents stay this far inside their block, so streets run between all of them.
pub const MARGIN: f32 = 70.0;
/// Building wall thickness.
const WALL: f32 = 20.0;
/// Width of every doorway. Wide enough that the bots' nav grid (32 px cells, ~28 px clearance)
/// always finds a cell through it.
pub const DOOR: f32 = 100.0;
/// Free space kept between scattered solids so every gap stays walkable.
const CLEAR: f32 = 100.0;

/// Loot spot with the technology tier of the surrounding zone. Tier 0 is a medical cache.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LootSpot {
    pub pos: Vec2,
    pub tier: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Biome {
    Wasteland,
    Urban,
    Industrial,
    Radiation,
    Vault,
}

/// One city block of the grid and what was built on it.
#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub rect: Rect,
    pub biome: Biome,
    pub tier: u8,
    pub name: &'static str,
}

/// How a wall is drawn (and how tall it looks). Collision is the same for all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WallStyle {
    Brick,
    /// Shipping containers, tanks, car wrecks, shelving.
    Metal,
    Rock,
    /// Low sandbag barricades.
    Sandbag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Roof {
    Tin,
    Tar,
    Concrete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Floor {
    Planks,
    Tiles,
    Concrete,
}

/// A roofed building. Its walls are in `ArenaMap::walls`; the roof hides whoever is inside
/// from everyone outside.
#[derive(Clone, Copy, Debug)]
pub struct Building {
    pub rect: Rect,
    pub roof: Roof,
    pub floor: Floor,
}

/// Breakable or interactive things placed by the generator; the server spawns them as entities.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum PropKind {
    /// Smash it open for loot.
    Crate,
    /// Explodes when destroyed, hurting everything around it.
    Barrel,
    /// Indestructible; hold interact next to it to crack it open for better loot.
    Safe,
}

impl PropKind {
    pub fn radius(self) -> f32 {
        match self {
            PropKind::Crate => 20.0,
            PropKind::Barrel => 16.0,
            PropKind::Safe => 22.0,
        }
    }

    pub fn max_hp(self) -> f32 {
        match self {
            PropKind::Crate => 40.0,
            PropKind::Barrel => 25.0,
            PropKind::Safe => 1.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PropKind::Crate => "补给箱",
            PropKind::Barrel => "油桶",
            PropKind::Safe => "保险柜",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropSpot {
    pub pos: Vec2,
    pub kind: PropKind,
    /// Tier of the loot it holds.
    pub tier: u8,
}

pub struct ArenaMap {
    pub seed: u64,
    pub walls: Vec<Rect>,
    /// Parallel to `walls`.
    pub wall_styles: Vec<WallStyle>,
    pub blocks: Vec<Block>,
    pub buildings: Vec<Building>,
    /// Gaps in buildings' outer walls (the doorway itself, wall-thick).
    pub entrances: Vec<Rect>,
    /// Radiation zones: standing in one hurts unless you wear power armor.
    pub hazards: Vec<Rect>,
    pub loot_spots: Vec<LootSpot>,
    pub props: Vec<PropSpot>,
    pub spawn_points: Vec<Vec2>,
    index: WallIndex,
}

impl ArenaMap {
    pub fn generate(seed: u64) -> Self {
        let mut b = Builder::new(seed);
        let t = 60.0;
        let (hx, hy) = (ARENA_HALF.x, ARENA_HALF.y);
        // Boundary.
        for (c, s) in [
            (
                Vec2::new(0.0, hy + t / 2.0),
                Vec2::new(2.0 * hx + 2.0 * t, t),
            ),
            (
                Vec2::new(0.0, -hy - t / 2.0),
                Vec2::new(2.0 * hx + 2.0 * t, t),
            ),
            (Vec2::new(hx + t / 2.0, 0.0), Vec2::new(t, 2.0 * hy)),
            (Vec2::new(-hx - t / 2.0, 0.0), Vec2::new(t, 2.0 * hy)),
        ] {
            b.walls.push(Rect::from_center_size(c, s));
            b.styles.push(WallStyle::Brick);
        }

        // Inner ring (around the vault) gets the better loot and a radiation zone or two;
        // the outer columns are scrapyards and wilderness.
        let mut inner: Vec<(i32, i32)> = Vec::new();
        let mut outer: Vec<(i32, i32)> = Vec::new();
        for row in 0..ROWS {
            for col in 0..COLS {
                match col {
                    2 if row == 1 => {}
                    1..=3 => inner.push((col, row)),
                    _ => outer.push((col, row)),
                }
            }
        }
        let rad_count = if b.rng.chance(0.5) { 2 } else { 1 };
        b.rng.shuffle(&mut inner);
        let mut inner_deck = vec![
            Prefab::Apartments,
            Prefab::Hospital,
            Prefab::Supermarket,
            Prefab::Checkpoint,
            Prefab::Shops,
            Prefab::Warehouse,
            Prefab::Homes,
            Prefab::TankFarm,
        ];
        b.rng.shuffle(&mut inner_deck);
        let mut outer_deck = vec![
            Prefab::Rocks,
            Prefab::Rocks,
            Prefab::Camp,
            Prefab::Homes,
            Prefab::GasStation,
            Prefab::Warehouse,
            Prefab::TankFarm,
            Prefab::Shops,
        ];
        b.rng.shuffle(&mut outer_deck);

        let mut plan: Vec<((i32, i32), Prefab, u8)> = vec![((2, 1), Prefab::Vault, 3)];
        for (i, cell) in inner.iter().enumerate() {
            if i < rad_count {
                plan.push((*cell, Prefab::RadLab, 3));
            } else {
                plan.push((*cell, inner_deck[i - rad_count], 2));
            }
        }
        for (i, cell) in outer.iter().enumerate() {
            plan.push((*cell, outer_deck[i], 1));
        }
        // Furnish in grid order so the result never depends on the shuffles above.
        plan.sort_by_key(|((col, row), ..)| (*row, *col));
        for ((col, row), prefab, tier) in plan {
            let center = Vec2::new(
                (col - COLS / 2) as f32 * BLOCK.x,
                (row - ROWS / 2) as f32 * BLOCK.y,
            );
            let flip = if prefab == Prefab::Vault {
                Vec2::ONE
            } else {
                Vec2::new(b.rng.sign(), b.rng.sign())
            };
            let mut lot = Lot {
                b: &mut b,
                center,
                flip,
                tier,
            };
            prefab.build(&mut lot);
            b.blocks.push(Block {
                rect: Rect::from_center_size(center, BLOCK),
                biome: prefab.biome(),
                tier,
                name: prefab.name(),
            });
        }

        // Respawns happen in the streets around the edge, never inside a building.
        let edge = Vec2::new(hx - MARGIN / 2.0, hy - MARGIN / 2.0);
        let mut spawn_points = Vec::new();
        for s in [-1.0, 1.0] {
            for y in [-edge.y, -500.0, 500.0, edge.y] {
                spawn_points.push(Vec2::new(s * edge.x, y));
            }
            for x in [-1320.0, -440.0, 440.0, 1320.0] {
                spawn_points.push(Vec2::new(x, s * edge.y));
            }
            for y in [-500.0, 500.0] {
                spawn_points.push(Vec2::new(s * 1320.0, y));
            }
        }

        let index = WallIndex::new(&b.walls);
        Self {
            seed,
            walls: b.walls,
            wall_styles: b.styles,
            blocks: b.blocks,
            buildings: b.buildings,
            entrances: b.entrances,
            hazards: b.hazards,
            loot_spots: b.loot,
            props: b.props,
            spawn_points,
            index,
        }
    }

    /// Push a circle out of every wall it overlaps.
    pub fn resolve_circle(&self, mut pos: Vec2, radius: f32) -> Vec2 {
        for _ in 0..2 {
            let reach = Vec2::splat(radius * 2.0);
            self.index
                .query(&self.walls, pos - reach, pos + reach, |w| {
                    let closest = pos.clamp(w.min, w.max);
                    let delta = pos - closest;
                    let dist_sq = delta.length_squared();
                    if dist_sq >= radius * radius {
                        return;
                    }
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
                });
        }
        pos.clamp(-ARENA_HALF, ARENA_HALF)
    }

    /// Like `resolve_circle`, but also slides around round obstacles (crates, barrels, safes).
    pub fn resolve_with(&self, pos: Vec2, radius: f32, obstacles: &[(Vec2, f32)]) -> Vec2 {
        let mut p = self.resolve_circle(pos, radius);
        let mut pushed = false;
        for (center, r) in obstacles {
            let min = r + radius;
            let d = p - *center;
            if d.length_squared() < min * min {
                p = *center + d.normalize_or(Vec2::Y) * min;
                pushed = true;
            }
        }
        if pushed {
            p = self.resolve_circle(p, radius);
        }
        p
    }

    /// Fraction `t` in [0, 1] along `from -> to` where the segment first enters a wall.
    pub fn segment_hits_wall(&self, from: Vec2, to: Vec2) -> Option<f32> {
        let mut best: Option<f32> = None;
        self.index
            .query(&self.walls, from.min(to), from.max(to), |w| {
                if let Some(t) = segment_rect(from, to, *w) {
                    best = Some(best.map_or(t, |b| b.min(t)));
                }
            });
        best
    }

    /// Whether there is an unobstructed straight line between two points.
    pub fn line_of_sight(&self, a: Vec2, b: Vec2) -> bool {
        self.segment_hits_wall(a, b).is_none()
    }

    pub fn in_hazard(&self, p: Vec2) -> bool {
        self.hazards.iter().any(|h| h.contains(p))
    }

    /// Index of the building whose footprint contains `p`.
    pub fn building_at(&self, p: Vec2) -> Option<usize> {
        self.buildings.iter().position(|b| b.rect.contains(p))
    }

    pub fn block_at(&self, p: Vec2) -> Option<&Block> {
        self.blocks.iter().find(|b| b.rect.contains(p))
    }
}

impl Default for ArenaMap {
    fn default() -> Self {
        Self::generate(0)
    }
}
