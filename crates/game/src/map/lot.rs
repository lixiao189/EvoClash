//! Generation core: the seeded RNG (bit-identical on wasm and native, so never `rand` or
//! platform float functions here), the builder and the `Lot` scatter helpers.

use super::*;

/// SplitMix64: tiny and bit-identical on every platform, so the browser rebuilds exactly the
/// layout the server generated.
pub(super) struct Rng(pub(super) u64);

impl Rng {
    pub(super) fn next(&mut self) -> u64 {
        crate::rng::SplitMix64::step(&mut self.0)
    }

    pub(super) fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Whole number in `lo..hi`; layouts snap to whole pixels.
    pub(super) fn range(&mut self, lo: f32, hi: f32) -> f32 {
        (lo + (hi - lo) * self.unit()).round()
    }

    pub(super) fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub(super) fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    pub(super) fn sign(&mut self) -> f32 {
        if self.chance(0.5) { 1.0 } else { -1.0 }
    }

    pub(super) fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

pub(super) struct Builder {
    pub(super) rng: Rng,
    pub(super) walls: Vec<Rect>,
    pub(super) styles: Vec<WallStyle>,
    pub(super) blocks: Vec<Block>,
    pub(super) buildings: Vec<Building>,
    pub(super) entrances: Vec<Rect>,
    pub(super) hazards: Vec<Rect>,
    pub(super) loot: Vec<LootSpot>,
    pub(super) props: Vec<PropSpot>,
    /// Doorways (inflated) that clutter must keep clear of.
    pub(super) doors: Vec<Rect>,
}

impl Builder {
    pub(super) fn new(seed: u64) -> Self {
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

    pub(super) fn dist_to_walls(&self, p: Vec2) -> f32 {
        self.walls
            .iter()
            .map(|w| p.distance(p.clamp(w.min, w.max)))
            .fold(f32::MAX, f32::min)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    N,
    S,
    E,
    W,
}

/// One block being furnished, in local coordinates centered on the block and optionally
/// mirrored, so every prefab comes in four orientations.
pub(super) struct Lot<'a> {
    pub(super) b: &'a mut Builder,
    pub(super) center: Vec2,
    pub(super) flip: Vec2,
    pub(super) tier: u8,
}

pub(super) fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

/// Half the usable area of a block, in local coordinates.
pub(super) const HALF: Vec2 = Vec2::new(BLOCK.x / 2.0 - MARGIN, BLOCK.y / 2.0 - MARGIN);

impl Lot<'_> {
    pub(super) fn at(&self, p: Vec2) -> Vec2 {
        self.center + p * self.flip
    }

    pub(super) fn rect(&self, c: Vec2, size: Vec2) -> Rect {
        Rect::from_center_size(self.at(c), size)
    }

    pub(super) fn rand(&mut self, lo: f32, hi: f32) -> f32 {
        self.b.rng.range(lo, hi)
    }

    pub(super) fn chance(&mut self, p: f32) -> bool {
        self.b.rng.chance(p)
    }

    /// The whole usable area of the block, in world coordinates.
    pub(super) fn area(&self) -> Rect {
        Rect::from_center_size(self.center, HALF * 2.0)
    }

    pub(super) fn wall(&mut self, c: Vec2, size: Vec2, style: WallStyle) {
        let r = self.rect(c, size);
        self.b.walls.push(r);
        self.b.styles.push(style);
    }

    pub(super) fn loot(&mut self, p: Vec2, tier: u8) {
        let pos = self.at(p);
        self.b.loot.push(LootSpot { pos, tier });
    }

    pub(super) fn prop(&mut self, p: Vec2, kind: PropKind, tier: u8) {
        let pos = self.at(p);
        self.b.props.push(PropSpot { pos, kind, tier });
    }

    pub(super) fn hazard(&mut self, c: Vec2, size: Vec2) {
        let r = self.rect(c, size);
        self.b.hazards.push(r);
    }

    /// A horizontal wall from `x0` to `x1` at `y`, with doorways centered on `gaps`.
    pub(super) fn run_h(&mut self, x0: f32, x1: f32, y: f32, gaps: &[f32]) {
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
    pub(super) fn run_v(&mut self, y0: f32, y1: f32, x: f32, gaps: &[f32]) {
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
    pub(super) fn building(
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
    pub(super) fn corner(&mut self, interior: Rect, inset: f32) -> Vec2 {
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
    pub(super) fn world(&self, local: Rect) -> Rect {
        Rect::from_corners(self.at(local.min), self.at(local.max))
    }

    /// Drop a solid somewhere in `area` (world) that leaves room to walk around it.
    pub(super) fn scatter_solid(&mut self, area: Rect, size: Vec2, style: WallStyle) -> bool {
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
    pub(super) fn scatter_prop(
        &mut self,
        area: Rect,
        kind: PropKind,
        tier: u8,
        outdoor: bool,
    ) -> bool {
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
    pub(super) fn scatter_barrels(&mut self, area: Rect, outdoor: bool) {
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

    pub(super) fn scatter_loot(&mut self, area: Rect, tier: u8) -> bool {
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

    pub(super) fn next_tier(&self) -> u8 {
        (self.tier + 1).min(3)
    }
}

pub(super) fn overlaps(a: Rect, b: Rect) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}
