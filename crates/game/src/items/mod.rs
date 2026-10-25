//! Gear of the fallen civilization, grouped by technology tier.
//!
//! T0 primitive → T1 industrial scrap → T2 modern military → T3 information age → T4 pre-collapse apex.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

mod weapons;

pub use weapons::*;

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
            Item::Weapon { kind, .. } => kind.spec().tier(),
            Item::Armor { kind, .. } => kind.tier(),
            Item::Emp | Item::Medkit => 1,
        }
    }

    pub fn label(self) -> String {
        match self {
            Item::Weapon { kind, ammo } => format!("{} ({ammo})", kind.spec().name()),
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
            ammo: kind.max_ammo(),
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

/// Weighted loot table for each map zone tier (0 is a medical cache).
pub fn roll_loot(zone_tier: u8, rng: &mut impl rand::Rng) -> Item {
    let table: &[(Item, u32)] = match zone_tier {
        // Medical cache.
        0 => &[(Item::Medkit, 70), (Item::Emp, 30)],
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
