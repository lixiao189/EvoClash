//! The arena: a ruined city assembled fresh every match from hand-authored blocks, around a
//! sealed pre-collapse vault in the middle.
//!
//! The layout is a pure function of a seed (replicated in `MatchState`), so server and client
//! build identical walls and collision can still be predicted without replicating any walls.

use bevy::math::{Rect, Vec2};
use bevy::reflect::Reflect;
use serde::{Deserialize, Serialize};

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

// ---------------------------------------------------------------- broadphase

const INDEX_CELL: f32 = 256.0;

/// Uniform grid of wall indices, so collision and line-of-sight only test nearby walls.
struct WallIndex {
    origin: Vec2,
    cols: i32,
    rows: i32,
    cells: Vec<Vec<u16>>,
}

impl WallIndex {
    fn new(walls: &[Rect]) -> Self {
        let lo = walls
            .iter()
            .fold(Vec2::splat(f32::MAX), |a, w| a.min(w.min));
        let hi = walls
            .iter()
            .fold(Vec2::splat(f32::MIN), |a, w| a.max(w.max));
        let cols = ((hi.x - lo.x) / INDEX_CELL).ceil().max(1.0) as i32;
        let rows = ((hi.y - lo.y) / INDEX_CELL).ceil().max(1.0) as i32;
        let mut index = Self {
            origin: lo,
            cols,
            rows,
            cells: vec![Vec::new(); (cols * rows) as usize],
        };
        for (i, w) in walls.iter().enumerate() {
            let (x0, y0, x1, y1) = index.span(w.min, w.max);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    index.cells[(y * cols + x) as usize].push(i as u16);
                }
            }
        }
        index
    }

    fn span(&self, min: Vec2, max: Vec2) -> (i32, i32, i32, i32) {
        let cell = |v: f32, o: f32, n: i32| (((v - o) / INDEX_CELL).floor() as i32).clamp(0, n - 1);
        (
            cell(min.x, self.origin.x, self.cols),
            cell(min.y, self.origin.y, self.rows),
            cell(max.x, self.origin.x, self.cols),
            cell(max.y, self.origin.y, self.rows),
        )
    }

    /// Visit every wall in the cells overlapping `min..max`. A wall spanning several cells is
    /// visited once per cell, which is harmless for every caller; huge queries scan everything.
    fn query(&self, walls: &[Rect], min: Vec2, max: Vec2, mut f: impl FnMut(&Rect)) {
        let (x0, y0, x1, y1) = self.span(min, max);
        if (x1 - x0 + 1) * (y1 - y0 + 1) > 24 {
            walls.iter().for_each(f);
            return;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                for &i in &self.cells[(y * self.cols + x) as usize] {
                    f(&walls[i as usize]);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- generation

/// SplitMix64: tiny and bit-identical on every platform, so the browser rebuilds exactly the
/// layout the server generated.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Whole number in `lo..hi`; layouts snap to whole pixels.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        (lo + (hi - lo) * self.unit()).round()
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    fn sign(&mut self) -> f32 {
        if self.chance(0.5) { 1.0 } else { -1.0 }
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

struct Builder {
    rng: Rng,
    walls: Vec<Rect>,
    styles: Vec<WallStyle>,
    blocks: Vec<Block>,
    buildings: Vec<Building>,
    entrances: Vec<Rect>,
    hazards: Vec<Rect>,
    loot: Vec<LootSpot>,
    props: Vec<PropSpot>,
    /// Doorways (inflated) that clutter must keep clear of.
    doors: Vec<Rect>,
}

impl Builder {
    fn new(seed: u64) -> Self {
        Self {
            rng: Rng(seed ^ 0x45_766f_436c_6173),
            walls: Vec::new(),
            styles: Vec::new(),
            blocks: Vec::new(),
            buildings: Vec::new(),
            entrances: Vec::new(),
            hazards: Vec::new(),
            loot: Vec::new(),
            props: Vec::new(),
            doors: Vec::new(),
        }
    }

    fn dist_to_walls(&self, p: Vec2) -> f32 {
        self.walls
            .iter()
            .map(|w| p.distance(p.clamp(w.min, w.max)))
            .fold(f32::MAX, f32::min)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    N,
    S,
    E,
    W,
}

/// One block being furnished, in local coordinates centered on the block and optionally
/// mirrored, so every prefab comes in four orientations.
struct Lot<'a> {
    b: &'a mut Builder,
    center: Vec2,
    flip: Vec2,
    tier: u8,
}

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

/// Half the usable area of a block, in local coordinates.
const HALF: Vec2 = Vec2::new(BLOCK.x / 2.0 - MARGIN, BLOCK.y / 2.0 - MARGIN);

impl Lot<'_> {
    fn at(&self, p: Vec2) -> Vec2 {
        self.center + p * self.flip
    }

    fn rect(&self, c: Vec2, size: Vec2) -> Rect {
        Rect::from_center_size(self.at(c), size)
    }

    fn rand(&mut self, lo: f32, hi: f32) -> f32 {
        self.b.rng.range(lo, hi)
    }

    fn chance(&mut self, p: f32) -> bool {
        self.b.rng.chance(p)
    }

    /// The whole usable area of the block, in world coordinates.
    fn area(&self) -> Rect {
        Rect::from_center_size(self.center, HALF * 2.0)
    }

    fn wall(&mut self, c: Vec2, size: Vec2, style: WallStyle) {
        let r = self.rect(c, size);
        self.b.walls.push(r);
        self.b.styles.push(style);
    }

    fn loot(&mut self, p: Vec2, tier: u8) {
        let pos = self.at(p);
        self.b.loot.push(LootSpot { pos, tier });
    }

    fn prop(&mut self, p: Vec2, kind: PropKind, tier: u8) {
        let pos = self.at(p);
        self.b.props.push(PropSpot { pos, kind, tier });
    }

    fn hazard(&mut self, c: Vec2, size: Vec2) {
        let r = self.rect(c, size);
        self.b.hazards.push(r);
    }

    /// A horizontal wall from `x0` to `x1` at `y`, with doorways centered on `gaps`.
    fn run_h(&mut self, x0: f32, x1: f32, y: f32, gaps: &[f32]) {
        let mut start = x0;
        for g in gaps {
            let (a, b) = (g - DOOR / 2.0, g + DOOR / 2.0);
            if a > start {
                self.wall(
                    v((start + a) / 2.0, y),
                    v(a - start, WALL),
                    WallStyle::Brick,
                );
            }
            let door = self.rect(v(*g, y), v(DOOR, WALL)).inflate(60.0);
            self.b.doors.push(door);
            start = b;
        }
        if x1 > start {
            self.wall(
                v((start + x1) / 2.0, y),
                v(x1 - start, WALL),
                WallStyle::Brick,
            );
        }
    }

    /// A vertical wall from `y0` to `y1` at `x`, with doorways centered on `gaps`.
    fn run_v(&mut self, y0: f32, y1: f32, x: f32, gaps: &[f32]) {
        let mut start = y0;
        for g in gaps {
            let (a, b) = (g - DOOR / 2.0, g + DOOR / 2.0);
            if a > start {
                self.wall(
                    v(x, (start + a) / 2.0),
                    v(WALL, a - start),
                    WallStyle::Brick,
                );
            }
            let door = self.rect(v(x, *g), v(WALL, DOOR)).inflate(60.0);
            self.b.doors.push(door);
            start = b;
        }
        if y1 > start {
            self.wall(
                v(x, (start + y1) / 2.0),
                v(WALL, y1 - start),
                WallStyle::Brick,
            );
        }
    }

    /// A roofed building centered at `c`. Doors are `(side, offset along that side from the
    /// middle)`. Returns the interior in local coordinates.
    fn building(
        &mut self,
        c: Vec2,
        size: Vec2,
        doors: &[(Side, f32)],
        roof: Roof,
        floor: Floor,
    ) -> Rect {
        let h = size / 2.0;
        let gaps = |side: Side, base: f32, half_len: f32| {
            // Doors stay clear of the corners so no stub of wall is left dangling.
            let limit = (half_len - WALL - DOOR / 2.0 - 40.0).max(0.0);
            let mut out: Vec<f32> = doors
                .iter()
                .filter(|(s, _)| *s == side)
                .map(|(_, o)| base + o.clamp(-limit, limit))
                .collect();
            out.sort_by(f32::total_cmp);
            out
        };
        let n = gaps(Side::N, c.x, h.x);
        let s = gaps(Side::S, c.x, h.x);
        let e = gaps(Side::E, c.y, h.y);
        let w = gaps(Side::W, c.y, h.y);
        let (top, bottom) = (c.y + h.y - WALL / 2.0, c.y - h.y + WALL / 2.0);
        let (right, left) = (c.x + h.x - WALL / 2.0, c.x - h.x + WALL / 2.0);
        self.run_h(c.x - h.x, c.x + h.x, top, &n);
        self.run_h(c.x - h.x, c.x + h.x, bottom, &s);
        let (y0, y1) = (c.y - h.y + WALL, c.y + h.y - WALL);
        self.run_v(y0, y1, right, &e);
        self.run_v(y0, y1, left, &w);
        let horizontal = n
            .iter()
            .map(|x| (*x, top))
            .chain(s.iter().map(|x| (*x, bottom)));
        let vertical = e
            .iter()
            .map(|y| (right, *y))
            .chain(w.iter().map(|y| (left, *y)));
        let gaps: Vec<Rect> = horizontal
            .map(|(x, y)| self.rect(v(x, y), v(DOOR, WALL)))
            .chain(vertical.map(|(x, y)| self.rect(v(x, y), v(WALL, DOOR))))
            .collect();
        self.b.entrances.extend(gaps);
        let rect = self.rect(c, size);
        self.b.buildings.push(Building { rect, roof, floor });
        Rect::from_center_size(c, size - 2.0 * WALL)
    }

    /// Corner of a local interior for something that hugs the walls there.
    fn corner(&mut self, interior: Rect, inset: f32) -> Vec2 {
        let x = if self.chance(0.5) {
            interior.min.x + inset
        } else {
            interior.max.x - inset
        };
        let y = if self.chance(0.5) {
            interior.min.y + inset
        } else {
            interior.max.y - inset
        };
        v(x, y)
    }

    /// World rect of a local one (mirroring may swap its corners).
    fn world(&self, local: Rect) -> Rect {
        Rect::from_corners(self.at(local.min), self.at(local.max))
    }

    /// Drop a solid somewhere in `area` (world) that leaves room to walk around it.
    fn scatter_solid(&mut self, area: Rect, size: Vec2, style: WallStyle) -> bool {
        for _ in 0..24 {
            let half = size / 2.0;
            if area.width() < size.x || area.height() < size.y {
                return false;
            }
            let c = v(
                self.b.rng.range(area.min.x + half.x, area.max.x - half.x),
                self.b.rng.range(area.min.y + half.y, area.max.y - half.y),
            );
            let r = Rect::from_center_size(c, size);
            let room = r.inflate(CLEAR);
            let blocked = self.b.walls.iter().any(|w| overlaps(*w, room))
                || self.b.doors.iter().any(|d| overlaps(*d, r))
                || self.b.buildings.iter().any(|b| overlaps(b.rect, room))
                || self
                    .b
                    .props
                    .iter()
                    .any(|p| p.pos.distance(p.pos.clamp(room.min, room.max)) < p.kind.radius())
                || self.b.loot.iter().any(|l| r.inflate(30.0).contains(l.pos));
            if !blocked {
                self.b.walls.push(r);
                self.b.styles.push(style);
                return true;
            }
        }
        false
    }

    /// Place a prop in `area` (world). Outdoors it keeps away from buildings.
    fn scatter_prop(&mut self, area: Rect, kind: PropKind, tier: u8, outdoor: bool) -> bool {
        let r = kind.radius();
        for _ in 0..24 {
            let inner = area.inflate(-r - 4.0);
            if inner.width() <= 0.0 || inner.height() <= 0.0 {
                return false;
            }
            let p = v(
                self.b.rng.range(inner.min.x, inner.max.x),
                self.b.rng.range(inner.min.y, inner.max.y),
            );
            let blocked = self.b.dist_to_walls(p) < r + CLEAR - 8.0
                || self.b.doors.iter().any(|d| d.inflate(r).contains(p))
                || (outdoor
                    && self
                        .b
                        .buildings
                        .iter()
                        .any(|b| b.rect.inflate(r + CLEAR).contains(p)))
                || self.b.props.iter().any(|q| {
                    // Barrels may cluster; otherwise leave a walkable gap.
                    let d = q.pos.distance(p);
                    let touch = q.kind.radius() + r;
                    let cluster = kind == PropKind::Barrel && q.kind == PropKind::Barrel;
                    d < touch + 2.0 || (d < touch + CLEAR - 8.0 && !(cluster && d < touch + 6.0))
                })
                || self.b.loot.iter().any(|l| l.pos.distance(p) < r + 30.0);
            if !blocked {
                self.b.props.push(PropSpot { pos: p, kind, tier });
                return true;
            }
        }
        false
    }

    /// A barrel and a couple more right next to it.
    fn scatter_barrels(&mut self, area: Rect, outdoor: bool) {
        if !self.scatter_prop(area, PropKind::Barrel, 0, outdoor) {
            return;
        }
        let cluster = self.b.props.len() - 1;
        let first = self.b.props[cluster].pos;
        let extra = self.b.rng.below(3);
        for i in 0..extra {
            // A fixed compass rose rather than trig, which can differ between wasm and native.
            const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
            const ROSE: [Vec2; 8] = [
                Vec2::new(1.0, 0.0),
                Vec2::new(D, D),
                Vec2::new(0.0, 1.0),
                Vec2::new(-D, D),
                Vec2::new(-1.0, 0.0),
                Vec2::new(-D, -D),
                Vec2::new(0.0, -1.0),
                Vec2::new(D, -D),
            ];
            let dir = ROSE[(self.b.rng.below(8) + 3 * i) % 8];
            let p = first + dir * (2.0 * PropKind::Barrel.radius() + 3.0);
            let r = PropKind::Barrel.radius();
            let free = self.b.dist_to_walls(p) >= r + CLEAR - 8.0
                && area.contains(p)
                && !self.b.doors.iter().any(|d| d.inflate(r).contains(p))
                && self.b.props.iter().enumerate().all(|(i, q)| {
                    // Touching its own cluster is the point; anything else needs a gap.
                    let gap = if i >= cluster { 1.0 } else { CLEAR - 8.0 };
                    q.pos.distance(p) >= q.kind.radius() + r + gap
                });
            if free {
                self.b.props.push(PropSpot {
                    pos: p,
                    kind: PropKind::Barrel,
                    tier: 0,
                });
            }
        }
    }

    fn scatter_loot(&mut self, area: Rect, tier: u8) -> bool {
        for _ in 0..24 {
            let inner = area.inflate(-40.0);
            if inner.width() <= 0.0 || inner.height() <= 0.0 {
                return false;
            }
            let p = v(
                self.b.rng.range(inner.min.x, inner.max.x),
                self.b.rng.range(inner.min.y, inner.max.y),
            );
            let blocked = self.b.dist_to_walls(p) < 40.0
                || self
                    .b
                    .props
                    .iter()
                    .any(|q| q.pos.distance(p) < q.kind.radius() + 34.0)
                || self.b.loot.iter().any(|l| l.pos.distance(p) < 60.0);
            if !blocked {
                self.b.loot.push(LootSpot { pos: p, tier });
                return true;
            }
        }
        false
    }

    fn next_tier(&self) -> u8 {
        (self.tier + 1).min(3)
    }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

/// The hand-authored block layouts. Each one randomizes sizes, doors and clutter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Prefab {
    Vault,
    Homes,
    Shops,
    Apartments,
    Warehouse,
    TankFarm,
    Rocks,
    Camp,
    RadLab,
    GasStation,
    Hospital,
    Checkpoint,
    Supermarket,
}

impl Prefab {
    fn biome(self) -> Biome {
        match self {
            Prefab::Vault => Biome::Vault,
            Prefab::Homes
            | Prefab::Shops
            | Prefab::Apartments
            | Prefab::GasStation
            | Prefab::Hospital
            | Prefab::Supermarket => Biome::Urban,
            Prefab::Warehouse | Prefab::TankFarm | Prefab::Checkpoint => Biome::Industrial,
            Prefab::Rocks | Prefab::Camp => Biome::Wasteland,
            Prefab::RadLab => Biome::Radiation,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Prefab::Vault => "前文明地堡",
            Prefab::Homes => "住宅区",
            Prefab::Shops => "商业街",
            Prefab::Apartments => "公寓废墟",
            Prefab::Warehouse => "仓库区",
            Prefab::TankFarm => "储油罐区",
            Prefab::Rocks => "乱石岗",
            Prefab::Camp => "掠夺者营地",
            Prefab::RadLab => "辐射实验室",
            Prefab::GasStation => "加油站",
            Prefab::Hospital => "医院",
            Prefab::Checkpoint => "军事检查站",
            Prefab::Supermarket => "超市",
        }
    }

    fn build(self, lot: &mut Lot) {
        match self {
            Prefab::Vault => vault(lot),
            Prefab::Homes => homes(lot),
            Prefab::Shops => shops(lot),
            Prefab::Apartments => apartments(lot),
            Prefab::Warehouse => warehouse(lot),
            Prefab::TankFarm => tank_farm(lot),
            Prefab::Rocks => rocks(lot),
            Prefab::Camp => camp(lot),
            Prefab::RadLab => rad_lab(lot),
            Prefab::GasStation => gas_station(lot),
            Prefab::Hospital => hospital(lot),
            Prefab::Checkpoint => checkpoint(lot),
            Prefab::Supermarket => supermarket(lot),
        }
    }
}

/// Outdoor odds and ends shared by most blocks.
fn yard(lot: &mut Lot, crates: usize, barrels: usize, loot: usize) {
    let area = lot.area();
    let tier = lot.tier;
    for _ in 0..crates {
        lot.scatter_prop(area, PropKind::Crate, tier, true);
    }
    for _ in 0..barrels {
        lot.scatter_barrels(area, true);
    }
    for _ in 0..loot {
        lot.scatter_loot(area, tier);
    }
}

fn wreck(lot: &mut Lot) {
    let area = lot.area();
    let size = if lot.chance(0.5) {
        v(112.0, 56.0)
    } else {
        v(56.0, 112.0)
    };
    lot.scatter_solid(area, size, WallStyle::Metal);
}

fn rubble(lot: &mut Lot, count: usize) {
    let area = lot.area();
    for _ in 0..count {
        let size = v(lot.rand(50.0, 130.0), lot.rand(40.0, 100.0));
        lot.scatter_solid(area, size, WallStyle::Rock);
    }
}

/// The sealed bunker: four doorways, apex tech on a timer, information-age loot around it.
fn vault(lot: &mut Lot) {
    let half = 220.0;
    let gap = 120.0;
    let seg = half - gap / 2.0;
    let th = 36.0;
    // Side walls run out past the corner by half a thickness so the corner block is solid.
    let side = seg + th / 2.0;
    for sign in [-1.0, 1.0] {
        lot.wall(
            v(-(gap / 2.0 + seg / 2.0), sign * half),
            v(seg, th),
            WallStyle::Brick,
        );
        lot.wall(
            v(gap / 2.0 + seg / 2.0, sign * half),
            v(seg, th),
            WallStyle::Brick,
        );
        lot.wall(
            v(sign * half, -(gap / 2.0 + side / 2.0)),
            v(th, side),
            WallStyle::Brick,
        );
        lot.wall(
            v(sign * half, gap / 2.0 + side / 2.0),
            v(th, side),
            WallStyle::Brick,
        );
    }
    for sign in [-1.0, 1.0] {
        lot.wall(v(0.0, sign * 360.0), v(160.0, 26.0), WallStyle::Sandbag);
        lot.wall(v(sign * 350.0, 0.0), v(26.0, 160.0), WallStyle::Sandbag);
    }
    for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        lot.loot(v(300.0 * sx, 330.0 * sy), 3);
    }
}

/// Two or three family houses with yards.
fn homes(lot: &mut Lot) {
    let tier = lot.tier;
    let (w, h) = (lot.rand(240.0, 300.0), lot.rand(200.0, 240.0));
    let mut doors = vec![(Side::S, lot.rand(-40.0, 40.0))];
    if lot.chance(0.5) {
        doors.push((Side::E, 0.0));
    }
    let a = lot.building(
        v(-HALF.x + w / 2.0, HALF.y - h / 2.0),
        v(w, h),
        &doors,
        Roof::Tar,
        Floor::Planks,
    );
    lot.loot(a.center(), tier);
    if lot.chance(0.5) {
        let r = PropKind::Safe.radius() + 4.0;
        let p = lot.corner(a, r);
        lot.prop(p, PropKind::Safe, tier);
    }

    let (w, h) = (lot.rand(240.0, 300.0), lot.rand(200.0, 240.0));
    let mut doors = vec![(Side::N, lot.rand(-40.0, 40.0))];
    if lot.chance(0.5) {
        doors.push((Side::W, 0.0));
    }
    let b = lot.building(
        v(HALF.x - w / 2.0, -HALF.y + h / 2.0),
        v(w, h),
        &doors,
        Roof::Tin,
        Floor::Planks,
    );
    lot.loot(b.center(), tier);

    if lot.chance(0.6) {
        let side = if lot.chance(0.5) { Side::S } else { Side::W };
        let c = lot.building(
            v(HALF.x - 100.0, HALF.y - 90.0),
            v(200.0, 180.0),
            &[(side, 0.0)],
            Roof::Tar,
            Floor::Tiles,
        );
        lot.loot(c.center(), tier);
    }
    let area = lot.area();
    for _ in 0..2 {
        let size = if lot.chance(0.5) {
            v(lot.rand(80.0, 140.0), 24.0)
        } else {
            v(24.0, lot.rand(80.0, 140.0))
        };
        lot.scatter_solid(area, size, WallStyle::Sandbag);
    }
    wreck(lot);
    yard(lot, 3, 1, 1);
}

/// A strip of three shops facing a parking lot.
fn shops(lot: &mut Lot) {
    let tier = lot.tier;
    let w = lot.rand(680.0, 740.0);
    let h = lot.rand(220.0, 250.0);
    let c = v(0.0, HALF.y - h / 2.0);
    let mut doors = vec![(Side::S, -w / 3.0), (Side::S, 0.0), (Side::S, w / 3.0)];
    if lot.chance(0.5) {
        doors.push((Side::N, 0.0));
    }
    let inside = lot.building(c, v(w, h), &doors, Roof::Tin, Floor::Tiles);
    for x in [c.x - w / 6.0, c.x + w / 6.0] {
        lot.run_v(inside.min.y, inside.max.y, x, &[]);
    }
    for x in [c.x - w / 3.0, c.x, c.x + w / 3.0] {
        lot.loot(v(x, c.y), tier);
    }
    if lot.chance(0.5) {
        let shop = Rect::new(
            c.x + w / 6.0 + WALL / 2.0,
            inside.min.y,
            inside.max.x,
            inside.max.y,
        );
        let r = PropKind::Safe.radius() + 4.0;
        lot.prop(v(shop.max.x - r, shop.max.y - r), PropKind::Safe, tier);
    }
    for _ in 0..3 {
        wreck(lot);
    }
    let area = lot.area();
    lot.scatter_solid(area, v(160.0, 26.0), WallStyle::Sandbag);
    yard(lot, 2, 2, 1);
}

/// A gutted apartment block: three rooms, a back door, maybe a hole in the wall.
fn apartments(lot: &mut Lot) {
    let tier = lot.tier;
    let size = v(lot.rand(520.0, 580.0), lot.rand(460.0, 520.0));
    let c = v(0.0, 20.0);
    let h = size / 2.0;
    let split_x = c.x - 60.0;
    let split_y = c.y + 60.0;
    let mut doors = vec![
        (Side::S, -h.x / 2.0 - 30.0),
        (Side::E, (-h.y + WALL + (split_y - c.y) - 10.0) / 2.0),
        (Side::N, h.x / 2.0),
    ];
    if lot.chance(0.5) {
        doors.push((Side::W, lot.rand(-80.0, 80.0)));
    }
    let inside = lot.building(c, size, &doors, Roof::Concrete, Floor::Concrete);
    // Left wing | right wing, with a doorway low down.
    let gap_y = c.y - h.y / 2.0;
    lot.run_v(inside.min.y, inside.max.y, split_x, &[gap_y]);
    // Right wing split into two rooms.
    let x0 = split_x + WALL / 2.0;
    lot.run_h(x0, inside.max.x, split_y, &[(x0 + inside.max.x) / 2.0]);

    let left = Rect::new(
        inside.min.x,
        inside.min.y,
        split_x - WALL / 2.0,
        inside.max.y,
    );
    let low = Rect::new(x0, inside.min.y, inside.max.x, split_y - WALL / 2.0);
    let high = Rect::new(x0, split_y + WALL / 2.0, inside.max.x, inside.max.y);
    lot.loot(v(left.center().x, left.min.y + left.height() * 0.3), tier);
    lot.loot(v(left.center().x, left.min.y + left.height() * 0.75), tier);
    lot.loot(low.center(), tier);
    let next = lot.next_tier();
    lot.loot(high.center() - v(40.0, 0.0), next);
    let r = PropKind::Safe.radius() + 4.0;
    lot.prop(v(high.max.x - r, high.max.y - r), PropKind::Safe, tier);
    rubble(lot, 2);
    yard(lot, 2, 2, 0);
}

/// A big warehouse full of crates, shipping containers outside.
fn warehouse(lot: &mut Lot) {
    let tier = lot.tier;
    let size = v(lot.rand(520.0, 600.0), lot.rand(320.0, 360.0));
    let c = v(
        lot.rand(-60.0, 60.0)
            .clamp(-HALF.x + size.x / 2.0, HALF.x - size.x / 2.0),
        HALF.y - size.y / 2.0,
    );
    let mut doors = vec![(Side::S, -size.x / 4.0), (Side::S, size.x / 4.0)];
    if lot.chance(0.5) {
        doors.push((Side::W, 0.0));
    }
    if lot.chance(0.5) {
        doors.push((Side::N, 0.0));
    }
    let inside = lot.building(c, size, &doors, Roof::Tin, Floor::Concrete);
    let inside = lot.world(inside);
    for _ in 0..4 {
        lot.scatter_prop(inside, PropKind::Crate, tier, false);
    }
    lot.scatter_loot(inside, tier);
    lot.scatter_loot(inside, tier);
    let area = lot.area();
    for _ in 0..3 {
        let size = if lot.chance(0.5) {
            v(200.0, 64.0)
        } else {
            v(64.0, 200.0)
        };
        lot.scatter_solid(area, size, WallStyle::Metal);
    }
    yard(lot, 1, 2, 1);
}

/// Fuel tanks, pipes and a little control shed. Lots of things that go boom.
fn tank_farm(lot: &mut Lot) {
    let tier = lot.tier;
    let mut doors = vec![(Side::N, 0.0)];
    if lot.chance(0.5) {
        doors.push((Side::W, 0.0));
    }
    let shed = lot.building(
        v(HALF.x - 110.0, -HALF.y + 85.0),
        v(220.0, 170.0),
        &doors,
        Roof::Tin,
        Floor::Concrete,
    );
    lot.loot(shed.center(), tier);
    if lot.chance(0.4) {
        let r = PropKind::Safe.radius() + 4.0;
        let p = lot.corner(shed, r);
        lot.prop(p, PropKind::Safe, tier);
    }
    let area = lot.area();
    let tanks = 3 + lot.b.rng.below(2);
    for _ in 0..tanks {
        let s = lot.rand(110.0, 150.0);
        lot.scatter_solid(area, v(s, s), WallStyle::Metal);
    }
    for _ in 0..2 {
        let size = if lot.chance(0.5) {
            v(lot.rand(160.0, 240.0), 18.0)
        } else {
            v(18.0, lot.rand(160.0, 240.0))
        };
        lot.scatter_solid(area, size, WallStyle::Metal);
    }
    yard(lot, 2, 4, 2);
}

/// Open wasteland: boulders, wrecks and whatever fell off a truck.
fn rocks(lot: &mut Lot) {
    let count = 8 + lot.b.rng.below(4);
    rubble(lot, count);
    wreck(lot);
    wreck(lot);
    yard(lot, 4, 2, 3);
}

/// A raider camp: a sandbag ring around a tent with the good stuff.
fn camp(lot: &mut Lot) {
    let ring = 230.0;
    let len = 240.0;
    let open = lot.b.rng.below(6);
    for (i, (c, size)) in [
        (v(0.0, ring), v(len, 26.0)),
        (v(0.0, -ring), v(len, 26.0)),
        (v(ring, 0.0), v(26.0, len)),
        (v(-ring, 0.0), v(26.0, len)),
    ]
    .into_iter()
    .enumerate()
    {
        if i != open {
            lot.wall(c, size, WallStyle::Sandbag);
        }
    }
    let side = if lot.chance(0.5) { Side::N } else { Side::S };
    let tent = lot.building(
        v(0.0, 0.0),
        v(200.0, 150.0),
        &[(side, 0.0)],
        Roof::Tar,
        Floor::Planks,
    );
    let next = lot.next_tier();
    lot.loot(tent.center(), next);
    rubble(lot, 3);
    yard(lot, 3, 2, 1);
}

/// A crater-side lab soaked in radiation: the best loot outside the vault, at a price.
fn rad_lab(lot: &mut Lot) {
    lot.hazard(Vec2::ZERO, HALF * 2.0);
    let size = v(lot.rand(300.0, 360.0), lot.rand(240.0, 280.0));
    let c = v(lot.rand(-40.0, 40.0), lot.rand(-40.0, 60.0));
    let doors = [
        (Side::S, lot.rand(-60.0, 60.0)),
        (Side::N, lot.rand(-60.0, 60.0)),
    ];
    let inside = lot.building(c, size, &doors, Roof::Concrete, Floor::Tiles);
    lot.loot(inside.center() - v(60.0, 0.0), 3);
    lot.loot(inside.center() + v(60.0, 0.0), 3);
    let r = PropKind::Safe.radius() + 4.0;
    let p = lot.corner(inside, r);
    lot.prop(p, PropKind::Safe, 3);
    rubble(lot, 4);
    yard(lot, 3, 2, 1);
}

/// A gas station: a shop, two pump islands, and far too many fuel barrels.
fn gas_station(lot: &mut Lot) {
    let tier = lot.tier;
    let mut doors = vec![(Side::S, lot.rand(-50.0, 50.0))];
    if lot.chance(0.5) {
        doors.push((Side::E, 0.0));
    }
    let shop = lot.building(
        v(-HALF.x + 150.0, HALF.y - 100.0),
        v(300.0, 200.0),
        &doors,
        Roof::Tin,
        Floor::Tiles,
    );
    lot.loot(shop.center() - v(50.0, 0.0), tier);
    lot.loot(shop.center() + v(50.0, 0.0), tier);
    let base_x = lot.rand(-20.0, 40.0);
    for y in [-40.0, -220.0] {
        for dx in [0.0, 160.0] {
            lot.wall(v(base_x + dx, y), v(36.0, 60.0), WallStyle::Metal);
        }
    }
    wreck(lot);
    yard(lot, 2, 4, 1);
}

/// A hospital: lobby in the middle, wards on each side, medical caches everywhere.
fn hospital(lot: &mut Lot) {
    let tier = lot.tier;
    let size = v(lot.rand(600.0, 680.0), lot.rand(420.0, 460.0));
    let c = v(0.0, 30.0);
    let h = size / 2.0;
    let doors = [
        (Side::S, 0.0),
        (Side::W, lot.rand(-60.0, 60.0)),
        (Side::E, lot.rand(-60.0, 60.0)),
    ];
    let inside = lot.building(c, size, &doors, Roof::Concrete, Floor::Tiles);
    let wing = h.x * 0.38;
    let left_gap = c.y + lot.rand(-80.0, 80.0);
    let right_gap = c.y + lot.rand(-80.0, 80.0);
    lot.run_v(inside.min.y, inside.max.y, c.x - wing, &[left_gap]);
    lot.run_v(inside.min.y, inside.max.y, c.x + wing, &[right_gap]);
    lot.loot(v(c.x, c.y + 60.0), tier);
    for sx in [-1.0, 1.0] {
        let x = c.x + sx * (wing + inside.max.x - c.x) / 2.0;
        lot.loot(v(x, c.y + h.y / 2.0), tier);
        lot.loot(v(x, c.y - h.y / 2.0), 0);
    }
    lot.loot(v(c.x, c.y - 80.0), 0);
    wreck(lot);
    yard(lot, 2, 1, 0);
}

/// A military checkpoint: sandbag lines, a guard post, and crates of army surplus.
fn checkpoint(lot: &mut Lot) {
    let next = lot.next_tier();
    let side = [Side::N, Side::S, Side::E, Side::W][lot.b.rng.below(4)];
    let post = v(lot.rand(-200.0, 200.0), lot.rand(-250.0, 250.0));
    let inside = lot.building(
        post,
        v(180.0, 180.0),
        &[(side, 0.0)],
        Roof::Concrete,
        Floor::Concrete,
    );
    lot.loot(inside.center(), next);
    let area = lot.area();
    for _ in 0..6 {
        let size = if lot.chance(0.5) {
            v(lot.rand(120.0, 200.0), 26.0)
        } else {
            v(26.0, lot.rand(120.0, 200.0))
        };
        lot.scatter_solid(area, size, WallStyle::Sandbag);
    }
    for _ in 0..4 {
        lot.scatter_prop(area, PropKind::Crate, next, true);
    }
    let tier = lot.tier;
    lot.scatter_loot(area, next);
    lot.scatter_loot(area, tier);
    yard(lot, 0, 1, 0);
}

/// A supermarket: long aisles of shelving to fight through.
fn supermarket(lot: &mut Lot) {
    let tier = lot.tier;
    let c = v(0.0, lot.rand(0.0, 40.0));
    let size = v(620.0, 400.0);
    let doors = [
        (Side::S, -180.0),
        (Side::S, 180.0),
        (Side::N, lot.rand(-150.0, 150.0)),
    ];
    lot.building(c, size, &doors, Roof::Tin, Floor::Tiles);
    for x in [-180.0, -60.0, 60.0, 180.0] {
        lot.wall(v(c.x + x, c.y), v(20.0, 160.0), WallStyle::Metal);
    }
    for x in [-120.0, 0.0, 120.0] {
        let y = c.y + lot.rand(-60.0, 60.0);
        lot.loot(v(c.x + x, y), tier);
    }
    wreck(lot);
    wreck(lot);
    yard(lot, 2, 2, 1);
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
}
