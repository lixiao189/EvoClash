//! Gear of the fallen civilization, grouped by technology tier.
//!
//! T0 primitive → T1 industrial scrap → T2 modern military → T3 information age → T4 pre-collapse apex.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

mod inventory;
mod weapons;

pub use inventory::*;
pub use weapons::*;

/// EMPs a combatant can carry before 电磁专家.
pub const MAX_EMPS: u8 = 3;

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
    /// Just the gun: it fires from the matching stack in the ammo pouch.
    Weapon(WeaponKind),
    Armor {
        kind: ArmorKind,
        points: f32,
    },
    Emp,
    Medkit,
    /// Rounds for the ammo pouch.
    Ammo {
        kind: AmmoKind,
        rounds: u16,
    },
    /// Everything a fallen combatant carried. Only the server knows what is inside.
    Bag,
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
            Item::Weapon(kind) => kind.spec().tier(),
            Item::Armor { kind, .. } => kind.tier(),
            Item::Ammo { kind, .. } => kind.tier(),
            Item::Emp | Item::Medkit | Item::Bag => 1,
        }
    }

    pub fn label(self) -> String {
        match self {
            Item::Weapon(kind) => kind.spec().name().into(),
            Item::Armor { kind, points } => format!("{} ({points:.0})", kind.name()),
            Item::Emp => "电磁脉冲雷".into(),
            Item::Medkit => "医疗包".into(),
            Item::Ammo { kind, rounds } => format!("{} ×{rounds}", kind.name()),
            Item::Bag => "遗落的背包".into(),
        }
    }

    pub fn color(self) -> Color {
        tier_color(self.tier())
    }

    pub fn weapon(kind: WeaponKind) -> Self {
        Item::Weapon(kind)
    }

    /// A box of `kind`: `AmmoKind::box_rounds` of them.
    pub fn ammo(kind: AmmoKind) -> Self {
        Item::Ammo {
            kind,
            rounds: kind.box_rounds(),
        }
    }

    /// The box of ammo that comes with a gun found in the world, so it can be fired at once.
    pub fn matching_ammo(self) -> Option<Item> {
        match self {
            Item::Weapon(kind) => kind.ammo().map(Item::ammo),
            _ => None,
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
        0 => &[(Item::Medkit, 70), (Item::Emp, 30)],
        1 => &[
            (Item::weapon(WeaponKind::Bow), 25),
            (Item::weapon(WeaponKind::PipeShotgun), 25),
            (Item::armor(ArmorKind::Scrap), 25),
            (Item::Medkit, 20),
            (Item::Emp, 12),
            (Item::ammo(AmmoKind::Arrow), 20),
            (Item::ammo(AmmoKind::Shell), 20),
        ],
        2 => &[
            (Item::weapon(WeaponKind::Rifle), 30),
            (Item::weapon(WeaponKind::PipeShotgun), 12),
            (Item::armor(ArmorKind::Kevlar), 25),
            (Item::Emp, 15),
            (Item::Medkit, 15),
            (Item::ammo(AmmoKind::Bullet), 35),
            (Item::ammo(AmmoKind::Shell), 12),
        ],
        3 => &[
            (Item::weapon(WeaponKind::Laser), 40),
            (Item::weapon(WeaponKind::Rifle), 20),
            (Item::armor(ArmorKind::Kevlar), 25),
            (Item::Emp, 15),
            (Item::ammo(AmmoKind::Cell), 35),
            (Item::ammo(AmmoKind::Bullet), 15),
        ],
        _ => &[
            (Item::armor(ArmorKind::PowerArmor), 50),
            (Item::weapon(WeaponKind::Railgun), 50),
        ],
    };
    pick(table, rng)
}

/// A loose box of ammo of a family found in a zone of this tier.
pub fn roll_ammo(zone_tier: u8, rng: &mut impl rand::Rng) -> Item {
    let table: &[(Item, u32)] = match zone_tier {
        0 | 1 => &[
            (Item::ammo(AmmoKind::Arrow), 50),
            (Item::ammo(AmmoKind::Shell), 50),
        ],
        2 => &[
            (Item::ammo(AmmoKind::Bullet), 70),
            (Item::ammo(AmmoKind::Shell), 30),
        ],
        _ => &[
            (Item::ammo(AmmoKind::Cell), 60),
            (Item::ammo(AmmoKind::Bullet), 40),
        ],
    };
    pick(table, rng)
}

/// Loot from a crate or safe: `weapon_pct` percent of the time a gun from the tier's table
/// (crates in a medical cache zone still hold tier-1 guns), otherwise the zone's usual loot.
pub fn roll_container(zone_tier: u8, weapon_pct: u32, rng: &mut impl rand::Rng) -> Item {
    if rng.random_range(0..100) >= weapon_pct {
        return roll_loot(zone_tier, rng);
    }
    let table: &[(Item, u32)] = match zone_tier {
        0 | 1 => &[
            (Item::weapon(WeaponKind::Bow), 45),
            (Item::weapon(WeaponKind::PipeShotgun), 55),
        ],
        2 => &[
            (Item::weapon(WeaponKind::Rifle), 65),
            (Item::weapon(WeaponKind::PipeShotgun), 35),
        ],
        3 => &[
            (Item::weapon(WeaponKind::Laser), 65),
            (Item::weapon(WeaponKind::Rifle), 35),
        ],
        _ => &[
            (Item::weapon(WeaponKind::Railgun), 60),
            (Item::weapon(WeaponKind::Laser), 40),
        ],
    };
    pick(table, rng)
}

fn pick(table: &[(Item, u32)], rng: &mut impl rand::Rng) -> Item {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers_hold_guns_in_every_zone() {
        let mut rng = rand::rng();
        for tier in 0..=4 {
            let guns = (0..400)
                .filter(|_| matches!(roll_container(tier, 40, &mut rng), Item::Weapon(_)))
                .count();
            assert!(guns > 100, "tier {tier}: only {guns}/400 guns");
        }
    }

    #[test]
    fn the_vault_never_drops_ammo_and_lab_safes_can_give_power_armor() {
        let mut rng = rand::rng();
        assert!((0..500).all(|_| !matches!(roll_loot(4, &mut rng), Item::Ammo { .. })));
        let armor = (0..400)
            .filter(|_| matches!(roll_container(4, 50, &mut rng), Item::Armor { .. }))
            .count();
        assert!(armor >= 60, "only {armor}/400 power armor");
    }

    #[test]
    fn found_guns_come_with_ammo_they_can_fire() {
        for kind in [
            WeaponKind::Bow,
            WeaponKind::PipeShotgun,
            WeaponKind::Rifle,
            WeaponKind::Laser,
            WeaponKind::Railgun,
        ] {
            let Some(Item::Ammo { kind: ammo, rounds }) = Item::weapon(kind).matching_ammo() else {
                panic!("{kind:?} has no ammo box");
            };
            assert_eq!(Some(ammo), kind.ammo());
            assert!(rounds >= 6 * kind.ammo_cost(), "{kind:?}: {rounds} rounds");
            assert!(ammo.base_cap() >= 3 * rounds);
        }
        assert_eq!(Item::weapon(WeaponKind::Stick).matching_ammo(), None);
        assert_eq!(WeaponKind::Railgun.ammo_cost(), 6);
    }

    #[test]
    fn loose_ammo_matches_the_zone() {
        let mut rng = rand::rng();
        for _ in 0..200 {
            let Item::Ammo { kind, .. } = roll_ammo(1, &mut rng) else {
                panic!("not ammo");
            };
            assert!(matches!(kind, AmmoKind::Arrow | AmmoKind::Shell));
        }
    }
}
