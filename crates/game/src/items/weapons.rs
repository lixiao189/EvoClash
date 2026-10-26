//! Weapons as types: every weapon is a `Weapon`, and either a `MeleeWeapon` that swings at a
//! sector ahead of the wielder or a `RangedWeapon` that fires projectiles. `WeaponKind` is the
//! id that goes over the wire; `arm` turns it into the weapon's behaviour.

use bevy::prelude::*;
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};

use crate::sim::DT;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, Default)]
pub enum WeaponKind {
    #[default]
    Stick,
    Bow,
    PipeShotgun,
    Rifle,
    Laser,
    Railgun,
}

/// Ammunition families. Guns hold no rounds of their own: each draws from its family's stack in
/// the owner's ammo pouch, so a gun with an empty pouch is kept, not thrown away.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum AmmoKind {
    Arrow,
    Shell,
    Bullet,
    Cell,
}

impl AmmoKind {
    pub const ALL: [AmmoKind; 4] = [
        AmmoKind::Arrow,
        AmmoKind::Shell,
        AmmoKind::Bullet,
        AmmoKind::Cell,
    ];

    /// Index into `Loadout::ammo`.
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            AmmoKind::Arrow => "箭矢",
            AmmoKind::Shell => "霰弹",
            AmmoKind::Bullet => "步枪弹",
            AmmoKind::Cell => "能量电池",
        }
    }

    /// Pouch capacity before 弹药回收.
    pub fn base_cap(self) -> u16 {
        match self {
            AmmoKind::Arrow => 42,
            AmmoKind::Shell => 30,
            AmmoKind::Bullet => 135,
            AmmoKind::Cell => 108,
        }
    }

    /// Rounds in a box: one full load of the family's basic gun.
    pub fn box_rounds(self) -> u16 {
        match self {
            AmmoKind::Arrow => 14,
            AmmoKind::Shell => 10,
            AmmoKind::Bullet => 45,
            AmmoKind::Cell => 36,
        }
    }

    /// Tier of the cheapest gun that fires it, for loot colors.
    pub fn tier(self) -> u8 {
        match self {
            AmmoKind::Arrow | AmmoKind::Shell => 1,
            AmmoKind::Bullet => 2,
            AmmoKind::Cell => 3,
        }
    }
}

/// What every weapon has, whether it swings or shoots.
pub trait Weapon {
    fn name(&self) -> &'static str;
    fn tier(&self) -> u8;
    /// Ticks between attacks.
    fn cooldown(&self) -> u16;
    /// Damage per hit (per pellet for a shotgun).
    fn damage(&self) -> f32;
    /// Multiplier applied to damage absorbed by armor.
    fn armor_mult(&self) -> f32 {
        1.0
    }
    fn color(&self) -> Color;
}

/// A close-quarters weapon: each swing hits everything whose body reaches into the sector of
/// radius `reach` and half-angle `half_arc` around the aim. The client draws exactly that sector.
pub trait MeleeWeapon: Weapon {
    /// How far from the wielder's center the blow lands, px.
    fn reach(&self) -> f32;
    /// Half-angle of the struck sector around the aim, radians.
    fn half_arc(&self) -> f32;

    /// Whether a body of `radius` at `offset` from the wielder overlaps the swing along `aim`
    /// (a unit vector), with `bonus_reach` from perks.
    fn strikes(&self, offset: Vec2, radius: f32, aim: Vec2, bonus_reach: f32) -> bool {
        let dist = offset.length();
        if dist > self.reach() + bonus_reach + radius {
            return false;
        }
        if dist <= radius {
            return true;
        }
        // The body pokes into the sector if its edge does, not just its center.
        offset.angle_to(aim).abs() <= self.half_arc() + (radius / dist).asin()
    }
}

/// A weapon that fires projectiles and runs out of ammo.
pub trait RangedWeapon: Weapon {
    /// Projectile speed, px/s.
    fn muzzle_speed(&self) -> f32;
    /// Projectile lifetime in ticks.
    fn lifetime(&self) -> u16;
    /// The pouch stack it draws from.
    fn ammo(&self) -> AmmoKind;
    /// Rounds used per trigger pull.
    fn ammo_cost(&self) -> u16 {
        1
    }
    /// Projectiles per trigger pull.
    fn pellets(&self) -> u8 {
        1
    }
    /// Total fan of a multi-pellet shot, or the random bloom either side of a single shot, radians.
    fn spread(&self) -> f32 {
        0.0
    }
    /// Goes through walls and bodies.
    fn piercing(&self) -> bool {
        false
    }
    /// Boosted by 能量聚焦.
    fn energy(&self) -> bool {
        false
    }

    /// How far a projectile flies before it expires, px.
    fn range(&self) -> f32 {
        self.muzzle_speed() * self.lifetime() as f32 * DT
    }

    /// Flight directions of one trigger pull along `aim`: shotguns fan out evenly, single shots
    /// get a little random bloom.
    fn volley(&self, aim: Vec2, rng: &mut dyn RngCore) -> Vec<Vec2> {
        let base = aim.to_angle();
        let (pellets, spread) = (self.pellets(), self.spread());
        (0..pellets)
            .map(|i| {
                let off = if pellets > 1 {
                    (i as f32 / (pellets - 1) as f32 - 0.5) * spread
                } else if spread > 0.0 {
                    rng.random_range(-spread..spread)
                } else {
                    0.0
                };
                Vec2::from_angle(base + off)
            })
            .collect()
    }
}

/// A weapon by how it attacks.
#[derive(Clone, Copy)]
pub enum Arm {
    Melee(&'static dyn MeleeWeapon),
    Ranged(&'static dyn RangedWeapon),
}

impl WeaponKind {
    pub fn arm(self) -> Arm {
        match self {
            WeaponKind::Stick => Arm::Melee(&Stick),
            WeaponKind::Bow => Arm::Ranged(&Bow),
            WeaponKind::PipeShotgun => Arm::Ranged(&PipeShotgun),
            WeaponKind::Rifle => Arm::Ranged(&Rifle),
            WeaponKind::Laser => Arm::Ranged(&Laser),
            WeaponKind::Railgun => Arm::Ranged(&Railgun),
        }
    }

    pub fn spec(self) -> &'static dyn Weapon {
        match self.arm() {
            Arm::Melee(w) => w,
            Arm::Ranged(w) => w,
        }
    }

    pub fn melee(self) -> Option<&'static dyn MeleeWeapon> {
        match self.arm() {
            Arm::Melee(w) => Some(w),
            Arm::Ranged(_) => None,
        }
    }

    pub fn ranged(self) -> Option<&'static dyn RangedWeapon> {
        match self.arm() {
            Arm::Ranged(w) => Some(w),
            Arm::Melee(_) => None,
        }
    }

    /// The ammo a gun draws from; `None` for a melee weapon, which never runs dry.
    pub fn ammo(self) -> Option<AmmoKind> {
        self.ranged().map(|w| w.ammo())
    }

    /// Rounds one trigger pull uses (0 for melee).
    pub fn ammo_cost(self) -> u16 {
        self.ranged().map_or(0, |w| w.ammo_cost())
    }
}

/// T0: always in the melee slot, so nobody is ever unarmed.
pub struct Stick;

impl Weapon for Stick {
    fn name(&self) -> &'static str {
        "木棍"
    }
    fn tier(&self) -> u8 {
        0
    }
    fn cooldown(&self) -> u16 {
        22
    }
    fn damage(&self) -> f32 {
        22.0
    }
    fn color(&self) -> Color {
        Color::srgb(0.65, 0.5, 0.3)
    }
}

impl MeleeWeapon for Stick {
    fn reach(&self) -> f32 {
        78.0
    }
    fn half_arc(&self) -> f32 {
        1.1
    }
}

pub struct Bow;

impl Weapon for Bow {
    fn name(&self) -> &'static str {
        "猎弓"
    }
    fn tier(&self) -> u8 {
        1
    }
    fn cooldown(&self) -> u16 {
        36
    }
    fn damage(&self) -> f32 {
        34.0
    }
    fn color(&self) -> Color {
        Color::srgb(0.85, 0.75, 0.5)
    }
}

impl RangedWeapon for Bow {
    fn muzzle_speed(&self) -> f32 {
        950.0
    }
    fn lifetime(&self) -> u16 {
        60
    }
    fn ammo(&self) -> AmmoKind {
        AmmoKind::Arrow
    }
}

pub struct PipeShotgun;

impl Weapon for PipeShotgun {
    fn name(&self) -> &'static str {
        "管式霰弹枪"
    }
    fn tier(&self) -> u8 {
        1
    }
    fn cooldown(&self) -> u16 {
        48
    }
    fn damage(&self) -> f32 {
        11.0
    }
    fn color(&self) -> Color {
        Color::srgb(1.0, 0.6, 0.2)
    }
}

impl RangedWeapon for PipeShotgun {
    fn muzzle_speed(&self) -> f32 {
        1150.0
    }
    fn lifetime(&self) -> u16 {
        22
    }
    fn ammo(&self) -> AmmoKind {
        AmmoKind::Shell
    }
    fn pellets(&self) -> u8 {
        7
    }
    fn spread(&self) -> f32 {
        0.38
    }
}

pub struct Rifle;

impl Weapon for Rifle {
    fn name(&self) -> &'static str {
        "制式步枪"
    }
    fn tier(&self) -> u8 {
        2
    }
    fn cooldown(&self) -> u16 {
        7
    }
    fn damage(&self) -> f32 {
        11.0
    }
    fn color(&self) -> Color {
        Color::srgb(1.0, 0.95, 0.4)
    }
}

impl RangedWeapon for Rifle {
    fn muzzle_speed(&self) -> f32 {
        1600.0
    }
    fn lifetime(&self) -> u16 {
        50
    }
    fn ammo(&self) -> AmmoKind {
        AmmoKind::Bullet
    }
    fn spread(&self) -> f32 {
        0.06
    }
}

pub struct Laser;

impl Weapon for Laser {
    fn name(&self) -> &'static str {
        "脉冲激光"
    }
    fn tier(&self) -> u8 {
        3
    }
    fn cooldown(&self) -> u16 {
        9
    }
    fn damage(&self) -> f32 {
        15.0
    }
    fn armor_mult(&self) -> f32 {
        2.0
    }
    fn color(&self) -> Color {
        Color::srgb(0.3, 1.0, 1.0)
    }
}

impl RangedWeapon for Laser {
    fn muzzle_speed(&self) -> f32 {
        2600.0
    }
    fn lifetime(&self) -> u16 {
        34
    }
    fn ammo(&self) -> AmmoKind {
        AmmoKind::Cell
    }
    fn energy(&self) -> bool {
        true
    }
}

pub struct Railgun;

impl Weapon for Railgun {
    fn name(&self) -> &'static str {
        "轨道炮"
    }
    fn tier(&self) -> u8 {
        4
    }
    fn cooldown(&self) -> u16 {
        64
    }
    fn damage(&self) -> f32 {
        95.0
    }
    fn armor_mult(&self) -> f32 {
        1.5
    }
    fn color(&self) -> Color {
        Color::srgb(0.85, 0.4, 1.0)
    }
}

impl RangedWeapon for Railgun {
    fn muzzle_speed(&self) -> f32 {
        5200.0
    }
    fn lifetime(&self) -> u16 {
        24
    }
    fn ammo(&self) -> AmmoKind {
        AmmoKind::Cell
    }
    fn ammo_cost(&self) -> u16 {
        6
    }
    fn piercing(&self) -> bool {
        true
    }
    fn energy(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swing_hits_bodies_that_poke_into_the_sector() {
        let stick = WeaponKind::Stick.melee().unwrap();
        let aim = Vec2::X;
        // Edge exactly at the reach.
        assert!(stick.strikes(Vec2::new(78.0 + 22.0, 0.0), 22.0, aim, 0.0));
        assert!(!stick.strikes(Vec2::new(78.0 + 23.0, 0.0), 22.0, aim, 0.0));
        assert!(stick.strikes(Vec2::new(78.0 + 25.0, 0.0), 22.0, aim, 4.0));
        // Center just outside the arc, but the body overlaps its edge.
        let beside = Vec2::from_angle(stick.half_arc() + 0.2) * 60.0;
        assert!(stick.strikes(beside, 22.0, aim, 0.0));
        assert!(!stick.strikes(-beside, 22.0, aim, 0.0));
        // Overlapping the wielder always counts.
        assert!(stick.strikes(Vec2::new(-10.0, 0.0), 22.0, aim, 0.0));
    }

    #[test]
    fn volleys_fan_and_bloom_within_the_spread() {
        let mut rng = rand::rng();
        for kind in [
            WeaponKind::PipeShotgun,
            WeaponKind::Rifle,
            WeaponKind::Railgun,
        ] {
            let gun = kind.ranged().unwrap();
            let dirs = gun.volley(Vec2::Y, &mut rng);
            assert_eq!(dirs.len(), gun.pellets() as usize);
            for d in dirs {
                assert!(d.angle_to(Vec2::Y).abs() <= gun.spread() + 1e-4);
            }
        }
    }
}
