//! Procedurally painted pixel art. Everything is generated at startup, so the client stays a
//! single binary with no asset folder to ship or lose.
//!
//! Sprites are authored at half resolution and drawn at 2x with nearest sampling. Characters,
//! weapons and monsters face +X; the renderer rotates them toward where they're heading.

use std::f32::consts::{PI, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use evoclash_game::items::{ArmorKind, Item, WeaponKind};

/// World pixels per texture pixel.
pub const PIXEL: f32 = 2.0;

#[derive(Resource)]
pub struct Textures {
    pub ground: Handle<Image>,
    pub vault_floor: Handle<Image>,
    pub trefoil: Handle<Image>,
    /// Brick face of a wall, seen from the front.
    pub wall_front: Handle<Image>,
    /// Cracked concrete cap of a wall, seen from above.
    pub wall_top: Handle<Image>,
    /// Flat ground stains.
    pub decals: Vec<Handle<Image>>,
    /// Upright clutter (dead grass, bushes, rocks, bones), depth-sorted like everything else.
    pub decor: Vec<Handle<Image>>,
    pub shadow: Handle<Image>,
    pub glow: Handle<Image>,
    survivor: Vec<SurvivorFrame>,
    pub hand: Handle<Image>,
    /// Side view, two skitter frames.
    pub roach: [Handle<Image>; 2],
    pub pod: Handle<Image>,
    pub spore: Handle<Image>,
    pub splat: Handle<Image>,
    weapons: [(Handle<Image>, Vec2); 6],
    armors: [Handle<Image>; 3],
    pub medkit: Handle<Image>,
    pub emp: Handle<Image>,
}

impl Textures {
    pub fn survivor(&self, facing: Facing, frame: usize) -> &SurvivorFrame {
        &self.survivor[facing.index() * WALK_FRAMES + frame % WALK_FRAMES]
    }

    /// Image and its size in texture pixels.
    pub fn weapon(&self, kind: WeaponKind) -> (Handle<Image>, Vec2) {
        self.weapons[weapon_index(kind)].clone()
    }

    pub fn item(&self, item: Item) -> Handle<Image> {
        match item {
            Item::Weapon { kind, .. } => self.weapon(kind).0,
            Item::Armor { kind, .. } => self.armors[armor_index(kind)].clone(),
            Item::Emp => self.emp.clone(),
            Item::Medkit => self.medkit.clone(),
        }
    }
}

fn weapon_index(kind: WeaponKind) -> usize {
    match kind {
        WeaponKind::Stick => 0,
        WeaponKind::Bow => 1,
        WeaponKind::PipeShotgun => 2,
        WeaponKind::Rifle => 3,
        WeaponKind::Laser => 4,
        WeaponKind::Railgun => 5,
    }
}

fn armor_index(kind: ArmorKind) -> usize {
    match kind {
        ArmorKind::None | ArmorKind::Scrap => 0,
        ArmorKind::Kevlar => 1,
        ArmorKind::PowerArmor => 2,
    }
}

pub fn build(images: &mut Assets<Image>) -> Textures {
    let mut add = |canvas: Canvas, nearest: bool| images.add(canvas.into_image(nearest));
    let weapons = [
        WeaponKind::Stick,
        WeaponKind::Bow,
        WeaponKind::PipeShotgun,
        WeaponKind::Rifle,
        WeaponKind::Laser,
        WeaponKind::Railgun,
    ]
    .map(|kind| {
        let canvas = paint_weapon(kind);
        let size = Vec2::new(canvas.w as f32, canvas.h as f32);
        (add(canvas, true), size)
    });
    Textures {
        ground: add(paint_ground(), true),
        vault_floor: add(paint_vault_floor(), true),
        trefoil: add(paint_trefoil(), true),
        wall_front: add(paint_wall(), true),
        wall_top: add(paint_wall_top(), true),
        decals: (0..6).map(|i| add(paint_decal(i), true)).collect(),
        decor: (0..5).map(|i| add(paint_decor(i), true)).collect(),
        shadow: add(paint_soft_disc(32, [0.0, 0.0, 0.0], 1.6), false),
        glow: add(paint_soft_disc(32, [1.0, 1.0, 1.0], 2.2), false),
        survivor: Facing::ALL
            .iter()
            .flat_map(|facing| (0..WALK_FRAMES).map(move |frame| (*facing, frame)))
            .map(|(facing, frame)| {
                let (jacket, gear) = paint_survivor(facing, frame);
                SurvivorFrame {
                    jacket: add(jacket, true),
                    gear: add(gear, true),
                }
            })
            .collect(),
        hand: add(paint_hand(), true),
        roach: [0, 1].map(|frame| add(paint_roach(frame), true)),
        pod: add(paint_pod(), true),
        spore: add(paint_spore(), false),
        splat: add(paint_splat(), true),
        weapons,
        armors: [ArmorKind::Scrap, ArmorKind::Kevlar, ArmorKind::PowerArmor]
            .map(|kind| add(paint_armor(kind), true)),
        medkit: add(paint_medkit(), true),
        emp: add(paint_emp(), true),
    }
}

// ---------------------------------------------------------------- canvas

type Rgba = [f32; 4];

fn rgb(r: f32, g: f32, b: f32) -> Rgba {
    [r, g, b, 1.0]
}

fn shade(c: Rgba, k: f32) -> Rgba {
    [
        (c[0] * k).min(1.0),
        (c[1] * k).min(1.0),
        (c[2] * k).min(1.0),
        c[3],
    ]
}

fn alpha(c: Rgba, a: f32) -> Rgba {
    [c[0], c[1], c[2], a]
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

const OUTLINE: Rgba = [0.06, 0.05, 0.05, 1.0];

/// Integer hash in [0, 1). Stable across runs so the art never changes between launches.
fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// Smooth value noise that tiles every `period` cells.
fn noise(x: f32, y: f32, period: i32, seed: u32) -> f32 {
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

/// A tiny RGBA painter. Origin top-left, y down, pixel centers at +0.5.
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<Rgba>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![[0.0; 4]; (w * h) as usize],
        }
    }

    fn filled(w: u32, h: u32, f: impl Fn(i32, i32) -> Rgba) -> Self {
        let mut c = Self::new(w, h);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                c.px[(y as u32 * w + x as u32) as usize] = f(x, y);
            }
        }
        c
    }

    fn get(&self, x: i32, y: i32) -> Rgba {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return [0.0; 4];
        }
        self.px[(y as u32 * self.w + x as u32) as usize]
    }

    /// Alpha-blend a pixel; out-of-bounds writes are dropped.
    fn put(&mut self, x: i32, y: i32, c: Rgba) {
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
    fn clear(&mut self, x: i32, y: i32) {
        if x >= 0 && y >= 0 && x < self.w as i32 && y < self.h as i32 {
            self.px[(y as u32 * self.w + x as u32) as usize] = [0.0; 4];
        }
    }

    /// Like `put`, but wraps around the edges (for seamless tiles).
    fn put_wrap(&mut self, x: i32, y: i32, c: Rgba) {
        self.put(x.rem_euclid(self.w as i32), y.rem_euclid(self.h as i32), c);
    }

    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.put(x, y, c);
            }
        }
    }

    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: Rgba) {
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

    fn disc(&mut self, cx: f32, cy: f32, r: f32, c: Rgba) {
        self.ellipse(cx, cy, r, r, c);
    }

    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba) {
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let x = x0 + (x1 - x0) * t;
            let y = y0 + (y1 - y0) * t;
            self.put(x.floor() as i32, y.floor() as i32, c);
        }
    }

    /// Classic pixel-art outline: paint every empty pixel that touches an opaque one.
    fn outline(&mut self, c: Rgba) {
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
    fn bevel(&mut self, cx: f32, cy: f32, r: f32, strength: f32) {
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

    fn into_image(self, nearest: bool) -> Image {
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

// ---------------------------------------------------------------- arena

fn paint_ground() -> Canvas {
    let base = rgb(0.36, 0.22, 0.16);
    let mut c = Canvas::filled(64, 64, |x, y| {
        let (fx, fy) = (x as f32, y as f32);
        let broad = noise(fx / 16.0, fy / 16.0, 4, 1);
        let mid = noise(fx / 6.4, fy / 6.4, 10, 2);
        let grain = hash(x, y, 3) - 0.5;
        let k = 0.78 + broad * 0.22 + mid * 0.14 + grain * 0.07;
        // Patches of ashy gray dust.
        let ash = (noise(fx / 10.67, fy / 10.67, 6, 4) - 0.55).max(0.0) * 1.6;
        // Patches of violet ash, like the rust-red badlands they settle on.
        mix(shade(base, k), rgb(0.27, 0.2, 0.25), ash)
    });
    // Pebbles.
    for i in 0..26 {
        let x = (hash(i, 0, 10) * 64.0) as i32;
        let y = (hash(i, 1, 10) * 64.0) as i32;
        let light = hash(i, 2, 10) > 0.5;
        let col = if light {
            rgb(0.5, 0.34, 0.25)
        } else {
            rgb(0.2, 0.12, 0.1)
        };
        c.put_wrap(x, y, col);
        if hash(i, 3, 10) > 0.6 {
            c.put_wrap(x + 1, y, col);
            c.put_wrap(x, y + 1, shade(col, 0.7));
        }
    }
    // Hairline cracks: short random walks.
    for i in 0..2 {
        let (mut x, mut y) = (
            (hash(i, 0, 20) * 64.0) as i32,
            (hash(i, 1, 20) * 64.0) as i32,
        );
        for step in 0..18 {
            c.put_wrap(x, y, [0.16, 0.09, 0.07, 0.45]);
            let r = hash(i, step, 21);
            if r < 0.45 {
                x += 1;
            } else if r < 0.7 {
                y += 1;
            } else if r < 0.85 {
                y -= 1;
            } else {
                x -= 1;
            }
        }
    }
    c
}

fn paint_vault_floor() -> Canvas {
    let plate = rgb(0.25, 0.25, 0.24);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let n = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 30);
        let mut col = shade(plate, 0.9 + n * 0.18 + (hash(x, y, 31) - 0.5) * 0.05);
        // Rust bloom.
        let rust = (noise(x as f32 / 5.33, y as f32 / 5.33, 6, 32) - 0.6).max(0.0) * 2.2;
        col = mix(col, rgb(0.42, 0.24, 0.12), rust);
        if x == 0 || y == 0 {
            col = shade(plate, 0.45);
        } else if x == 1 || y == 1 {
            col = shade(col, 1.25);
        }
        col
    });
    for (x, y) in [(4, 4), (27, 4), (4, 27), (27, 27)] {
        c.put(x, y, rgb(0.42, 0.42, 0.4));
        c.put(x + 1, y + 1, rgb(0.12, 0.12, 0.12));
    }
    // Grating slots in the middle of each plate.
    for slot in 0..4 {
        let y = 10 + slot * 4;
        c.rect(9, y, 22, y + 1, rgb(0.08, 0.08, 0.08));
        c.rect(9, y + 2, 22, y + 2, shade(plate, 1.2));
    }
    c
}

fn paint_trefoil() -> Canvas {
    let mut c = Canvas::new(48, 48);
    let ink = rgb(0.95, 0.75, 0.12);
    for y in 0..48 {
        for x in 0..48 {
            let d = Vec2::new(x as f32 + 0.5 - 24.0, y as f32 + 0.5 - 24.0);
            let r = d.length();
            let a = (d.to_angle() + PI / 2.0).rem_euclid(TAU / 3.0);
            let blade = (7.0..21.0).contains(&r) && a < TAU / 6.0;
            if r < 4.5 || blade {
                c.put(x, y, ink);
            } else if (22.0..23.5).contains(&r) {
                c.put(x, y, alpha(ink, 0.8));
            }
        }
    }
    c
}

fn paint_wall() -> Canvas {
    let concrete = rgb(0.42, 0.39, 0.35);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let row = y / 8;
        let offset = if row % 2 == 0 { 0 } else { 8 };
        let bx = (x + offset).rem_euclid(32) / 16;
        let (lx, ly) = ((x + offset).rem_euclid(16), y % 8);
        if lx == 0 || ly == 0 {
            return rgb(0.16, 0.15, 0.14);
        }
        let tone = 0.82 + hash(bx, row, 40) * 0.26;
        let grain = (hash(x, y, 41) - 0.5) * 0.08;
        let mut col = shade(concrete, tone + grain);
        if ly == 1 || lx == 1 {
            col = shade(col, 1.18);
        } else if ly == 7 || lx == 15 {
            col = shade(col, 0.72);
        }
        // Soot and moss creeping up.
        let grime = (noise(x as f32 / 8.0, y as f32 / 8.0, 4, 42) - 0.55).max(0.0) * 1.8;
        mix(col, rgb(0.2, 0.22, 0.14), grime)
    });
    for i in 0..3 {
        let (mut x, mut y) = (
            (hash(i, 0, 43) * 32.0) as i32,
            (hash(i, 1, 43) * 32.0) as i32,
        );
        for step in 0..6 {
            c.put_wrap(x, y, rgb(0.13, 0.12, 0.11));
            x += 1;
            y += if hash(i, step, 44) > 0.5 { 1 } else { 0 };
        }
    }
    c
}

fn paint_wall_top() -> Canvas {
    let slab = rgb(0.6, 0.55, 0.48);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let n = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 45);
        let grain = (hash(x, y, 46) - 0.5) * 0.06;
        let mut col = shade(slab, 0.88 + n * 0.2 + grain);
        // Seams between cast slabs.
        if x % 16 == 0 || y % 16 == 0 {
            col = shade(slab, 0.68);
        }
        let moss = (noise(x as f32 / 5.33, y as f32 / 5.33, 6, 47) - 0.62).max(0.0) * 2.4;
        mix(col, rgb(0.42, 0.5, 0.3), moss)
    });
    for i in 0..3 {
        let (mut x, mut y) = (
            (hash(i, 0, 48) * 32.0) as i32,
            (hash(i, 1, 48) * 32.0) as i32,
        );
        for step in 0..7 {
            c.put_wrap(x, y, rgb(0.3, 0.27, 0.24));
            x += 1;
            y += if hash(i, step, 49) > 0.5 { 1 } else { -1 };
        }
    }
    c
}

/// Knee-high clutter. Bottom row is where it touches the ground.
fn paint_decor(variant: i32) -> Canvas {
    match variant {
        // Dead grass tuft.
        0 => {
            let mut c = Canvas::new(12, 10);
            let straw = rgb(0.72, 0.58, 0.32);
            for (i, x) in [2.0, 4.0, 5.5, 7.0, 9.0].into_iter().enumerate() {
                let lean = (i as f32 - 2.0) * 0.8;
                c.line(
                    x,
                    9.0,
                    x + lean,
                    2.0 + (i % 2) as f32 * 2.0,
                    shade(straw, 0.8 + i as f32 * 0.08),
                );
            }
            c
        }
        // Scraggly dead bush.
        1 => {
            let mut c = Canvas::new(16, 13);
            let twig = rgb(0.38, 0.26, 0.2);
            c.line(8.0, 12.0, 8.0, 6.0, twig);
            c.line(8.0, 8.0, 3.0, 3.0, twig);
            c.line(8.0, 7.0, 13.0, 2.0, twig);
            c.line(8.0, 6.0, 7.0, 1.0, twig);
            c.line(5.0, 5.0, 3.0, 6.0, twig);
            c.line(11.0, 4.0, 14.0, 5.0, twig);
            for (x, y) in [(3, 2), (13, 1), (7, 0), (14, 5), (2, 6)] {
                c.put(x, y, rgb(0.55, 0.62, 0.3));
            }
            c.outline(alpha(OUTLINE, 0.7));
            c
        }
        // Boulder.
        2 => {
            let mut c = Canvas::new(14, 10);
            c.ellipse(7.0, 6.0, 6.0, 4.0, rgb(0.5, 0.42, 0.38));
            c.ellipse(9.0, 4.5, 3.0, 2.5, rgb(0.5, 0.42, 0.38));
            c.bevel(7.0, 5.0, 6.0, 1.0);
            c.outline(OUTLINE);
            c
        }
        // Bleached skull.
        3 => {
            let mut c = Canvas::new(10, 9);
            let bone = rgb(0.9, 0.86, 0.76);
            c.disc(5.0, 4.0, 3.6, bone);
            c.rect(3, 6, 7, 7, bone);
            c.rect(3, 4, 3, 4, OUTLINE);
            c.rect(6, 4, 6, 4, OUTLINE);
            c.put(4, 7, shade(bone, 0.6));
            c.put(6, 7, shade(bone, 0.6));
            c.outline(OUTLINE);
            c
        }
        // Rusted tire half-buried.
        _ => {
            let mut c = Canvas::new(14, 9);
            c.ellipse(7.0, 6.0, 6.0, 4.5, rgb(0.15, 0.13, 0.13));
            c.ellipse(7.0, 6.0, 3.0, 2.0, [0.0; 4]);
            for y in 0..9 {
                for x in 0..14 {
                    let d = Vec2::new((x as f32 + 0.5 - 7.0) / 3.0, (y as f32 + 0.5 - 6.0) / 2.0);
                    if d.length_squared() <= 1.0 {
                        c.clear(x, y);
                    }
                }
            }
            c.rect(0, 7, 13, 8, rgb(0.36, 0.22, 0.16));
            c.outline(alpha(OUTLINE, 0.8));
            c
        }
    }
}

/// Rubble piles, scorch marks and oil stains scattered over the floor.
fn paint_decal(variant: i32) -> Canvas {
    let mut c = Canvas::new(24, 24);
    let seed = 50 + variant as u32;
    match variant % 3 {
        0 => {
            for i in 0..9 {
                let x = 5.0 + hash(i, 0, seed) * 14.0;
                let y = 5.0 + hash(i, 1, seed) * 14.0;
                let r = 1.2 + hash(i, 2, seed) * 2.4;
                let tone = 0.3 + hash(i, 3, seed) * 0.18;
                c.disc(x, y, r, rgb(tone, tone * 0.95, tone * 0.88));
                c.put(
                    x as i32,
                    y as i32 - 1,
                    rgb(tone * 1.4, tone * 1.35, tone * 1.25),
                );
            }
            c.outline(alpha(OUTLINE, 0.6));
        }
        1 => {
            for y in 0..24 {
                for x in 0..24 {
                    let d = Vec2::new(x as f32 - 11.5, y as f32 - 11.5).length() / 11.0;
                    let n = noise(x as f32 / 4.0, y as f32 / 4.0, 6, seed);
                    let a = ((1.0 - d) * 1.4 - n * 0.6).clamp(0.0, 0.75);
                    c.put(x, y, [0.04, 0.035, 0.03, a]);
                }
            }
        }
        _ => {
            for i in 0..5 {
                let x = 7.0 + hash(i, 0, seed) * 10.0;
                let y = 7.0 + hash(i, 1, seed) * 10.0;
                let (rx, ry) = (3.0 + hash(i, 2, seed) * 4.0, 2.0 + hash(i, 3, seed) * 3.0);
                c.ellipse(x, y, rx, ry, [0.07, 0.07, 0.09, 0.55]);
            }
            c.put(10, 9, [0.3, 0.25, 0.4, 0.5]);
            c.put(14, 13, [0.2, 0.35, 0.35, 0.5]);
        }
    }
    c
}

/// Radial falloff, used for drop shadows and loot glows.
fn paint_soft_disc(size: u32, color: [f32; 3], falloff: f32) -> Canvas {
    let half = size as f32 / 2.0;
    Canvas::filled(size, size, |x, y| {
        let d = Vec2::new(x as f32 + 0.5 - half, y as f32 + 0.5 - half).length() / half;
        let a = (1.0 - d).max(0.0).powf(falloff);
        [color[0], color[1], color[2], a]
    })
}

// ---------------------------------------------------------------- survivors

/// Survivor canvas, in texture pixels. Feet touch the bottom; the sprite stands upright.
pub const SURVIVOR: UVec2 = UVec2::new(20, 30);
pub const WALK_FRAMES: usize = 4;

/// Which way a survivor sprite faces. `Side` faces right; flip it for left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    Down,
    Up,
    Side,
}

impl Facing {
    const ALL: [Facing; 3] = [Facing::Down, Facing::Up, Facing::Side];

    fn index(self) -> usize {
        match self {
            Facing::Down => 0,
            Facing::Up => 1,
            Facing::Side => 2,
        }
    }

    /// Pick a facing from an aim direction: mostly sideways reads as side-on.
    pub fn from_aim(aim: Vec2) -> (Facing, bool) {
        let flip = aim.x < 0.0;
        if aim.y.abs() < aim.x.abs() * 1.2 {
            (Facing::Side, flip)
        } else if aim.y > 0.0 {
            (Facing::Up, flip)
        } else {
            (Facing::Down, flip)
        }
    }
}

/// One animation frame: the jacket (tinted with the player's color) and everything else on top.
#[derive(Clone)]
pub struct SurvivorFrame {
    pub jacket: Handle<Image>,
    pub gear: Handle<Image>,
}

const SKIN: Rgba = [0.93, 0.73, 0.57, 1.0];
const SKIN_SHADE: Rgba = [0.76, 0.54, 0.42, 1.0];
const HELMET: Rgba = [0.4, 0.42, 0.29, 1.0];
const PANTS: Rgba = [0.42, 0.35, 0.25, 1.0];
const BOOTS: Rgba = [0.25, 0.16, 0.1, 1.0];
const PACK: Rgba = [0.42, 0.31, 0.19, 1.0];
const ROLL: Rgba = [0.62, 0.52, 0.36, 1.0];
const SCARF: Rgba = [0.75, 0.22, 0.17, 1.0];
const LENS: Rgba = [0.35, 0.85, 0.9, 1.0];
const STRAP: Rgba = [0.2, 0.14, 0.09, 1.0];

/// Paint one survivor frame. Frames 1 and 3 are mid-stride (opposite legs), 0 and 2 are passing.
fn paint_survivor(facing: Facing, frame: usize) -> (Canvas, Canvas) {
    let (w, h) = (SURVIVOR.x, SURVIVOR.y);
    let mut jacket = Canvas::new(w, h);
    let mut gear = Canvas::new(w, h);
    let stride: i32 = match frame {
        1 => 1,
        3 => -1,
        _ => 0,
    };
    // The upper body dips a pixel on each stride.
    let bob = stride.abs();
    let cloth = rgb(0.93, 0.93, 0.93);

    // Legs and boots.
    match facing {
        Facing::Down | Facing::Up => {
            for (x, lift) in [(6, stride.max(0)), (11, (-stride).max(0))] {
                gear.rect(x, 21, x + 2, 25 - lift, PANTS);
                gear.rect(x + 2, 21, x + 2, 25 - lift, shade(PANTS, 0.78));
                gear.rect(x, 26 - lift, x + 2, 27 - lift, BOOTS);
            }
        }
        Facing::Side => {
            // Back leg darker; both swing around the hip at x = 9.
            for (dx, tone) in [(-stride * 2, 0.72), (stride * 2, 1.0)] {
                let x = 8 + dx;
                gear.rect(x, 21, x + 2, 25, shade(PANTS, tone));
                gear.rect(x, 26, x + 3, 27, shade(BOOTS, tone));
            }
        }
    }

    let y0 = bob; // shift for everything above the hips
    match facing {
        Facing::Down | Facing::Up => {
            // Torso with rounded shoulders, sleeves hanging at the sides.
            jacket.rect(5, 13 + y0, 14, 21, cloth);
            jacket.clear(5, 13 + y0);
            jacket.clear(14, 13 + y0);
            jacket.rect(3, 14 + y0, 4, 19 + y0, shade(cloth, 0.85));
            jacket.rect(15, 14 + y0, 16, 19 + y0, shade(cloth, 0.85));
            jacket.rect(13, 14 + y0, 14, 21, shade(cloth, 0.8));
            jacket.rect(6, 14 + y0, 6, 20, shade(cloth, 1.08));
            gear.rect(3, 20 + y0, 4, 21 + y0, SKIN);
            gear.rect(15, 20 + y0, 16, 21 + y0, SKIN);
            gear.rect(5, 21, 14, 21, STRAP);
        }
        Facing::Side => {
            jacket.rect(6, 13 + y0, 12, 21, cloth);
            jacket.rect(11, 13 + y0, 12, 21, shade(cloth, 0.8));
            jacket.clear(6, 13 + y0);
            // One arm swings opposite the front leg.
            let ax = 9 - stride;
            jacket.rect(ax, 14 + y0, ax + 2, 19 + y0, shade(cloth, 0.85));
            gear.rect(ax, 20 + y0, ax + 2, 21 + y0, SKIN);
            gear.rect(6, 21, 12, 21, STRAP);
        }
    }

    match facing {
        Facing::Down => {
            jacket.rect(7, 12 + y0, 12, 13 + y0, shade(cloth, 0.7));
            jacket.rect(9, 14 + y0, 10, 20, shade(cloth, 0.72));
            gear.rect(6, 13 + y0, 6, 20, STRAP);
            gear.rect(13, 13 + y0, 13, 20, STRAP);
            gear.rect(9, 21, 10, 21, rgb(0.78, 0.66, 0.3));
            paint_head_front(&mut gear, y0);
        }
        Facing::Up => {
            paint_head_back(&mut gear, y0);
            // Backpack with a blanket roll on top.
            gear.rect(6, 14 + y0, 13, 20, PACK);
            gear.rect(6, 14 + y0, 13, 15 + y0, shade(PACK, 0.78));
            gear.rect(12, 16 + y0, 13, 20, shade(PACK, 0.8));
            gear.rect(8, 17 + y0, 11, 18 + y0, shade(PACK, 1.2));
            gear.rect(5, 12 + y0, 14, 13 + y0, ROLL);
            gear.rect(5, 13 + y0, 14, 13 + y0, shade(ROLL, 0.75));
            gear.put(7, 12 + y0, STRAP);
            gear.put(12, 12 + y0, STRAP);
        }
        Facing::Side => {
            gear.rect(3, 13 + y0, 6, 20, PACK);
            gear.rect(3, 13 + y0, 3, 20, shade(PACK, 0.75));
            gear.rect(2, 12 + y0, 6, 13 + y0, ROLL);
            jacket.rect(7, 12 + y0, 11, 13 + y0, shade(cloth, 0.7));
            gear.line(7.0, 13.0 + y0 as f32, 7.0, 20.0, STRAP);
            paint_head_side(&mut gear, y0);
        }
    }
    jacket.bevel(9.5, 16.0, 7.0, 0.35);
    jacket.outline(OUTLINE);
    gear.outline(alpha(OUTLINE, 0.9));
    (jacket, gear)
}

fn helmet_dome(c: &mut Canvas, x0: i32, x1: i32, y0: i32) {
    c.rect(x0 + 2, 1 + y0, x1 - 2, 1 + y0, HELMET);
    c.rect(x0 + 1, 2 + y0, x1 - 1, 2 + y0, HELMET);
    c.rect(x0, 3 + y0, x1, 5 + y0, HELMET);
    c.rect(x0 + 2, 2 + y0, x0 + 4, 2 + y0, shade(HELMET, 1.35));
    c.rect(x0 + 1, 3 + y0, x0 + 2, 3 + y0, shade(HELMET, 1.25));
}

fn paint_head_front(c: &mut Canvas, y0: i32) {
    c.rect(6, 6 + y0, 13, 11 + y0, SKIN);
    c.rect(6, 6 + y0, 13, 6 + y0, SKIN_SHADE);
    c.rect(13, 7 + y0, 13, 11 + y0, SKIN_SHADE);
    helmet_dome(c, 5, 14, y0);
    c.rect(4, 6 + y0, 15, 6 + y0, shade(HELMET, 0.7));
    // Goggles pushed up on the helmet.
    c.rect(5, 4 + y0, 14, 4 + y0, STRAP);
    c.rect(7, 4 + y0, 8, 4 + y0, LENS);
    c.rect(11, 4 + y0, 12, 4 + y0, LENS);
    c.put(7, 4 + y0, rgb(0.9, 1.0, 1.0));
    c.put(11, 4 + y0, rgb(0.9, 1.0, 1.0));
    c.put(7, 8 + y0, OUTLINE);
    c.put(12, 8 + y0, OUTLINE);
    c.put(7, 7 + y0, SKIN_SHADE);
    c.put(12, 7 + y0, SKIN_SHADE);
    c.rect(9, 10 + y0, 10, 10 + y0, SKIN_SHADE);
    // Scarf over the chin.
    c.rect(6, 11 + y0, 13, 12 + y0, SCARF);
    c.rect(6, 12 + y0, 13, 12 + y0, shade(SCARF, 0.75));
}

fn paint_head_back(c: &mut Canvas, y0: i32) {
    c.rect(8, 9 + y0, 11, 11 + y0, SKIN_SHADE);
    helmet_dome(c, 5, 14, y0);
    c.rect(5, 6 + y0, 14, 8 + y0, HELMET);
    c.rect(4, 8 + y0, 15, 8 + y0, shade(HELMET, 0.7));
    c.rect(5, 4 + y0, 14, 4 + y0, STRAP);
    c.rect(7, 10 + y0, 12, 11 + y0, SCARF);
    c.rect(11, 12 + y0, 12, 13 + y0, shade(SCARF, 0.8));
}

fn paint_head_side(c: &mut Canvas, y0: i32) {
    c.rect(8, 6 + y0, 14, 11 + y0, SKIN);
    c.rect(8, 6 + y0, 10, 11 + y0, SKIN_SHADE);
    helmet_dome(c, 6, 14, y0);
    c.rect(6, 6 + y0, 9, 7 + y0, HELMET);
    // Brim sticks out forward.
    c.rect(6, 6 + y0, 16, 6 + y0, shade(HELMET, 0.7));
    c.rect(7, 4 + y0, 14, 4 + y0, STRAP);
    c.rect(12, 4 + y0, 13, 4 + y0, LENS);
    c.put(13, 4 + y0, rgb(0.9, 1.0, 1.0));
    c.put(12, 8 + y0, OUTLINE);
    c.put(15, 9 + y0, SKIN);
    c.rect(8, 11 + y0, 13, 12 + y0, SCARF);
    c.rect(6, 12 + y0, 7, 13 + y0, shade(SCARF, 0.8));
}

/// Glove drawn where a held weapon is gripped.
fn paint_hand() -> Canvas {
    let mut c = Canvas::new(6, 6);
    c.disc(3.0, 3.0, 1.8, BOOTS);
    c.put(3, 2, SKIN);
    c.outline(OUTLINE);
    c
}

fn paint_weapon(kind: WeaponKind) -> Canvas {
    match kind {
        WeaponKind::Stick => {
            let mut c = Canvas::new(16, 5);
            let wood = rgb(0.55, 0.38, 0.2);
            c.rect(1, 2, 14, 2, wood);
            c.rect(1, 1, 9, 1, shade(wood, 1.25));
            c.rect(10, 1, 14, 3, shade(wood, 0.9));
            c.put(6, 2, shade(wood, 0.6));
            c.put(12, 2, shade(wood, 0.6));
            c.outline(OUTLINE);
            c
        }
        WeaponKind::Bow => {
            let mut c = Canvas::new(8, 18);
            let wood = rgb(0.62, 0.45, 0.25);
            for i in 0..=24 {
                let t = i as f32 / 24.0;
                let a = (t - 0.5) * 2.4;
                let (x, y) = (2.0 + a.cos() * 4.0, 9.0 + a.sin() * 7.5);
                c.put(x as i32, y as i32, wood);
            }
            c.line(2.0, 1.5, 2.0, 16.5, rgb(0.85, 0.82, 0.72));
            c.outline(OUTLINE);
            c
        }
        WeaponKind::PipeShotgun => {
            let mut c = Canvas::new(16, 6);
            let pipe = rgb(0.5, 0.5, 0.48);
            c.rect(5, 1, 14, 2, pipe);
            c.rect(5, 1, 14, 1, shade(pipe, 1.3));
            c.rect(5, 3, 13, 3, shade(pipe, 0.75));
            c.rect(1, 2, 5, 4, rgb(0.45, 0.3, 0.17));
            c.rect(8, 1, 9, 3, rgb(0.75, 0.62, 0.3));
            c.outline(OUTLINE);
            c
        }
        WeaponKind::Rifle => {
            let mut c = Canvas::new(21, 7);
            let metal = rgb(0.24, 0.26, 0.24);
            c.rect(1, 2, 6, 3, rgb(0.3, 0.27, 0.2));
            c.rect(6, 1, 15, 3, metal);
            c.rect(6, 1, 15, 1, shade(metal, 1.5));
            c.rect(15, 2, 19, 2, shade(metal, 0.8));
            c.rect(10, 4, 11, 5, shade(metal, 0.7));
            c.rect(8, 0, 9, 0, rgb(0.15, 0.15, 0.15));
            c.outline(OUTLINE);
            c
        }
        WeaponKind::Laser => {
            let mut c = Canvas::new(19, 7);
            let shell = rgb(0.85, 0.88, 0.9);
            c.rect(1, 2, 15, 4, shell);
            c.rect(1, 2, 15, 2, shade(shell, 1.1));
            c.rect(1, 4, 15, 4, shade(shell, 0.7));
            c.rect(4, 3, 13, 3, rgb(0.2, 1.0, 1.0));
            c.rect(16, 2, 17, 4, rgb(0.15, 0.5, 0.55));
            c.put(17, 3, rgb(0.7, 1.0, 1.0));
            c.outline(OUTLINE);
            c
        }
        WeaponKind::Railgun => {
            let mut c = Canvas::new(26, 9);
            let rail = rgb(0.35, 0.33, 0.4);
            c.rect(1, 3, 6, 5, rgb(0.25, 0.22, 0.3));
            c.rect(6, 2, 24, 2, rail);
            c.rect(6, 6, 24, 6, rail);
            c.rect(6, 3, 20, 5, rgb(0.18, 0.1, 0.25));
            for x in [8, 11, 14, 17] {
                c.rect(x, 1, x + 1, 7, rgb(0.75, 0.4, 1.0));
                c.put(x, 1, rgb(1.0, 0.8, 1.0));
            }
            c.rect(21, 4, 24, 4, rgb(0.95, 0.7, 1.0));
            c.outline(OUTLINE);
            c
        }
    }
}

// ---------------------------------------------------------------- loot

fn paint_armor(kind: ArmorKind) -> Canvas {
    let mut c = Canvas::new(16, 16);
    let (base, trim) = match kind {
        ArmorKind::Kevlar => (rgb(0.3, 0.36, 0.3), rgb(0.2, 0.24, 0.2)),
        ArmorKind::PowerArmor => (rgb(0.8, 0.62, 0.22), rgb(0.35, 0.36, 0.4)),
        _ => (rgb(0.48, 0.4, 0.33), rgb(0.55, 0.32, 0.18)),
    };
    // Vest: torso with a neck cut-out and two shoulder straps.
    c.rect(3, 4, 12, 13, base);
    c.rect(2, 2, 5, 5, base);
    c.rect(10, 2, 13, 5, base);
    for y in 2..6 {
        for x in 6..10 {
            c.px[(y * 16 + x) as usize] = [0.0; 4];
        }
    }
    c.rect(3, 9, 12, 9, trim);
    c.bevel(8.0, 8.0, 7.0, 0.7);
    match kind {
        ArmorKind::Kevlar => {
            c.rect(4, 10, 6, 12, trim);
            c.rect(9, 10, 11, 12, trim);
        }
        ArmorKind::PowerArmor => {
            c.rect(6, 6, 9, 8, rgb(0.3, 0.9, 1.0));
            c.rect(1, 2, 3, 6, trim);
            c.rect(12, 2, 14, 6, trim);
        }
        _ => {
            for (x, y) in [(4, 5), (11, 5), (4, 12), (11, 12), (7, 11)] {
                c.put(x, y, rgb(0.75, 0.7, 0.62));
            }
        }
    }
    c.outline(OUTLINE);
    c
}

fn paint_medkit() -> Canvas {
    let mut c = Canvas::new(14, 13);
    let white = rgb(0.92, 0.9, 0.86);
    c.rect(1, 3, 12, 11, white);
    c.rect(1, 10, 12, 11, shade(white, 0.75));
    c.rect(5, 1, 8, 2, rgb(0.4, 0.4, 0.4));
    c.rect(6, 4, 7, 9, rgb(0.85, 0.15, 0.12));
    c.rect(4, 6, 9, 7, rgb(0.85, 0.15, 0.12));
    c.outline(OUTLINE);
    c
}

fn paint_emp() -> Canvas {
    let mut c = Canvas::new(13, 13);
    c.disc(6.5, 7.0, 5.0, rgb(0.22, 0.24, 0.28));
    c.bevel(6.5, 7.0, 5.0, 0.9);
    c.rect(2, 6, 10, 7, rgb(0.3, 0.75, 1.0));
    c.put(6, 6, rgb(0.85, 1.0, 1.0));
    c.rect(5, 1, 7, 2, rgb(0.55, 0.55, 0.5));
    c.put(8, 0, rgb(0.75, 0.7, 0.4));
    c.outline(OUTLINE);
    c
}

// ---------------------------------------------------------------- monsters

/// Side view, facing right. `frame` alternates which legs are planted.
fn paint_roach(frame: i32) -> Canvas {
    let mut c = Canvas::new(26, 15);
    let shell = rgb(0.55, 0.3, 0.14);
    let leg = rgb(0.3, 0.17, 0.09);
    // Legs: alternate tripods lean forward and back.
    for (i, x) in [6.0, 11.0, 16.0].into_iter().enumerate() {
        let lean = if (i as i32 + frame) % 2 == 0 {
            2.0
        } else {
            -2.0
        };
        c.line(x, 9.0, x + lean * 0.5, 11.5, leg);
        c.line(x + lean * 0.5, 11.5, x + lean, 13.0, leg);
    }
    // Antennae sweep up and forward.
    c.line(21.0, 6.0, 23.0, 2.0, rgb(0.45, 0.3, 0.17));
    c.line(23.0, 2.0, 25.0, 1.0, rgb(0.45, 0.3, 0.17));
    c.line(20.0, 6.0, 21.0, 1.0, rgb(0.45, 0.3, 0.17));
    // Abdomen, wing case and head.
    c.ellipse(11.0, 7.5, 9.0, 3.8, shell);
    c.ellipse(10.0, 6.0, 7.5, 2.5, shade(shell, 1.25));
    c.line(4.0, 7.0, 17.0, 6.0, shade(shell, 0.7));
    c.ellipse(19.5, 8.0, 2.8, 2.6, shade(shell, 0.7));
    c.put(20, 7, rgb(1.0, 0.3, 0.2));
    // Radioactive sores.
    for (x, y) in [(7, 6), (12, 5), (14, 8)] {
        c.put(x, y, rgb(0.6, 1.0, 0.35));
    }
    c.outline(OUTLINE);
    c
}

/// Upright bulb on a stalk, roots splayed over the ground, maw facing the viewer.
fn paint_pod() -> Canvas {
    let mut c = Canvas::new(32, 36);
    let flesh = rgb(0.55, 0.28, 0.46);
    let root = rgb(0.32, 0.17, 0.24);
    for (dx, len) in [(-9.0, 4.0), (-5.0, 5.0), (5.0, 5.0), (9.0, 4.0), (0.0, 3.0)] {
        c.line(16.0, 29.0, 16.0 + dx, 29.0 + len, root);
    }
    c.rect(13, 23, 18, 30, shade(flesh, 0.7));
    c.rect(13, 23, 14, 30, shade(flesh, 0.55));
    // Lumpy bulb.
    c.ellipse(16.0, 13.0, 12.5, 11.0, flesh);
    for i in 0..8 {
        let p = Vec2::from_angle(i as f32 / 8.0 * TAU) * Vec2::new(10.5, 9.0);
        c.disc(16.0 + p.x, 13.0 + p.y, 3.0, flesh);
    }
    c.bevel(16.0, 13.0, 12.0, 0.9);
    // Glowing veins.
    for i in 0..5 {
        let a = i as f32 / 5.0 * TAU - 0.4;
        let (d1, d2) = (Vec2::from_angle(a) * 5.0, Vec2::from_angle(a + 0.35) * 11.0);
        c.line(
            16.0 + d1.x,
            13.0 + d1.y,
            16.0 + d2.x,
            13.0 + d2.y,
            rgb(0.58, 0.92, 0.3),
        );
    }
    for (x, y) in [(8.0, 9.0), (24.0, 10.0), (11.0, 19.0), (22.0, 19.0)] {
        c.disc(x, y, 1.3, rgb(0.15, 0.06, 0.1));
        c.put(x as i32, y as i32, rgb(0.8, 1.0, 0.4));
    }
    // Maw.
    c.ellipse(16.0, 15.0, 5.0, 4.0, rgb(0.62, 0.95, 0.28));
    c.ellipse(16.0, 15.0, 3.8, 2.8, rgb(0.14, 0.04, 0.09));
    c.put(14, 14, rgb(0.9, 1.0, 0.6));
    c.outline(OUTLINE);
    c
}

fn paint_spore() -> Canvas {
    let half = 8.0;
    Canvas::filled(16, 16, |x, y| {
        let d = Vec2::new(x as f32 + 0.5 - half, y as f32 + 0.5 - half).length() / half;
        let core = (1.0 - d * 1.6).max(0.0);
        let body: f32 = if d < 0.62 { 1.0 } else { 0.0 };
        let halo = (1.0 - d).max(0.0).powf(2.0) * 0.6;
        let a = body.max(halo);
        alpha(mix(rgb(0.5, 0.9, 0.2), rgb(0.95, 1.0, 0.7), core), a)
    })
}

fn paint_splat() -> Canvas {
    let mut c = Canvas::new(28, 28);
    let goo = [0.32, 0.45, 0.12, 0.85];
    c.disc(14.0, 14.0, 6.0, goo);
    for i in 0..11 {
        let a = hash(i, 0, 70) * TAU;
        let r = 4.0 + hash(i, 1, 70) * 8.0;
        let p = Vec2::from_angle(a) * r;
        c.disc(14.0 + p.x, 14.0 + p.y, 1.0 + hash(i, 2, 70) * 2.2, goo);
    }
    c.disc(12.0, 12.0, 2.0, [0.55, 0.75, 0.25, 0.8]);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_in_unit_range_and_stable() {
        for i in 0..1000 {
            let h = hash(i, -i, 7);
            assert!((0.0..1.0).contains(&h));
            assert_eq!(h, hash(i, -i, 7));
        }
    }

    #[test]
    fn noise_tiles_seamlessly() {
        for y in 0..8 {
            let a = noise(0.0, y as f32 * 0.5, 4, 1);
            let b = noise(4.0, y as f32 * 0.5, 4, 1);
            assert!((a - b).abs() < 1e-6);
        }
    }

    /// Dev aid: `EVOCLASH_SPRITE_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview`
    /// writes every survivor walk frame (rows: down, up, side) plus the monsters, blown up 8x.
    #[test]
    #[ignore]
    fn preview_survivor() {
        let Ok(path) = std::env::var("EVOCLASH_SPRITE_PREVIEW") else {
            return;
        };
        let tint = [0.35, 0.6, 0.95];
        let zoom = 8;
        let (cw, ch) = (SURVIVOR.x as i32 + 8, 40);
        let (w, h) = (cw * WALK_FRAMES as i32 + 44, ch * 3);
        let mut sheet = Canvas::filled(w as u32, h as u32, |_, _| rgb(0.36, 0.22, 0.16));
        let blit = |sheet: &mut Canvas, src: &Canvas, dx: i32, dy: i32, tint: [f32; 3]| {
            for y in 0..src.h as i32 {
                for x in 0..src.w as i32 {
                    let p = src.get(x, y);
                    sheet.put(
                        dx + x,
                        dy + y,
                        [p[0] * tint[0], p[1] * tint[1], p[2] * tint[2], p[3]],
                    );
                }
            }
        };
        for (row, facing) in Facing::ALL.iter().enumerate() {
            for frame in 0..WALK_FRAMES {
                let (jacket, gear) = paint_survivor(*facing, frame);
                let (ox, oy) = (frame as i32 * cw + 4, row as i32 * ch + 4);
                blit(&mut sheet, &jacket, ox, oy, tint);
                blit(&mut sheet, &gear, ox, oy, [1.0; 3]);
            }
        }
        let x = cw * WALK_FRAMES as i32 + 4;
        blit(&mut sheet, &paint_roach(0), x, 6, [1.0; 3]);
        blit(&mut sheet, &paint_roach(1), x, 24, [1.0; 3]);
        blit(&mut sheet, &paint_pod(), x, 44, [1.0; 3]);
        for v in 0..5 {
            blit(
                &mut sheet,
                &paint_decor(v),
                x + 2 + (v % 2) * 18,
                84 + (v / 2) * 12,
                [1.0; 3],
            );
        }
        // 24-bit BMP, bottom-up rows.
        let (zw, zh) = (w * zoom, h * zoom);
        let stride = (zw * 3 + 3) & !3;
        let mut out = Vec::new();
        let size = 54 + stride * zh;
        out.extend(b"BM");
        out.extend((size as u32).to_le_bytes());
        out.extend([0u8; 4]);
        out.extend(54u32.to_le_bytes());
        out.extend(40u32.to_le_bytes());
        out.extend(zw.to_le_bytes());
        out.extend(zh.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(24u16.to_le_bytes());
        out.extend([0u8; 24]);
        for y in (0..zh).rev() {
            let mut line = Vec::new();
            for x in 0..zw {
                let p = sheet.get(x / zoom, y / zoom);
                line.extend([2, 1, 0].map(|i| (p[i].clamp(0.0, 1.0) * 255.0) as u8));
            }
            line.resize(stride as usize, 0);
            out.extend(line);
        }
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn sprites_have_opaque_pixels() {
        for canvas in [
            paint_survivor(Facing::Side, 1).0,
            paint_roach(0),
            paint_pod(),
            paint_medkit(),
        ] {
            assert!(canvas.px.iter().any(|p| p[3] > 0.9));
        }
    }
}
