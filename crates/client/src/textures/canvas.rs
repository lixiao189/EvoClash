//! The pixel canvas sprites are painted on, plus colour and noise helpers.

use super::*;

pub(super) type Rgba = [f32; 4];

pub(super) fn rgb(r: f32, g: f32, b: f32) -> Rgba {
    [r, g, b, 1.0]
}

pub(super) fn shade(c: Rgba, k: f32) -> Rgba {
    [
        (c[0] * k).min(1.0),
        (c[1] * k).min(1.0),
        (c[2] * k).min(1.0),
        c[3],
    ]
}

pub(super) fn alpha(c: Rgba, a: f32) -> Rgba {
    [c[0], c[1], c[2], a]
}

pub(super) fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

pub(super) const OUTLINE: Rgba = [0.06, 0.05, 0.05, 1.0];

/// Integer hash in [0, 1). Stable across runs so the art never changes between launches.
pub(super) fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// Smooth value noise that tiles every `period` cells.
pub(super) fn noise(x: f32, y: f32, period: i32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let cell = |dx: i32, dy: i32| {
        hash(
            (x0 as i32 + dx).rem_euclid(period),
            (y0 as i32 + dy).rem_euclid(period),
            seed,
        )
    };
    let top = cell(0, 0) + (cell(1, 0) - cell(0, 0)) * sx;
    let bottom = cell(0, 1) + (cell(1, 1) - cell(0, 1)) * sx;
    top + (bottom - top) * sy
}

#[derive(Clone)]
/// A tiny RGBA painter. Origin top-left, y down, pixel centers at +0.5.
pub(super) struct Canvas {
    pub(super) w: u32,
    pub(super) h: u32,
    pub(super) px: Vec<Rgba>,
}

impl Canvas {
    pub(super) fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![[0.0; 4]; (w * h) as usize],
        }
    }

    pub(super) fn filled(w: u32, h: u32, f: impl Fn(i32, i32) -> Rgba) -> Self {
        let mut c = Self::new(w, h);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                c.px[(y as u32 * w + x as u32) as usize] = f(x, y);
            }
        }
        c
    }

    pub(super) fn get(&self, x: i32, y: i32) -> Rgba {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return [0.0; 4];
        }
        self.px[(y as u32 * self.w + x as u32) as usize]
    }

    /// Alpha-blend a pixel; out-of-bounds writes are dropped.
    pub(super) fn put(&mut self, x: i32, y: i32, c: Rgba) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let dst = &mut self.px[(y as u32 * self.w + x as u32) as usize];
        let a = c[3];
        let out_a = a + dst[3] * (1.0 - a);
        if out_a <= 0.0 {
            return;
        }
        for i in 0..3 {
            dst[i] = (c[i] * a + dst[i] * dst[3] * (1.0 - a)) / out_a;
        }
        dst[3] = out_a;
    }

    /// Erase a pixel back to transparent.
    pub(super) fn clear(&mut self, x: i32, y: i32) {
        if x >= 0 && y >= 0 && x < self.w as i32 && y < self.h as i32 {
            self.px[(y as u32 * self.w + x as u32) as usize] = [0.0; 4];
        }
    }

    /// Like `put`, but wraps around the edges (for seamless tiles).
    pub(super) fn put_wrap(&mut self, x: i32, y: i32, c: Rgba) {
        self.put(x.rem_euclid(self.w as i32), y.rem_euclid(self.h as i32), c);
    }

    pub(super) fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.put(x, y, c);
            }
        }
    }

    pub(super) fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: Rgba) {
        for y in (cy - ry).floor() as i32..=(cy + ry).ceil() as i32 {
            for x in (cx - rx).floor() as i32..=(cx + rx).ceil() as i32 {
                let dx = (x as f32 + 0.5 - cx) / rx;
                let dy = (y as f32 + 0.5 - cy) / ry;
                if dx * dx + dy * dy <= 1.0 {
                    self.put(x, y, c);
                }
            }
        }
    }

    pub(super) fn disc(&mut self, cx: f32, cy: f32, r: f32, c: Rgba) {
        self.ellipse(cx, cy, r, r, c);
    }

    pub(super) fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba) {
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let x = x0 + (x1 - x0) * t;
            let y = y0 + (y1 - y0) * t;
            self.put(x.floor() as i32, y.floor() as i32, c);
        }
    }

    /// Classic pixel-art outline: paint every empty pixel that touches an opaque one.
    pub(super) fn outline(&mut self, c: Rgba) {
        let mut edge = Vec::new();
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                if self.get(x, y)[3] > 0.3 {
                    continue;
                }
                let touches = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| self.get(x + dx, y + dy)[3] > 0.5);
                if touches {
                    edge.push((x, y));
                }
            }
        }
        for (x, y) in edge {
            self.put(x, y, c);
        }
    }

    /// Light from the top-left: brighten opaque pixels near that edge, darken the far side.
    pub(super) fn bevel(&mut self, cx: f32, cy: f32, r: f32, strength: f32) {
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let i = (y as u32 * self.w + x as u32) as usize;
                if self.px[i][3] < 0.5 {
                    continue;
                }
                let d = Vec2::new(x as f32 + 0.5 - cx, y as f32 + 0.5 - cy) / r;
                let light = 1.0 - (d.x + d.y) * 0.5 * strength;
                self.px[i] = shade(self.px[i], light.clamp(0.55, 1.35));
            }
        }
    }

    pub(super) fn into_image(self, nearest: bool) -> Image {
        let data = self
            .px
            .iter()
            .flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
            .collect();
        let mut image = Image::new(
            Extent3d {
                width: self.w,
                height: self.h,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        if nearest {
            image.sampler = ImageSampler::nearest();
        }
        image
    }
}
