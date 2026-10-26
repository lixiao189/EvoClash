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
use evoclash_game::items::{AmmoKind, ArmorKind, Item, WeaponKind};
use evoclash_game::map::Floor;

mod arena;
mod badges;
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
use badges::*;
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
/// Frames in a melee swing's slash.
pub const SLASH_FRAMES: usize = 6;

#[derive(Resource)]
pub struct Textures {
    pub vault_floor: Handle<Image>,
    pub trefoil: Handle<Image>,
    pub rad_ground: Handle<Image>,
    floors: [Handle<Image>; 3],
    /// Sandbag barricades (the other wall styles are LPC tiles, see `crate::lpc`).
    pub sandbags: WallSprites,
    /// Flat ground stains.
    pub decals: Vec<Handle<Image>>,
    /// Upright clutter (dead grass, bushes, rocks, bones), depth-sorted like everything else.
    pub decor: Vec<Handle<Image>>,
    pub shadow: Handle<Image>,
    pub glow: Handle<Image>,
    /// Side view, two skitter frames.
    pub roach: [Handle<Image>; 2],
    pub pod: Handle<Image>,
    pub spore: Handle<Image>,
    /// Seen from above, facing +X; rotated to its heading at draw time.
    pub crawler: Handle<Image>,
    /// Two rotor phases.
    pub drone: [Handle<Image>; 2],
    /// Acid glob in flight.
    pub acid: Handle<Image>,
    /// Chevron pointing +X, white; tinted for off-screen markers.
    pub arrow: Handle<Image>,
    pub splat: Handle<Image>,
    weapons: [WeaponSprite; 6],
    armors: [Handle<Image>; 3],
    pub medkit: Handle<Image>,
    /// Ammo boxes per `AmmoKind::index`.
    ammo: [Handle<Image>; 4],
    bag: Handle<Image>,
    pub emp: Handle<Image>,
    /// Bullet streak: white, fading tail on the left, bright round head on the right.
    pub bolt: Handle<Image>,
    /// White muzzle flash: hot spot at the left edge, flame tongue toward +X; tinted per weapon.
    pub muzzle: Handle<Image>,
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
    /// Gold crown over the match host. 11×8 texels; about 22×16 world px at [`PIXEL`].
    pub host_crown: Handle<Image>,
    /// Gray rounded "BOT" tag. 14×7 texels; 28×14 world px at [`PIXEL`].
    pub bot_tag: Handle<Image>,
}

#[derive(Clone)]
pub struct WallSprites {
    pub front: Handle<Image>,
    pub top: Handle<Image>,
}

impl Textures {
    pub fn floor(&self, floor: Floor) -> Handle<Image> {
        self.floors[match floor {
            Floor::Planks => 0,
            Floor::Tiles => 1,
            Floor::Concrete => 2,
        }]
        .clone()
    }

    pub fn weapon(&self, kind: WeaponKind) -> &WeaponSprite {
        &self.weapons[weapon_index(kind)]
    }

    pub fn item(&self, item: Item) -> Handle<Image> {
        match item {
            Item::Weapon(kind) => self.weapon(kind).icon.clone(),
            Item::Armor { kind, .. } => self.armors[armor_index(kind)].clone(),
            Item::Emp => self.emp.clone(),
            Item::Medkit => self.medkit.clone(),
            Item::Ammo { kind, .. } => self.ammo[kind.index()].clone(),
            Item::Bag => self.bag.clone(),
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
    /// Melee only: the swing's frames, `SLASH_FRAMES` of them; empty for a gun. Each is the
    /// struck sector centered on +X, radius = half the texture, with the blade sweeping clockwise.
    pub slash: Vec<Handle<Image>>,
}

/// Where to center a held weapon so its rear grip lands on `grip_at`, drawn at `scale` world
/// pixels per texture pixel. `size` and `grip` come from [`WeaponSprite`]; `flip` says the weapon
/// is drawn flipped vertically (it points left), which mirrors the grip too.
pub fn held_layout(
    grip_at: Vec2,
    aim: Vec2,
    size: Vec2,
    grip: Vec2,
    flip: bool,
    scale: f32,
) -> Vec2 {
    let mut g = Vec2::new(grip.x - size.x / 2.0, size.y / 2.0 - grip.y) * scale;
    if flip {
        g.y = -g.y;
    }
    grip_at - aim.rotate(g)
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
            slash: kind.melee().map_or(Vec::new(), |club| {
                (0..SLASH_FRAMES)
                    .map(|i| {
                        let progress = (i + 1) as f32 / SLASH_FRAMES as f32;
                        add(paint_slash(club.half_arc(), progress), false)
                    })
                    .collect()
            }),
        }
    });
    Textures {
        vault_floor: add(paint_vault_floor(), true),
        trefoil: add(paint_trefoil(), true),
        rad_ground: add(paint_rad_ground(), true),
        floors: [paint_planks(), paint_tiles(), paint_floor_concrete()].map(|c| add(c, true)),
        sandbags: WallSprites {
            front: add(paint_sandbags(false), true),
            top: add(paint_sandbags(true), true),
        },
        decals: (0..6).map(|i| add(paint_decal(i), true)).collect(),
        decor: (0..5).map(|i| add(paint_decor(i), true)).collect(),
        shadow: add(paint_soft_disc(32, [0.0, 0.0, 0.0], 1.6), false),
        glow: add(paint_soft_disc(32, [1.0, 1.0, 1.0], 2.2), false),
        roach: [0, 1].map(|frame| add(paint_roach(frame), true)),
        pod: add(paint_pod(), true),
        spore: add(paint_spore(), false),
        crawler: add(paint_crawler(), true),
        drone: [0, 1].map(|frame| add(paint_drone(frame), true)),
        acid: add(paint_acid(), true),
        arrow: add(paint_arrow(), true),
        splat: add(paint_splat(), true),
        weapons,
        armors: [ArmorKind::Scrap, ArmorKind::Kevlar, ArmorKind::PowerArmor]
            .map(|kind| add(paint_armor(kind), true)),
        medkit: add(paint_medkit(), true),
        ammo: [
            add(paint_ammo_kind(AmmoKind::Arrow), true),
            add(paint_ammo_kind(AmmoKind::Shell), true),
            add(paint_ammo(), true),
            add(paint_ammo_kind(AmmoKind::Cell), true),
        ],
        bag: add(paint_bag(), true),
        emp: add(paint_emp(), true),
        bolt: add(paint_bolt(), false),
        muzzle: add(paint_muzzle(), false),
        shock: add(paint_shock(), false),
        blast: add(paint_blast(), false),
        reticle: add(paint_reticle(), true),
        bar: add(paint_bar(), true),
        turret_base: add(paint_turret_base(), true),
        turret_head: add(paint_turret_head(), true),
        trap: add(paint_trap(), true),
        host_crown: add(paint_host_crown(), true),
        bot_tag: add(paint_bot_tag(), true),
    }
}
