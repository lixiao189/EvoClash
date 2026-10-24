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
use evoclash_game::map::{Floor, Roof, WallStyle};

/// World pixels per texture pixel.
pub const PIXEL: f32 = 2.0;

#[derive(Resource)]
pub struct Textures {
    pub ground: Handle<Image>,
    pub vault_floor: Handle<Image>,
    pub trefoil: Handle<Image>,
    /// Ground cover of city blocks and streets.
    pub pavement: Handle<Image>,
    pub oily_concrete: Handle<Image>,
    pub rad_ground: Handle<Image>,
    pub asphalt: Handle<Image>,
    floors: [Handle<Image>; 3],
    roofs: [Handle<Image>; 3],
    /// (face seen from the front, cap seen from above) for each wall style.
    walls: [WallSprites; 4],
    pub crate_box: Handle<Image>,
    pub barrel: Handle<Image>,
    pub safe: Handle<Image>,
    pub safe_open: Handle<Image>,
    /// Flat ground stains.
    pub decals: Vec<Handle<Image>>,
    /// Upright clutter (dead grass, bushes, rocks, bones), depth-sorted like everything else.
    pub decor: Vec<Handle<Image>>,
    pub shadow: Handle<Image>,
    pub glow: Handle<Image>,
    survivor: Vec<SurvivorFrame>,
    /// Sleeve reaching from the shoulder to the weapon grip; tinted like the jacket.
    pub arm: Handle<Image>,
    /// Side view, two skitter frames.
    pub roach: [Handle<Image>; 2],
    pub pod: Handle<Image>,
    pub spore: Handle<Image>,
    pub splat: Handle<Image>,
    weapons: [WeaponSprite; 6],
    armors: [Handle<Image>; 3],
    pub medkit: Handle<Image>,
    pub emp: Handle<Image>,
    /// Bullet streak: white, fading tail on the left, bright round head on the right.
    pub bolt: Handle<Image>,
    /// Crescent of a melee swing, centered, opening toward +X.
    pub slash: Handle<Image>,
    /// Soft expanding shock ring.
    pub shock: Handle<Image>,
    /// Ragged billowing fireball puff.
    pub blast: Handle<Image>,
    /// Aiming reticle with four ticks.
    pub reticle: Handle<Image>,
}

#[derive(Clone)]
pub struct WallSprites {
    pub front: Handle<Image>,
    pub top: Handle<Image>,
}

impl Textures {
    pub fn wall(&self, style: WallStyle) -> &WallSprites {
        &self.walls[match style {
            WallStyle::Brick => 0,
            WallStyle::Metal => 1,
            WallStyle::Rock => 2,
            WallStyle::Sandbag => 3,
        }]
    }

    pub fn floor(&self, floor: Floor) -> Handle<Image> {
        self.floors[match floor {
            Floor::Planks => 0,
            Floor::Tiles => 1,
            Floor::Concrete => 2,
        }]
        .clone()
    }

    pub fn roof(&self, roof: Roof) -> Handle<Image> {
        self.roofs[match roof {
            Roof::Tin => 0,
            Roof::Tar => 1,
            Roof::Concrete => 2,
        }]
        .clone()
    }

    pub fn survivor(&self, facing: Facing, frame: usize) -> &SurvivorFrame {
        &self.survivor[facing.index() * WALK_FRAMES + frame % WALK_FRAMES]
    }

    pub fn weapon(&self, kind: WeaponKind) -> &WeaponSprite {
        &self.weapons[weapon_index(kind)]
    }

    pub fn item(&self, item: Item) -> Handle<Image> {
        match item {
            Item::Weapon { kind, .. } => self.weapon(kind).icon.clone(),
            Item::Armor { kind, .. } => self.armors[armor_index(kind)].clone(),
            Item::Emp => self.emp.clone(),
            Item::Medkit => self.medkit.clone(),
        }
    }
}

/// A weapon as carried (with the survivor's fists gripping it) and as loot lying on the ground.
#[derive(Clone)]
pub struct WeaponSprite {
    pub held: Handle<Image>,
    pub icon: Handle<Image>,
    /// Size in texture pixels.
    pub size: Vec2,
    /// Where the rear hand grips, in texture pixels from the top-left. The renderer pivots here.
    pub grip: Vec2,
}

/// Where to put a held weapon and the sleeve reaching for it, so the rear fist lands on `pivot`.
/// `size` and `grip` come from [`WeaponSprite`]; the weapon is drawn flipped vertically when it
/// points left, which mirrors the grip too.
pub fn held_layout(pivot: Vec2, aim: Vec2, size: Vec2, grip: Vec2) -> (Vec2, Vec2) {
    let mut g = Vec2::new(grip.x - size.x / 2.0, size.y / 2.0 - grip.y) * PIXEL;
    if aim.x < 0.0 {
        g.y = -g.y;
    }
    (pivot - aim.rotate(g), pivot - aim * 3.0 * PIXEL)
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
        let (icon, held, grip) = paint_weapon(kind);
        WeaponSprite {
            size: Vec2::new(held.w as f32, held.h as f32),
            held: add(held, true),
            icon: add(icon, true),
            grip,
        }
    });
    Textures {
        ground: add(paint_ground(), true),
        vault_floor: add(paint_vault_floor(), true),
        trefoil: add(paint_trefoil(), true),
        pavement: add(paint_pavement(), true),
        oily_concrete: add(paint_oily_concrete(), true),
        rad_ground: add(paint_rad_ground(), true),
        asphalt: add(paint_asphalt(), true),
        floors: [paint_planks(), paint_tiles(), paint_floor_concrete()].map(|c| add(c, true)),
        roofs: [Roof::Tin, Roof::Tar, Roof::Concrete].map(|r| add(paint_roof(r), true)),
        walls: [
            (paint_wall(), paint_wall_top()),
            (paint_metal(), paint_metal_top()),
            (paint_rock(false), paint_rock(true)),
            (paint_sandbags(false), paint_sandbags(true)),
        ]
        .map(|(front, top)| WallSprites {
            front: add(front, true),
            top: add(top, true),
        }),
        crate_box: add(paint_crate(), true),
        barrel: add(paint_barrel(), true),
        safe: add(paint_safe(false), true),
        safe_open: add(paint_safe(true), true),
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
        arm: add(paint_arm(), true),
        roach: [0, 1].map(|frame| add(paint_roach(frame), true)),
        pod: add(paint_pod(), true),
        spore: add(paint_spore(), false),
        splat: add(paint_splat(), true),
        weapons,
        armors: [ArmorKind::Scrap, ArmorKind::Kevlar, ArmorKind::PowerArmor]
            .map(|kind| add(paint_armor(kind), true)),
        medkit: add(paint_medkit(), true),
        emp: add(paint_emp(), true),
        bolt: add(paint_bolt(), false),
        slash: add(paint_slash(), false),
        shock: add(paint_shock(), false),
        blast: add(paint_blast(), false),
        reticle: add(paint_reticle(), true),
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

#[derive(Clone)]
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

// ---------------------------------------------------------------- effects

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn paint_bolt() -> Canvas {
    let (w, h) = (48u32, 12u32);
    Canvas::filled(w, h, |x, y| {
        let u = (x as f32 + 0.5) / w as f32;
        let v = ((y as f32 + 0.5) / h as f32 - 0.5).abs() * 2.0;
        // Streak narrows toward the tail; the head is a round, hot bead.
        let half = 0.15 + 0.35 * u;
        let streak = (1.0 - v / half).max(0.0).powf(1.2) * u.powf(1.4);
        let head = Vec2::new((u - 0.86) * 3.4, v).length();
        let bead = (1.0 - head).max(0.0).powf(1.5);
        [1.0, 1.0, 1.0, streak.max(bead).min(1.0)]
    })
}

fn paint_slash() -> Canvas {
    let n = 96u32;
    let half = n as f32 / 2.0;
    Canvas::filled(n, n, |x, y| {
        let p = Vec2::new(x as f32 + 0.5 - half, half - (y as f32 + 0.5)) / half;
        let (r, ang) = (p.length(), p.to_angle().abs());
        let reach = 0.95 - 0.1 * (ang / 1.0);
        let band = (1.0 - (r - reach).abs() / (0.12 + 0.28 * (1.0 - ang / 1.1).max(0.0))).max(0.0);
        let taper = smooth((1.1 - ang) / 0.7);
        let grain = 0.75 + 0.25 * noise(x as f32 / 5.0, y as f32 / 5.0, 19, 21);
        [1.0, 1.0, 1.0, (band * taper * grain).min(1.0)]
    })
}

fn paint_shock() -> Canvas {
    let n = 128u32;
    let half = n as f32 / 2.0;
    Canvas::filled(n, n, |x, y| {
        let d = Vec2::new(x as f32 + 0.5 - half, y as f32 + 0.5 - half).length() / half;
        let ring = (1.0 - (d - 0.85).abs() / 0.15).max(0.0).powf(1.6);
        let haze = (1.0 - d).max(0.0) * 0.12;
        [1.0, 1.0, 1.0, (ring + haze).min(1.0)]
    })
}

fn paint_blast() -> Canvas {
    let n = 64u32;
    let half = n as f32 / 2.0;
    Canvas::filled(n, n, |x, y| {
        let d = Vec2::new(x as f32 + 0.5 - half, y as f32 + 0.5 - half).length() / half;
        let wobble = noise(x as f32 / 8.0, y as f32 / 8.0, 8, 33) - 0.5;
        let fine = noise(x as f32 / 3.0, y as f32 / 3.0, 21, 34) - 0.5;
        let edge = 0.74 + wobble * 0.45 + fine * 0.07;
        let a = smooth((edge - d) / 0.28);
        // Darker, sootier toward the rim.
        let k = 1.0 - 0.4 * d - 0.12 * (fine + 0.5);
        [k, k, k, a]
    })
}

fn paint_reticle() -> Canvas {
    let mut c = Canvas::new(20, 20);
    let white = rgb(1.0, 1.0, 1.0);
    for (a, b) in [(2, 5), (14, 17)] {
        c.rect(9, a, 10, b, white);
        c.rect(a, 9, b, 10, white);
    }
    c
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
            // Shoulders only: the arms reach forward to the weapon (see `paint_arm`).
            jacket.rect(4, 14 + y0, 4, 16 + y0, shade(cloth, 0.85));
            jacket.rect(15, 14 + y0, 15, 16 + y0, shade(cloth, 0.85));
            jacket.rect(13, 14 + y0, 14, 21, shade(cloth, 0.8));
            jacket.rect(6, 14 + y0, 6, 20, shade(cloth, 1.08));
            gear.rect(5, 21, 14, 21, STRAP);
        }
        Facing::Side => {
            jacket.rect(6, 13 + y0, 12, 21, cloth);
            jacket.rect(11, 13 + y0, 12, 21, shade(cloth, 0.8));
            jacket.clear(6, 13 + y0);
            // Shoulder seam; the arm itself reaches forward to the weapon.
            jacket.rect(9, 14 + y0, 11, 15 + y0, shade(cloth, 0.86));
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

/// Forearm sleeve, pointing +X, cuff at the right where the fist takes over.
fn paint_arm() -> Canvas {
    let mut c = Canvas::new(7, 5);
    let cloth = rgb(0.93, 0.93, 0.93);
    c.rect(1, 1, 5, 3, cloth);
    c.rect(1, 1, 4, 1, shade(cloth, 1.08));
    c.rect(1, 3, 5, 3, shade(cloth, 0.72));
    c.rect(5, 1, 5, 3, shade(cloth, 0.8));
    c.outline(OUTLINE);
    c
}

/// A fist wrapped around something two pixels thick, seen side-on: 3x4 pixels from (x, y).
/// It gets its own dark edge where it overlaps the weapon so it doesn't melt into it.
fn paint_fist(c: &mut Canvas, x: i32, y: i32) {
    for (ex, ey) in (x - 1..=x + 3)
        .flat_map(|ex| [(ex, y - 1), (ex, y + 4)])
        .chain((y..=y + 3).flat_map(|ey| [(x - 1, ey), (x + 3, ey)]))
    {
        if c.get(ex, ey)[3] > 0.5 {
            c.put(ex, ey, OUTLINE);
        }
    }
    c.rect(x, y, x + 2, y + 3, SKIN);
    c.rect(x, y, x + 1, y, shade(SKIN, 1.08));
    c.rect(x, y + 3, x + 2, y + 3, SKIN_SHADE);
    c.rect(x, y + 1, x, y + 3, SKIN_SHADE);
    // Knuckle creases between the curled fingers.
    c.put(x + 2, y + 1, shade(SKIN_SHADE, 0.88));
    c.put(x + 2, y + 2, shade(SKIN_SHADE, 0.88));
    c.put(x + 1, y + 2, SKIN_SHADE);
}

/// Paint a weapon facing +X. Returns the loot icon, the held version with fists on it, and the
/// rear grip in texture pixels.
fn paint_weapon(kind: WeaponKind) -> (Canvas, Canvas, Vec2) {
    let (base, fists): (Canvas, &[(i32, i32)]) = match kind {
        WeaponKind::Stick => {
            // Splintered club, tape-wrapped handle, a couple of bent nails.
            let mut c = Canvas::new(20, 9);
            let wood = rgb(0.6, 0.42, 0.22);
            c.rect(1, 3, 9, 4, wood);
            c.rect(1, 3, 9, 3, shade(wood, 1.22));
            c.rect(1, 4, 9, 4, shade(wood, 0.72));
            c.rect(1, 2, 1, 5, shade(wood, 0.65));
            let tape = rgb(0.72, 0.68, 0.58);
            for x in [2, 4, 6] {
                c.rect(x, 3, x, 4, tape);
                c.put(x, 4, shade(tape, 0.75));
            }
            for x in 9..=18 {
                let t = (x - 9) as f32 / 9.0;
                let half = 1.0 + t * 1.3;
                let (y0, y1) = ((3.5 - half).round() as i32, (3.5 + half).round() as i32 - 1);
                c.rect(x, y0, x, y1, wood);
                c.put(x, y0, shade(wood, 1.25));
                c.put(x, y1, shade(wood, 0.7));
            }
            c.rect(18, 2, 18, 5, shade(wood, 0.85));
            for (x, y) in [(11, 3), (14, 4), (16, 2)] {
                c.put(x, y, shade(wood, 0.62));
            }
            let nail = rgb(0.72, 0.72, 0.74);
            for (x, y, dx) in [(12, 1, 1), (15, 0, -1), (14, 6, 1)] {
                c.put(x, y, nail);
                c.put(x + dx, y + if y > 3 { 1 } else { -1 }, shade(nail, 0.7));
            }
            (c, &[(3, 2)])
        }
        WeaponKind::Bow => {
            // Recurve bow with a wrapped grip and an arrow on the string.
            let mut c = Canvas::new(15, 21);
            let wood = rgb(0.64, 0.44, 0.22);
            for y in 1..=19 {
                let t = (y as f32 - 10.0) / 9.0;
                let x = (3.0 + 4.5 * (1.0 - t * t)).round() as i32;
                c.put(x, y, wood);
                if t.abs() < 0.6 {
                    c.put(x - 1, y, shade(wood, 1.25));
                } else if t.abs() > 0.88 {
                    c.put(x + 1, y, shade(wood, 0.8));
                }
            }
            c.rect(6, 8, 7, 12, rgb(0.36, 0.22, 0.13));
            c.put(6, 9, rgb(0.5, 0.33, 0.2));
            c.put(6, 11, rgb(0.5, 0.33, 0.2));
            c.line(3.0, 1.5, 3.0, 19.5, rgb(0.9, 0.88, 0.78));
            let shaft = rgb(0.78, 0.66, 0.45);
            c.rect(2, 10, 11, 10, shaft);
            let head = rgb(0.62, 0.64, 0.66);
            c.rect(12, 10, 13, 10, head);
            c.put(12, 9, shade(head, 1.2));
            c.put(12, 11, shade(head, 0.7));
            let feather = rgb(0.82, 0.25, 0.2);
            c.rect(1, 9, 2, 9, feather);
            c.rect(1, 11, 2, 11, shade(feather, 0.75));
            (c, &[(5, 8)])
        }
        WeaponKind::PipeShotgun => {
            // Scrap pipe on a wooden stock, held together with tape and a hose clamp.
            let mut c = Canvas::new(23, 9);
            let wood = rgb(0.5, 0.33, 0.18);
            c.rect(1, 3, 2, 7, wood);
            c.rect(3, 3, 7, 5, wood);
            c.rect(1, 3, 7, 3, shade(wood, 1.25));
            c.rect(1, 7, 2, 7, shade(wood, 0.65));
            c.rect(3, 5, 7, 5, shade(wood, 0.72));
            c.put(1, 5, shade(wood, 0.6));
            let iron = rgb(0.42, 0.4, 0.38);
            c.rect(8, 2, 11, 5, iron);
            c.rect(8, 2, 11, 2, shade(iron, 1.35));
            c.rect(8, 5, 11, 5, shade(iron, 0.7));
            c.put(10, 3, rgb(0.55, 0.3, 0.16));
            c.rect(9, 6, 10, 6, shade(iron, 0.7));
            c.put(8, 6, shade(iron, 0.7));
            let pipe = rgb(0.55, 0.55, 0.52);
            c.rect(11, 2, 20, 4, pipe);
            c.rect(11, 2, 20, 2, shade(pipe, 1.35));
            c.rect(11, 4, 20, 4, shade(pipe, 0.66));
            for (x, y) in [(13, 3), (18, 4), (16, 2)] {
                c.put(x, y, rgb(0.6, 0.36, 0.2));
            }
            c.rect(20, 1, 21, 5, shade(pipe, 0.8));
            c.rect(20, 1, 21, 1, shade(pipe, 1.2));
            c.put(21, 3, OUTLINE);
            let tape = rgb(0.68, 0.7, 0.66);
            c.rect(17, 2, 17, 4, tape);
            c.put(17, 4, shade(tape, 0.72));
            (c, &[(5, 3), (14, 2)])
        }
        WeaponKind::Rifle => {
            // Scoped rifle: stock, pistol grip, curved mag, vented handguard, muzzle brake.
            let mut c = Canvas::new(27, 11);
            let poly = rgb(0.32, 0.27, 0.2);
            c.rect(1, 3, 1, 7, shade(poly, 0.6));
            c.rect(2, 3, 4, 6, poly);
            c.rect(5, 4, 7, 5, poly);
            c.rect(2, 3, 7, 3, shade(poly, 1.3));
            c.rect(2, 6, 4, 6, shade(poly, 0.72));
            let metal = rgb(0.25, 0.27, 0.26);
            c.rect(7, 3, 15, 5, metal);
            c.rect(7, 3, 15, 3, shade(metal, 1.6));
            c.rect(7, 5, 15, 5, shade(metal, 0.7));
            c.rect(12, 4, 13, 4, shade(metal, 1.9));
            c.rect(7, 6, 8, 8, shade(metal, 0.8));
            c.put(9, 6, shade(metal, 0.6));
            c.rect(11, 6, 12, 7, shade(metal, 0.85));
            c.rect(12, 8, 13, 8, shade(metal, 0.85));
            c.put(11, 6, shade(metal, 1.3));
            let scope = rgb(0.14, 0.15, 0.15);
            c.rect(8, 1, 13, 2, scope);
            c.rect(8, 1, 12, 1, shade(scope, 2.0));
            c.put(13, 1, rgb(0.55, 0.95, 1.0));
            c.put(13, 2, rgb(0.2, 0.55, 0.65));
            let guard = rgb(0.36, 0.33, 0.26);
            c.rect(16, 3, 21, 5, guard);
            c.rect(16, 3, 21, 3, shade(guard, 1.3));
            c.rect(16, 5, 21, 5, shade(guard, 0.7));
            for x in [17, 19, 21] {
                c.put(x, 4, shade(guard, 0.55));
            }
            c.rect(22, 4, 24, 4, metal);
            c.put(23, 3, shade(metal, 1.4));
            c.rect(25, 3, 25, 5, shade(metal, 1.2));
            (c, &[(6, 4), (17, 3)])
        }
        WeaponKind::Laser => {
            // Pre-war energy carbine: white shell, glowing cell window, finned emitter.
            let mut c = Canvas::new(23, 10);
            let shell = rgb(0.86, 0.88, 0.9);
            c.rect(2, 2, 15, 5, shell);
            c.rect(1, 3, 1, 5, shade(shell, 0.85));
            c.rect(2, 2, 15, 2, shade(shell, 1.12));
            c.rect(2, 5, 15, 5, shade(shell, 0.68));
            c.clear(2, 2);
            c.rect(3, 3, 3, 4, rgb(0.95, 0.55, 0.15));
            let cell = rgb(0.15, 0.85, 0.9);
            c.rect(6, 3, 13, 4, shade(cell, 0.6));
            for x in [7, 9, 11, 13] {
                c.put(x, 3, rgb(0.75, 1.0, 1.0));
                c.put(x - 1, 4, cell);
            }
            c.rect(4, 6, 6, 7, rgb(0.3, 0.32, 0.36));
            let fin = rgb(0.2, 0.42, 0.48);
            for x in [16, 18] {
                c.rect(x, 1, x, 6, fin);
                c.put(x, 1, shade(fin, 1.5));
            }
            c.rect(17, 3, 19, 4, shade(fin, 0.75));
            c.rect(20, 3, 21, 4, rgb(0.55, 1.0, 1.0));
            c.put(21, 3, rgb(0.95, 1.0, 1.0));
            (c, &[(4, 4), (11, 4)])
        }
        WeaponKind::Railgun => {
            // Twin rails with glowing coils and a capacitor pack.
            let mut c = Canvas::new(31, 12);
            let body = rgb(0.27, 0.24, 0.32);
            c.rect(1, 4, 6, 7, body);
            c.rect(1, 4, 6, 4, shade(body, 1.35));
            c.rect(1, 7, 6, 7, shade(body, 0.7));
            c.rect(6, 2, 12, 8, shade(body, 1.1));
            c.rect(6, 2, 12, 2, shade(body, 1.6));
            c.rect(6, 8, 12, 8, shade(body, 0.75));
            c.rect(8, 4, 10, 5, rgb(0.75, 0.4, 1.0));
            c.put(8, 4, rgb(1.0, 0.85, 1.0));
            let rail = rgb(0.5, 0.48, 0.56);
            c.rect(12, 2, 28, 2, rail);
            c.rect(12, 8, 28, 8, shade(rail, 0.75));
            c.rect(12, 3, 26, 7, rgb(0.14, 0.08, 0.2));
            c.rect(12, 5, 26, 5, rgb(0.55, 0.25, 0.8));
            for x in [14, 18, 22] {
                c.rect(x, 1, x + 1, 9, rgb(0.62, 0.32, 0.9));
                c.rect(x, 1, x + 1, 1, rgb(1.0, 0.82, 1.0));
                c.rect(x + 1, 2, x + 1, 9, rgb(0.42, 0.2, 0.62));
            }
            c.rect(27, 4, 29, 6, rgb(0.85, 0.6, 1.0));
            c.put(29, 5, rgb(1.0, 0.95, 1.0));
            (c, &[(6, 5), (10, 6)])
        }
    };
    let mut icon = base.clone();
    icon.outline(OUTLINE);
    let mut held = base;
    for &(x, y) in fists {
        paint_fist(&mut held, x, y);
    }
    held.outline(OUTLINE);
    let (x, y) = fists[0];
    (icon, held, Vec2::new(x as f32 + 1.5, y as f32 + 2.0))
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

// ---------------------------------------------------------------- city

/// Short random walk of dark pixels, for cracks in tiles.
fn crack(c: &mut Canvas, seed: u32, count: i32, len: i32, color: Rgba) {
    for i in 0..count {
        let (mut x, mut y) = (
            (hash(i, 0, seed) * c.w as f32) as i32,
            (hash(i, 1, seed) * c.h as f32) as i32,
        );
        for step in 0..len {
            c.put_wrap(x, y, color);
            let r = hash(i, step, seed + 1);
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
}

/// Cast concrete slabs with grimy seams and weeds pushing through.
fn paint_pavement() -> Canvas {
    let slab = rgb(0.44, 0.42, 0.39);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let n = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 80);
        let tone = 0.86
            + n * 0.18
            + (hash(x / 16, y / 16, 81) - 0.5) * 0.12
            + (hash(x, y, 82) - 0.5) * 0.06;
        let mut col = shade(slab, tone);
        if x % 16 == 0 || y % 16 == 0 {
            col = shade(slab, 0.62);
        } else if x % 16 == 1 || y % 16 == 1 {
            col = shade(col, 1.08);
        }
        let dirt = (noise(x as f32 / 5.33, y as f32 / 5.33, 6, 83) - 0.6).max(0.0) * 1.8;
        mix(col, rgb(0.36, 0.25, 0.18), dirt)
    });
    crack(&mut c, 84, 2, 10, [0.2, 0.19, 0.18, 0.7]);
    for (x, y) in [(0, 5), (16, 22), (9, 16), (25, 0)] {
        c.put(x, y, rgb(0.42, 0.55, 0.25));
        c.put(x + 1, y - 1, rgb(0.5, 0.62, 0.3));
    }
    c
}

/// Cracked road surface with tar patches.
fn paint_asphalt() -> Canvas {
    let tar = rgb(0.2, 0.19, 0.2);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let patch = (noise(x as f32 / 8.0, y as f32 / 8.0, 4, 90) - 0.55).max(0.0) * 1.2;
        let grain = (hash(x, y, 91) - 0.5) * 0.18;
        let col = shade(tar, 1.0 + grain + patch * 0.5);
        if hash(x, y, 92) > 0.94 {
            shade(col, 1.6)
        } else {
            col
        }
    });
    crack(&mut c, 93, 2, 12, [0.08, 0.08, 0.08, 0.8]);
    c
}

/// Factory-yard concrete soaked with oil.
fn paint_oily_concrete() -> Canvas {
    let slab = rgb(0.38, 0.37, 0.34);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let n = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 100);
        let mut col = shade(slab, 0.88 + n * 0.16 + (hash(x, y, 101) - 0.5) * 0.06);
        if x == 0 || y == 0 {
            col = shade(slab, 0.6);
        }
        let oil = (noise(x as f32 / 6.4, y as f32 / 6.4, 5, 102) - 0.66).max(0.0) * 2.0;
        mix(col, rgb(0.12, 0.12, 0.14), oil.min(0.5))
    });
    // Flaking yellow safety paint.
    for x in 4..12 {
        if hash(x, 0, 103) > 0.25 {
            c.put(x, 24, rgb(0.75, 0.62, 0.2));
        }
    }
    c
}

/// Sickly soil around the craters: yellow-green, with glowing specks.
fn paint_rad_ground() -> Canvas {
    let base = rgb(0.3, 0.32, 0.15);
    let mut c = Canvas::filled(64, 64, |x, y| {
        let (fx, fy) = (x as f32, y as f32);
        let broad = noise(fx / 16.0, fy / 16.0, 4, 110);
        let mid = noise(fx / 6.4, fy / 6.4, 10, 111);
        let k = 0.75 + broad * 0.25 + mid * 0.15 + (hash(x, y, 112) - 0.5) * 0.08;
        let scorch = (noise(fx / 10.67, fy / 10.67, 6, 113) - 0.55).max(0.0) * 1.6;
        mix(shade(base, k), rgb(0.16, 0.14, 0.1), scorch)
    });
    for i in 0..14 {
        let x = (hash(i, 0, 114) * 64.0) as i32;
        let y = (hash(i, 1, 114) * 64.0) as i32;
        c.put_wrap(x, y, rgb(0.6, 1.0, 0.3));
        c.put_wrap(x + 1, y, [0.6, 1.0, 0.3, 0.4]);
    }
    crack(&mut c, 115, 3, 16, [0.12, 0.12, 0.06, 0.6]);
    c
}

fn paint_planks() -> Canvas {
    let wood = rgb(0.46, 0.31, 0.19);
    Canvas::filled(32, 32, |x, y| {
        let board = y / 8;
        let joint = (hash(board, 0, 120) * 32.0) as i32;
        let tone = 0.82 + hash(board, 1, 120) * 0.3;
        let grain = noise(x as f32 / 8.0, y as f32 * 1.0, 4, 121) * 0.16;
        let mut col = shade(wood, tone + grain);
        if y % 8 == 0 {
            col = shade(wood, 0.45);
        } else if y % 8 == 1 {
            col = shade(col, 1.12);
        }
        if x == joint {
            col = shade(wood, 0.5);
        }
        if (x == (joint + 2) % 32 || x == (joint + 30) % 32) && y % 8 == 4 {
            col = rgb(0.25, 0.22, 0.2);
        }
        col
    })
}

fn paint_tiles() -> Canvas {
    let a = rgb(0.7, 0.7, 0.64);
    let b = rgb(0.52, 0.6, 0.58);
    let mut c = Canvas::filled(32, 32, |x, y| {
        if x % 8 == 0 || y % 8 == 0 {
            return rgb(0.28, 0.27, 0.25);
        }
        let (tx, ty) = (x / 8, y / 8);
        let col = if (tx + ty) % 2 == 0 { a } else { b };
        let grime = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 130) * 0.2;
        let mut col = shade(col, 0.82 + grime + (hash(tx, ty, 131) - 0.5) * 0.1);
        if x % 8 == 1 || y % 8 == 1 {
            col = shade(col, 1.1);
        }
        col
    });
    crack(&mut c, 132, 1, 8, [0.2, 0.2, 0.18, 0.8]);
    c
}

fn paint_floor_concrete() -> Canvas {
    let slab = rgb(0.4, 0.4, 0.38);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let n = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 140);
        let col = shade(slab, 0.86 + n * 0.2 + (hash(x, y, 141) - 0.5) * 0.04);
        if x == 0 || y == 0 {
            shade(slab, 0.7)
        } else {
            col
        }
    });
    crack(&mut c, 142, 1, 10, [0.22, 0.22, 0.2, 0.7]);
    c
}

fn paint_roof(kind: Roof) -> Canvas {
    match kind {
        // Corrugated sheet metal, rusting through.
        Roof::Tin => Canvas::filled(32, 32, |x, y| {
            let base = rgb(0.5, 0.5, 0.52);
            let ridge = [0.7, 0.95, 1.15, 0.95][(x % 4) as usize];
            let mut col = shade(base, ridge + (hash(x, y, 150) - 0.5) * 0.05);
            let rust = (noise(x as f32 / 6.4, y as f32 / 6.4, 5, 151) - 0.58).max(0.0) * 1.6;
            col = mix(col, rgb(0.5, 0.27, 0.13), rust.min(0.6));
            if y % 16 == 0 {
                col = shade(col, 0.55);
            } else if y % 16 == 1 {
                col = shade(col, 1.15);
            }
            col
        }),
        // Tar-paper shingles, staggered.
        Roof::Tar => Canvas::filled(32, 32, |x, y| {
            let row = y / 6;
            let lx = (x + if row % 2 == 0 { 0 } else { 8 }).rem_euclid(32);
            let shingle = lx / 16;
            let base = rgb(0.27, 0.22, 0.22);
            let mut col = shade(base, 0.85 + hash(shingle, row, 152) * 0.35);
            if y % 6 == 5 || lx % 16 == 0 {
                col = shade(base, 0.5);
            }
            let moss = (noise(x as f32 / 5.33, y as f32 / 5.33, 6, 153) - 0.7).max(0.0) * 1.6;
            mix(col, rgb(0.3, 0.36, 0.2), moss)
        }),
        // Flat poured roof with gravel and puddle stains.
        Roof::Concrete => Canvas::filled(32, 32, |x, y| {
            let base = rgb(0.56, 0.54, 0.5);
            let n = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 154);
            let mut col = shade(base, 0.85 + n * 0.2);
            if hash(x, y, 155) > 0.9 {
                col = shade(col, 0.75);
            }
            let stain = (noise(x as f32 / 10.67, y as f32 / 10.67, 3, 156) - 0.62).max(0.0) * 2.0;
            col = mix(col, rgb(0.3, 0.3, 0.3), stain);
            if x == 0 || y == 0 {
                col = shade(base, 0.7);
            }
            col
        }),
    }
}

/// Corrugated steel (containers, tanks, wrecks). Neutral gray: the renderer tints it.
fn paint_metal() -> Canvas {
    let base = rgb(0.68, 0.68, 0.68);
    Canvas::filled(32, 32, |x, y| {
        let ridge = [0.72, 0.95, 1.12, 0.95][(x % 4) as usize];
        let mut col = shade(base, ridge + (hash(x, y, 160) - 0.5) * 0.05);
        // Rust streaks running down from the top rail.
        let streak = hash(x, 0, 161);
        if streak > 0.8 && y as f32 > 4.0 + streak * 10.0 {
            col = mix(col, rgb(0.45, 0.24, 0.12), 0.5);
        }
        if !(3..=28).contains(&y) {
            col = shade(base, if y == 0 || y == 31 { 0.45 } else { 0.8 });
        }
        col
    })
}

fn paint_metal_top() -> Canvas {
    let base = rgb(0.62, 0.62, 0.62);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let n = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 162);
        let mut col = shade(base, 0.9 + n * 0.15);
        let rust = (noise(x as f32 / 5.33, y as f32 / 5.33, 6, 163) - 0.7).max(0.0) * 1.8;
        col = mix(col, rgb(0.45, 0.24, 0.12), rust);
        if x % 16 == 0 || y % 16 == 0 {
            col = shade(base, 0.6);
        }
        col
    });
    for (x, y) in [(3, 3), (13, 3), (3, 13), (13, 13), (19, 19), (29, 29)] {
        c.put(x, y, rgb(0.85, 0.85, 0.82));
    }
    c
}

fn paint_rock(top: bool) -> Canvas {
    let base = if top {
        rgb(0.58, 0.5, 0.43)
    } else {
        rgb(0.45, 0.38, 0.33)
    };
    let seed = if top { 170 } else { 171 };
    Canvas::filled(32, 32, |x, y| {
        let n = noise(x as f32 / 8.0, y as f32 / 8.0, 4, seed);
        let fine = noise(x as f32 / 3.2, y as f32 / 3.2, 10, seed + 2);
        let mut col = shade(base, 0.78 + n * 0.3 + fine * 0.12);
        // Pits and chips.
        if fine < 0.2 {
            col = shade(col, 0.72);
        } else if hash(x, y, seed + 3) > 0.96 {
            col = shade(col, 1.2);
        }
        if top {
            let lichen = (noise(x as f32 / 5.33, y as f32 / 5.33, 6, 172) - 0.64).max(0.0) * 2.4;
            col = mix(col, rgb(0.62, 0.66, 0.4), lichen);
        }
        col
    })
}

/// Khaki sandbags stacked in staggered rows (front face) or seen from above (cap).
fn paint_sandbags(top: bool) -> Canvas {
    let bag = rgb(0.62, 0.55, 0.38);
    let row_h = if top { 16 } else { 8 };
    Canvas::filled(32, 32, |x, y| {
        let row = y / row_h;
        let lx = (x + if row % 2 == 0 { 0 } else { 8 }).rem_euclid(32) % 16;
        let ly = y % row_h;
        let dx = (lx as f32 + 0.5 - 8.0) / 8.0;
        let dy = (ly as f32 + 0.5 - row_h as f32 / 2.0) / (row_h as f32 / 2.0);
        let d = dx * dx + dy * dy;
        if d > 1.05 {
            return rgb(0.2, 0.17, 0.12);
        }
        let light = 1.15 - 0.35 * (ly as f32 / row_h as f32) - 0.15 * dx.abs();
        let mut col = shade(bag, light + (hash(x, y, 180) - 0.5) * 0.08);
        if lx == 8 && ly % 2 == 0 {
            col = shade(bag, 0.7);
        }
        col
    })
}

/// A wooden supply crate in three-quarter view: lid on top, braced front.
fn paint_crate() -> Canvas {
    let mut c = Canvas::new(20, 22);
    let wood = rgb(0.6, 0.42, 0.24);
    // Lid.
    c.rect(1, 1, 18, 6, shade(wood, 1.2));
    for x in [6, 12] {
        c.rect(x, 1, x, 6, shade(wood, 0.8));
    }
    // Front planks.
    c.rect(1, 7, 18, 20, wood);
    for y in [11, 16] {
        c.rect(1, y, 18, y, shade(wood, 0.65));
    }
    // Frame and cross brace.
    let frame = shade(wood, 0.55);
    c.rect(1, 7, 18, 7, frame);
    c.rect(1, 20, 18, 20, frame);
    c.rect(1, 7, 2, 20, frame);
    c.rect(17, 7, 18, 20, frame);
    c.line(3.0, 19.0, 16.0, 8.0, shade(wood, 0.75));
    c.line(3.0, 8.0, 16.0, 19.0, shade(wood, 0.75));
    // Stenciled supply mark.
    c.rect(8, 12, 11, 15, rgb(0.85, 0.75, 0.35));
    c.rect(9, 13, 10, 14, wood);
    c.outline(OUTLINE);
    c
}

/// A rusty red fuel drum with a hazard band.
fn paint_barrel() -> Canvas {
    let mut c = Canvas::new(16, 22);
    let red = rgb(0.68, 0.17, 0.12);
    for y in 3..21 {
        for x in 1..15 {
            let fx = (x as f32 + 0.5 - 6.0) / 7.0;
            let light = 1.2 - fx * fx * 0.6 - fx * 0.15;
            let rust = (noise(x as f32 / 3.0, y as f32 / 3.0, 8, 190) - 0.6).max(0.0) * 2.5;
            c.put(
                x,
                y,
                mix(shade(red, light), rgb(0.4, 0.22, 0.12), rust.min(0.8)),
            );
        }
    }
    for y in [7, 16] {
        for x in 1..15 {
            let p = c.get(x, y);
            c.put(x, y, shade(p, 0.6));
        }
    }
    // Hazard band.
    for x in 1..15 {
        let stripe = (x / 2) % 2 == 0;
        let col = if stripe {
            rgb(0.95, 0.78, 0.15)
        } else {
            rgb(0.12, 0.1, 0.08)
        };
        c.rect(x, 11, x, 12, col);
    }
    // Lid.
    c.ellipse(8.0, 3.5, 7.0, 2.5, shade(red, 0.75));
    c.ellipse(8.0, 3.3, 5.5, 1.6, shade(red, 1.1));
    c.put(11, 3, rgb(0.3, 0.3, 0.3));
    c.outline(OUTLINE);
    c
}

/// A squat steel safe; `open` shows the door swung wide and the shelf bare.
fn paint_safe(open: bool) -> Canvas {
    let mut c = Canvas::new(22, 24);
    let steel = rgb(0.3, 0.35, 0.33);
    c.rect(1, 1, 20, 5, shade(steel, 1.3));
    c.rect(1, 6, 20, 21, steel);
    c.rect(2, 22, 4, 23, shade(steel, 0.6));
    c.rect(17, 22, 19, 23, shade(steel, 0.6));
    if open {
        c.rect(4, 8, 17, 19, rgb(0.06, 0.06, 0.07));
        c.rect(4, 14, 17, 14, rgb(0.25, 0.25, 0.25));
        // The door, swung toward the viewer on its left hinge.
        c.rect(0, 7, 3, 21, shade(steel, 1.15));
        c.rect(1, 12, 2, 14, rgb(0.75, 0.7, 0.5));
    } else {
        c.rect(3, 8, 18, 19, shade(steel, 0.85));
        c.rect(4, 9, 17, 18, shade(steel, 1.05));
        c.disc(13.5, 13.5, 3.0, rgb(0.7, 0.68, 0.6));
        c.disc(13.5, 13.5, 1.6, rgb(0.35, 0.34, 0.3));
        c.put(13, 10, rgb(0.9, 0.2, 0.15));
        c.rect(6, 12, 7, 15, rgb(0.75, 0.7, 0.5));
        c.rect(3, 9, 3, 10, rgb(0.15, 0.15, 0.15));
        c.rect(3, 17, 3, 18, rgb(0.15, 0.15, 0.15));
    }
    c.outline(OUTLINE);
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

    /// 24-bit BMP, bottom-up rows, each sheet pixel blown up `zoom` times.
    fn write_bmp(sheet: &Canvas, zoom: i32, path: &str) {
        let (w, h) = (sheet.w as i32, sheet.h as i32);
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
        write_bmp(&sheet, zoom, &path);
    }

    /// Dev aid: `EVOCLASH_ARMED_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_armed`
    /// poses a survivor holding every weapon at several aim angles, the way the renderer does.
    #[test]
    #[ignore]
    fn preview_armed() {
        let Ok(path) = std::env::var("EVOCLASH_ARMED_PREVIEW") else {
            return;
        };
        // Mirrors render.rs: one sheet pixel is one world pixel.
        const GUN_HEIGHT: f32 = 26.0;
        const FOOT_INSET: f32 = 2.0;
        let tint = [0.35, 0.6, 0.95];
        let angles = [0.0f32, 35.0, 80.0, 145.0, 180.0, -45.0, -90.0, -140.0];
        let kinds = [
            WeaponKind::Stick,
            WeaponKind::Bow,
            WeaponKind::PipeShotgun,
            WeaponKind::Rifle,
            WeaponKind::Laser,
            WeaponKind::Railgun,
        ];
        let (cw, ch) = (96, 96);
        let mut sheet =
            Canvas::filled(cw * angles.len() as u32, ch * kinds.len() as u32, |x, y| {
                if (x / 8 + y / 8) % 2 == 0 {
                    rgb(0.78, 0.62, 0.36)
                } else {
                    rgb(0.74, 0.58, 0.33)
                }
            });
        let arm = paint_arm();
        // Draw `src` scaled by PIXEL, centered at world `at` (y up), rotated by `angle`.
        let draw = |sheet: &mut Canvas,
                    src: &Canvas,
                    at: Vec2,
                    angle: f32,
                    flip: (bool, bool),
                    tint: [f32; 3]| {
            let (sin, cos) = angle.sin_cos();
            let half = Vec2::new(src.w as f32, src.h as f32) * PIXEL / 2.0;
            let r = half.length().ceil() as i32 + 1;
            for dy in -r..=r {
                for dx in -r..=r {
                    let d = Vec2::new(dx as f32 + 0.5, dy as f32 + 0.5);
                    // World offset is (d.x, -d.y); undo rotation.
                    let w = Vec2::new(d.x, -d.y);
                    let l = Vec2::new(w.x * cos + w.y * sin, -w.x * sin + w.y * cos);
                    let mut u = (l.x + half.x) / PIXEL;
                    let mut v = (half.y - l.y) / PIXEL;
                    if flip.0 {
                        u = src.w as f32 - u;
                    }
                    if flip.1 {
                        v = src.h as f32 - v;
                    }
                    if u < 0.0 || v < 0.0 {
                        continue;
                    }
                    let p = src.get(u as i32, v as i32);
                    if p[3] > 0.0 {
                        sheet.put(
                            (at.x + dx as f32) as i32,
                            (-at.y + dy as f32) as i32,
                            [p[0] * tint[0], p[1] * tint[1], p[2] * tint[2], p[3]],
                        );
                    }
                }
            }
        };
        for (row, kind) in kinds.iter().enumerate() {
            let (_, weapon, grip) = paint_weapon(*kind);
            let size = Vec2::new(weapon.w as f32, weapon.h as f32);
            for (col, deg) in angles.iter().enumerate() {
                let aim = Vec2::from_angle(deg.to_radians());
                let feet = Vec2::new(
                    col as f32 * cw as f32 + 48.0,
                    -(row as f32 * ch as f32 + 84.0),
                );
                let (facing, flip) = Facing::from_aim(aim);
                let (jacket, gear) = paint_survivor(facing, 0);
                let body = feet + Vec2::Y * (SURVIVOR.y as f32 * PIXEL / 2.0 - FOOT_INSET);
                let pivot = feet
                    + Vec2::new(
                        if facing == Facing::Side {
                            if flip { -4.0 } else { 4.0 }
                        } else {
                            0.0
                        },
                        GUN_HEIGHT,
                    );
                let (center, arm_at) = held_layout(pivot, aim, size, grip);
                let behind = facing == Facing::Up;
                let gun = |sheet: &mut Canvas| {
                    draw(
                        sheet,
                        &arm,
                        arm_at,
                        aim.to_angle(),
                        (false, aim.x < 0.0),
                        tint,
                    );
                    draw(
                        sheet,
                        &weapon,
                        center,
                        aim.to_angle(),
                        (false, aim.x < 0.0),
                        [1.0; 3],
                    );
                };
                if behind {
                    gun(&mut sheet);
                }
                draw(&mut sheet, &jacket, body, 0.0, (flip, false), tint);
                draw(&mut sheet, &gear, body, 0.0, (flip, false), [1.0; 3]);
                if !behind {
                    gun(&mut sheet);
                }
            }
        }
        write_bmp(&sheet, 4, &path);
    }

    /// Dev aid: `EVOCLASH_CITY_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_city`
    /// lays out the city tiles (2x2 repeats, to check seams) and the props.
    #[test]
    #[ignore]
    fn preview_city() {
        let Ok(path) = std::env::var("EVOCLASH_CITY_PREVIEW") else {
            return;
        };
        let tiles = [
            paint_pavement(),
            paint_asphalt(),
            paint_oily_concrete(),
            paint_rad_ground(),
            paint_planks(),
            paint_tiles(),
            paint_floor_concrete(),
            paint_roof(Roof::Tin),
            paint_roof(Roof::Tar),
            paint_roof(Roof::Concrete),
            paint_metal(),
            paint_metal_top(),
            paint_rock(false),
            paint_rock(true),
            paint_sandbags(false),
            paint_sandbags(true),
        ];
        let props = [
            paint_crate(),
            paint_barrel(),
            paint_safe(false),
            paint_safe(true),
        ];
        let cell = 132;
        let mut sheet = Canvas::filled(cell * 4, cell * 4 + 30, |_, _| rgb(0.36, 0.22, 0.16));
        for (i, tile) in tiles.iter().enumerate() {
            let (ox, oy) = ((i % 4) as i32 * cell as i32, (i / 4) as i32 * cell as i32);
            for y in 0..128 {
                for x in 0..128 {
                    let p = tile.get(x % tile.w as i32, y % tile.h as i32);
                    sheet.put(ox + x, oy + y, p);
                }
            }
        }
        for (i, prop) in props.iter().enumerate() {
            let (ox, oy) = (i as i32 * 30 + 4, cell as i32 * 4 + 4);
            for y in 0..prop.h as i32 {
                for x in 0..prop.w as i32 {
                    sheet.put(ox + x, oy + y, prop.get(x, y));
                }
            }
        }
        write_bmp(&sheet, 3, &path);
    }

    /// Dev aid: bolt, slash, shock ring and fireball (tinted) over ground color, blown up 3x.
    #[test]
    #[ignore]
    fn preview_effects() {
        let Ok(path) = std::env::var("EVOCLASH_SPRITE_PREVIEW") else {
            return;
        };
        let mut sheet = Canvas::filled(400, 120, |_, _| rgb(0.36, 0.22, 0.16));
        let tints = [
            (paint_bolt(), rgb(1.0, 0.9, 0.4), 0),
            (paint_slash(), rgb(1.0, 0.92, 0.75), 60),
            (paint_shock(), rgb(1.0, 0.6, 0.2), 160),
            (paint_blast(), rgb(1.0, 0.5, 0.12), 290),
        ];
        for (c, tint, ox) in tints {
            for y in 0..c.h as i32 {
                for x in 0..c.w as i32 {
                    let p = c.get(x, y);
                    let (px, py) = (ox + x, 10 + y);
                    if px >= 400 || py >= 120 {
                        continue;
                    }
                    let under = sheet.get(px, py);
                    let src = [p[0] * tint[0], p[1] * tint[1], p[2] * tint[2], 1.0];
                    sheet.put(px, py, mix(under, src, p[3]));
                }
            }
        }
        write_bmp(&sheet, 3, &path);
    }

    #[test]
    fn sprites_have_opaque_pixels() {
        for canvas in [
            paint_survivor(Facing::Side, 1).0,
            paint_roach(0),
            paint_pod(),
            paint_medkit(),
            paint_reticle(),
        ] {
            assert!(canvas.px.iter().any(|p| p[3] > 0.9));
        }
        // Soft effects never reach full opacity but must not be blank.
        for canvas in [paint_bolt(), paint_slash(), paint_shock(), paint_blast()] {
            assert!(canvas.px.iter().any(|p| p[3] > 0.5));
        }
    }
}
