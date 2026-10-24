//! Broadphase grid over wall rects, for fast collision and line-of-sight queries.

use super::*;

pub(super) const INDEX_CELL: f32 = 256.0;

/// Uniform grid of wall indices, so collision and line-of-sight only test nearby walls.
pub(super) struct WallIndex {
    pub(super) origin: Vec2,
    pub(super) cols: i32,
    pub(super) rows: i32,
    pub(super) cells: Vec<Vec<u16>>,
}

impl WallIndex {
    pub(super) fn new(walls: &[Rect]) -> Self {
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

    pub(super) fn span(&self, min: Vec2, max: Vec2) -> (i32, i32, i32, i32) {
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
    pub(super) fn query(&self, walls: &[Rect], min: Vec2, max: Vec2, mut f: impl FnMut(&Rect)) {
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
