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

mod arena;
mod canvas;
mod city;
mod effects;
mod loot;
mod monsters;
mod skills;
mod survivor;
#[cfg(test)]
mod tests;
mod weapons;

use arena::*;
use canvas::*;
use city::*;
use effects::*;
use loot::*;
use monsters::*;
use skills::*;
pub use survivor::*;
use weapons::*;

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
    /// Side views facing right, two gait frames.
    pub dog: [Handle<Image>; 2],
    pub ghoul: [Handle<Image>; 2],
    pub spitter: Handle<Image>,
    /// Seen from above, facing +X; rotated to its heading at draw time.
    pub crawler: Handle<Image>,
    pub raider: [Handle<Image>; WALK_FRAMES],
    /// Two rotor phases.
    pub drone: [Handle<Image>; 2],
    pub behemoth: [Handle<Image>; 2],
    /// Acid glob in flight.
    pub acid: Handle<Image>,
    /// Chevron pointing +X, white; tinted for off-screen markers.
    pub arrow: Handle<Image>,
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
    /// Rounded, lightly shaded bar for health gauges; tinted at draw time.
    pub bar: Handle<Image>,
    /// 自动炮塔: tripod stand (bottom-anchored) and the swivelling gun head, barrel toward +X.
    pub turret_base: Handle<Image>,
    pub turret_head: Handle<Image>,
    /// 陷阱: a bear trap lying open on the ground, seen from above.
    pub trap: Handle<Image>,
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
        dog: [0, 1].map(|frame| add(paint_dog(frame), true)),
        ghoul: [0, 1].map(|frame| add(paint_ghoul(frame), true)),
        spitter: add(paint_spitter(), true),
        crawler: add(paint_crawler(), true),
        raider: [0, 1, 2, 3].map(|frame| add(paint_raider(frame), true)),
        drone: [0, 1].map(|frame| add(paint_drone(frame), true)),
        behemoth: [0, 1].map(|frame| add(paint_behemoth(frame), true)),
        acid: add(paint_acid(), true),
        arrow: add(paint_arrow(), true),
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
        bar: add(paint_bar(), true),
        turret_base: add(paint_turret_base(), true),
        turret_head: add(paint_turret_head(), true),
        trap: add(paint_trap(), true),
    }
}
