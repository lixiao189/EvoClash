//! Gear of the fallen civilization, grouped by technology tier.
//!
//! T0 primitive → T1 industrial scrap → T2 modern military → T3 information age → T4 pre-collapse apex.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, Default)]
pub enum ArmorKind {
    #[default]
    None,
    Scrap,
    Kevlar,
    PowerArmor,
}

/// Something lying on the ground that can be picked up.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub enum Item {
    Weapon { kind: WeaponKind, ammo: u16 },
    Armor { kind: ArmorKind, points: f32 },
    Emp,
    Medkit,
}

pub struct WeaponStats {
    pub name: &'static str,
    pub tier: u8,
    /// Ticks between shots.
    pub cooldown: u16,
    pub damage: f32,
    pub melee: bool,
    /// Melee reach or projectile speed (px/s).
    pub range_or_speed: f32,
    /// Projectile lifetime in ticks.
    pub lifetime: u16,
    pub pellets: u8,
    pub spread: f32,
    pub max_ammo: u16,
    /// Multiplier applied to damage absorbed by armor.
    pub armor_mult: f32,
    /// Railgun slugs go through walls and bodies.
    pub piercing: bool,
    pub color: Color,
}

impl WeaponKind {
    pub fn stats(self) -> WeaponStats {
        match self {
            WeaponKind::Stick => WeaponStats {
                name: "木棍",
                tier: 0,
                cooldown: 22,
                damage: 22.0,
                melee: true,
                range_or_speed: 78.0,
                lifetime: 0,
                pellets: 1,
                spread: 0.0,
                max_ammo: 0,
                armor_mult: 1.0,
                piercing: false,
                color: Color::srgb(0.65, 0.5, 0.3),
            },
            WeaponKind::Bow => WeaponStats {
                name: "猎弓",
                tier: 1,
                cooldown: 36,
                damage: 34.0,
                melee: false,
                range_or_speed: 950.0,
                lifetime: 60,
                pellets: 1,
                spread: 0.0,
                max_ammo: 14,
                armor_mult: 1.0,
                piercing: false,
                color: Color::srgb(0.85, 0.75, 0.5),
            },
            WeaponKind::PipeShotgun => WeaponStats {
                name: "管式霰弹枪",
                tier: 1,
                cooldown: 48,
                damage: 11.0,
                melee: false,
                range_or_speed: 1150.0,
                lifetime: 22,
                pellets: 7,
                spread: 0.38,
                max_ammo: 10,
                armor_mult: 1.0,
                piercing: false,
                color: Color::srgb(1.0, 0.6, 0.2),
            },
            WeaponKind::Rifle => WeaponStats {
                name: "制式步枪",
                tier: 2,
                cooldown: 7,
                damage: 11.0,
                melee: false,
                range_or_speed: 1600.0,
                lifetime: 50,
                pellets: 1,
                spread: 0.06,
                max_ammo: 45,
                armor_mult: 1.0,
                piercing: false,
                color: Color::srgb(1.0, 0.95, 0.4),
            },
            WeaponKind::Laser => WeaponStats {
                name: "脉冲激光",
                tier: 3,
                cooldown: 9,
                damage: 15.0,
                melee: false,
                range_or_speed: 2600.0,
                lifetime: 34,
                pellets: 1,
                spread: 0.0,
                max_ammo: 36,
                armor_mult: 2.0,
                piercing: false,
                color: Color::srgb(0.3, 1.0, 1.0),
            },
            WeaponKind::Railgun => WeaponStats {
                name: "轨道炮",
                tier: 4,
                cooldown: 64,
                damage: 95.0,
                melee: false,
                range_or_speed: 5200.0,
                lifetime: 24,
                pellets: 1,
                spread: 0.0,
                max_ammo: 6,
                armor_mult: 1.5,
                piercing: true,
                color: Color::srgb(0.85, 0.4, 1.0),
            },
        }
    }
}

impl ArmorKind {
    pub fn name(self) -> &'static str {
        match self {
            ArmorKind::None => "无护甲",
            ArmorKind::Scrap => "废铁甲",
            ArmorKind::Kevlar => "防弹衣",
            ArmorKind::PowerArmor => "动力装甲",
        }
    }

    pub fn tier(self) -> u8 {
        match self {
            ArmorKind::None => 0,
            ArmorKind::Scrap => 1,
            ArmorKind::Kevlar => 2,
            ArmorKind::PowerArmor => 4,
        }
    }

    pub fn max_points(self) -> f32 {
        match self {
            ArmorKind::None => 0.0,
            ArmorKind::Scrap => 35.0,
            ArmorKind::Kevlar => 70.0,
            ArmorKind::PowerArmor => 180.0,
        }
    }

    /// Movement speed multiplier: power armor is a walking tank.
    pub fn speed_mult(self) -> f32 {
        match self {
            ArmorKind::PowerArmor => 0.82,
            _ => 1.0,
        }
    }
}

impl Item {
    pub fn tier(self) -> u8 {
        match self {
            Item::Weapon { kind, .. } => kind.stats().tier,
            Item::Armor { kind, .. } => kind.tier(),
            Item::Emp | Item::Medkit => 1,
        }
    }

    pub fn label(self) -> String {
        match self {
            Item::Weapon { kind, ammo } => format!("{} ({ammo})", kind.stats().name),
            Item::Armor { kind, points } => format!("{} ({points:.0})", kind.name()),
            Item::Emp => "电磁脉冲雷".into(),
            Item::Medkit => "医疗包".into(),
        }
    }

    pub fn color(self) -> Color {
        tier_color(self.tier())
    }

    pub fn weapon(kind: WeaponKind) -> Self {
        Item::Weapon {
            kind,
            ammo: kind.stats().max_ammo,
        }
    }

    pub fn armor(kind: ArmorKind) -> Self {
        Item::Armor {
            kind,
            points: kind.max_points(),
        }
    }
}

pub fn tier_color(tier: u8) -> Color {
    match tier {
        0 => Color::srgb(0.6, 0.6, 0.6),
        1 => Color::srgb(0.55, 0.85, 0.45),
        2 => Color::srgb(0.35, 0.6, 1.0),
        3 => Color::srgb(0.75, 0.4, 1.0),
        _ => Color::srgb(1.0, 0.7, 0.15),
    }
}

/// Weighted loot table for each map zone tier.
pub fn roll_loot(zone_tier: u8, rng: &mut impl rand::Rng) -> Item {
    let table: &[(Item, u32)] = match zone_tier {
        1 => &[
            (Item::weapon(WeaponKind::Bow), 30),
            (Item::weapon(WeaponKind::PipeShotgun), 30),
            (Item::armor(ArmorKind::Scrap), 25),
            (Item::Medkit, 20),
            (Item::Emp, 12),
        ],
        2 => &[
            (Item::weapon(WeaponKind::Rifle), 30),
            (Item::weapon(WeaponKind::PipeShotgun), 15),
            (Item::armor(ArmorKind::Kevlar), 25),
            (Item::Emp, 15),
            (Item::Medkit, 15),
        ],
        3 => &[
            (Item::weapon(WeaponKind::Laser), 40),
            (Item::weapon(WeaponKind::Rifle), 20),
            (Item::armor(ArmorKind::Kevlar), 25),
            (Item::Emp, 15),
        ],
        _ => &[
            (Item::armor(ArmorKind::PowerArmor), 50),
            (Item::weapon(WeaponKind::Railgun), 50),
        ],
    };
    let total: u32 = table.iter().map(|(_, w)| w).sum();
    let mut roll = rng.random_range(0..total);
    for (item, weight) in table {
        if roll < *weight {
            return *item;
        }
        roll -= weight;
    }
    table[0].0
}
