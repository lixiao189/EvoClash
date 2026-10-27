//! Death prep: 废料 (scrap), the death-screen shop, the respawn window and the commands a
//! player sends while dead (or, for spending evolution points, at any time).
//!
//! Scrap is earned from monsters, crates, safes and kills, kept on death and spent only on
//! the death screen. Purchases wait in `Wallet::cart` and are handed out at respawn through
//! the same function that equips pickups. The shop tops out at T2.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::evolution::{EvoPath, Evolution, Perk};
use crate::items::{AmmoKind, ArmorKind, Item, MAX_EMPS, WeaponKind};
use crate::protocol::MonsterKind;

const SEC: u16 = 64;

/// You stay dead at least this long; after it you may leave early.
pub const RESPAWN_MIN_TICKS: u16 = 4 * SEC;
/// Playing deaths count `Vitals::respawn_in` down from here. It is not an automatic revive:
/// a human stays on the death screen (the count sticks at 1) until they deploy.
pub const RESPAWN_MAX_TICKS: u16 = 8 * SEC;
/// The death card is shown this long before the respec and shop panels.
pub const DEATH_CARD_TICKS: u16 = 2 * SEC;
/// The highest tier the shop sells; T3 and T4 come only from the map and events.
pub const SHOP_MAX_TIER: u8 = 2;

/// Whether someone dead with `respawn_in` ticks left may deploy now.
pub fn can_deploy(respawn_in: u16) -> bool {
    respawn_in <= RESPAWN_MAX_TICKS - RESPAWN_MIN_TICKS
}

/// Seconds dead so far, from the replicated `Vitals::respawn_in`.
pub fn dead_ticks(respawn_in: u16) -> u16 {
    RESPAWN_MAX_TICKS.saturating_sub(respawn_in)
}

/// Scrap for killing a monster (elites are worth `ELITE_XP_MULT` times as much).
pub fn monster_scrap(kind: MonsterKind) -> u32 {
    match kind {
        MonsterKind::Roach | MonsterKind::Dog => 4,
        MonsterKind::Ghoul | MonsterKind::Drone => 6,
        MonsterKind::SporePod => 8,
        MonsterKind::Spitter => 10,
        MonsterKind::Crawler | MonsterKind::Raider => 15,
        // Split by damage share like its XP.
        MonsterKind::Behemoth => 80,
    }
}

pub fn crate_scrap(tier: u8) -> u32 {
    4 + 2 * tier as u32
}

pub fn safe_scrap(tier: u8) -> u32 {
    15 + 5 * tier as u32
}

pub const VAULT_SAFE_SCRAP: u32 = 40;

pub fn player_kill_scrap(bounty: bool) -> u32 {
    10 + if bounty { 10 } else { 0 }
}

/// What the death-screen shop sells.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum ShopItem {
    Bow,
    PipeShotgun,
    Rifle,
    ScrapArmor,
    Kevlar,
    Emp,
    /// Not in `STOCK` yet: at respawn you are at full health, so a medkit only makes sense
    /// once the backpack can carry one.
    Medkit,
    /// Not in `STOCK` yet: a bought gun already comes with a box of its ammo.
    Ammo,
}

impl ShopItem {
    /// What the death screen offers, in display order.
    pub const STOCK: [ShopItem; 6] = [
        ShopItem::Bow,
        ShopItem::PipeShotgun,
        ShopItem::Rifle,
        ShopItem::ScrapArmor,
        ShopItem::Kevlar,
        ShopItem::Emp,
    ];

    /// One normal life earns about one T1 item; rifle + kevlar takes two or three lives.
    pub fn price(self) -> u32 {
        match self {
            ShopItem::Bow => 35,
            ShopItem::PipeShotgun => 40,
            ShopItem::Rifle => 70,
            ShopItem::ScrapArmor => 30,
            ShopItem::Kevlar => 55,
            ShopItem::Emp => 15,
            ShopItem::Medkit => 10,
            ShopItem::Ammo => 10,
        }
    }

    /// The item handed out at respawn.
    pub fn item(self) -> Item {
        match self {
            ShopItem::Bow => Item::weapon(WeaponKind::Bow),
            ShopItem::PipeShotgun => Item::weapon(WeaponKind::PipeShotgun),
            ShopItem::Rifle => Item::weapon(WeaponKind::Rifle),
            ShopItem::ScrapArmor => Item::armor(ArmorKind::Scrap),
            ShopItem::Kevlar => Item::armor(ArmorKind::Kevlar),
            ShopItem::Emp => Item::Emp,
            ShopItem::Medkit => Item::Medkit,
            ShopItem::Ammo => Item::ammo(AmmoKind::Bullet),
        }
    }

    pub fn tier(self) -> u8 {
        self.item().tier()
    }

    pub fn name(self) -> &'static str {
        match self {
            ShopItem::Bow => WeaponKind::Bow.spec().name(),
            ShopItem::PipeShotgun => WeaponKind::PipeShotgun.spec().name(),
            ShopItem::Rifle => WeaponKind::Rifle.spec().name(),
            ShopItem::ScrapArmor => ArmorKind::Scrap.name(),
            ShopItem::Kevlar => ArmorKind::Kevlar.name(),
            ShopItem::Emp => "电磁脉冲雷",
            ShopItem::Medkit => "医疗包",
            ShopItem::Ammo => "弹药箱",
        }
    }

    /// Items that share a cart slot: one gun, one armor; the rest stack.
    fn slot(self) -> Slot {
        match self.item() {
            Item::Weapon(_) => Slot::Weapon,
            Item::Armor { .. } => Slot::Armor,
            _ => Slot::Stack,
        }
    }

    /// How many of this the cart may hold.
    pub fn limit(self, evo: &Evolution) -> usize {
        match self {
            ShopItem::Emp => evo.emp_cap(MAX_EMPS) as usize,
            ShopItem::Medkit | ShopItem::Ammo => 3,
            _ => 1,
        }
    }
}

#[derive(PartialEq)]
enum Slot {
    Weapon,
    Armor,
    Stack,
}

/// How the last life ended, for the death card.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
pub struct DeathRecap {
    /// Player name, monster name, or "废土".
    pub killer: String,
    /// Weapon name, or how the wasteland did it (same as the kill feed).
    pub how: String,
    /// The killer's leading path and level; `None`/0 when it wasn't a combatant.
    pub killer_path: Option<EvoPath>,
    pub killer_level: u8,
    /// The life just lost.
    pub alive_secs: u32,
    pub scrap: u32,
    pub kills: u32,
}

/// Scrap, the death-screen cart and the last death. Kept on death, reset when a match starts.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Default)]
pub struct Wallet {
    pub scrap: u32,
    /// Bought this death, already paid for, handed out at respawn.
    pub cart: Vec<ShopItem>,
    /// What was handed out last respawn (re-bought by 沿用上次配置).
    pub last_cart: Vec<ShopItem>,
    /// Set when you die; the client shows it while dead.
    pub recap: Option<DeathRecap>,
}

impl Wallet {
    /// Scrap a purchase would really cost now: buying a second gun or armor swaps it for the
    /// one in the cart, which is refunded.
    pub fn net_price(&self, item: ShopItem) -> i64 {
        let swapped = match item.slot() {
            Slot::Stack => None,
            slot => self.cart.iter().find(|c| c.slot() == slot && **c != item),
        };
        item.price() as i64 - swapped.map_or(0, |c| c.price() as i64)
    }

    pub fn can_buy(&self, item: ShopItem, evo: &Evolution) -> bool {
        let held = self.cart.iter().filter(|c| **c == item).count();
        ShopItem::STOCK.contains(&item)
            && held < item.limit(evo)
            && self.net_price(item) <= self.scrap as i64
    }

    /// Pay for `item` and put it in the cart (swapping out a gun or armor of another kind).
    /// The server only allows this while dead.
    pub fn buy(&mut self, item: ShopItem, evo: &Evolution) -> bool {
        if !self.can_buy(item, evo) {
            return false;
        }
        if item.slot() != Slot::Stack
            && let Some(i) = self.cart.iter().position(|c| c.slot() == item.slot())
        {
            let old = self.cart.remove(i);
            self.scrap += old.price();
        }
        self.scrap -= item.price();
        self.cart.push(item);
        true
    }

    /// Take one `item` back out of the cart at full refund.
    pub fn unbuy(&mut self, item: ShopItem) -> bool {
        let Some(i) = self.cart.iter().position(|c| *c == item) else {
            return false;
        };
        self.cart.remove(i);
        self.scrap += item.price();
        true
    }

    /// Empty the cart (refunded) and buy `last_cart` again, as much of it as is affordable,
    /// in order.
    pub fn rebuy_last(&mut self, evo: &Evolution) {
        for item in std::mem::take(&mut self.cart) {
            self.scrap += item.price();
        }
        for item in self.last_cart.clone() {
            self.buy(item, evo);
        }
    }

    /// Hand the cart over at respawn: returns the items in grant order and remembers them.
    /// A gun comes with a box of its ammo, like one found in the world.
    pub fn take_cart(&mut self) -> Vec<Item> {
        let cart = std::mem::take(&mut self.cart);
        if !cart.is_empty() {
            self.last_cart = cart.clone();
        }
        cart.into_iter()
            .map(ShopItem::item)
            .flat_map(|item| [Some(item), item.matching_ammo()])
            .flatten()
            .collect()
    }
}

/// Client → server, reliable and ordered. Everything but `Spend` is ignored while alive.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PlayerCommand {
    /// Put a point into a perk (alive or dead).
    Spend(Perk),
    /// Take a point back out (dead only).
    Refund(Perk),
    /// Refund every point (dead only).
    ResetPerks,
    Buy(ShopItem),
    Unbuy(ShopItem),
    /// Respawn now (dead at least `RESPAWN_MIN_TICKS`).
    Deploy,
    /// 沿用上次配置: restore the perks you died with, re-buy what you bought last time, and
    /// deploy (or deploy as soon as allowed).
    DeployLast,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shop_tops_out_at_t2() {
        for item in ShopItem::STOCK {
            assert!(item.tier() <= SHOP_MAX_TIER, "{item:?}");
            assert!(item.tier() >= 1, "{item:?}");
        }
        assert!(ShopItem::STOCK.iter().any(|i| i.tier() == SHOP_MAX_TIER));
    }

    #[test]
    fn prices_hit_the_targets() {
        // A normal life: four small monsters, three outer-block crates and a kill (44).
        let life =
            4 * monster_scrap(MonsterKind::Roach) + 3 * crate_scrap(1) + player_kill_scrap(false);
        let t1 = [ShopItem::Bow, ShopItem::PipeShotgun, ShopItem::ScrapArmor];
        for item in t1 {
            assert!(item.price() <= life, "{item:?} costs more than a life");
        }
        let t2_set = ShopItem::Rifle.price() + ShopItem::Kevlar.price();
        assert!(
            (2 * life..=3 * life).contains(&t2_set),
            "{t2_set} vs {life}"
        );
    }

    fn rich(scrap: u32) -> Wallet {
        Wallet { scrap, ..default() }
    }

    #[test]
    fn buying_swaps_guns_and_refunds() {
        let evo = Evolution::default();
        let mut w = rich(100);
        assert!(w.buy(ShopItem::Bow, &evo));
        assert!(!w.buy(ShopItem::Bow, &evo), "one gun at a time");
        assert_eq!(w.net_price(ShopItem::Rifle), 35);
        assert!(w.buy(ShopItem::Rifle, &evo));
        assert_eq!((w.scrap, w.cart.clone()), (30, vec![ShopItem::Rifle]));
        assert!(!w.buy(ShopItem::Kevlar, &evo), "too poor");
        assert!(w.unbuy(ShopItem::Rifle));
        assert!(!w.unbuy(ShopItem::Rifle));
        assert_eq!(w.scrap, 100);
        assert!(!w.buy(ShopItem::Medkit, &evo), "not stocked yet");
    }

    #[test]
    fn emps_stack_up_to_the_cap() {
        let mut evo = Evolution::default();
        let mut w = rich(1000);
        for _ in 0..MAX_EMPS {
            assert!(w.buy(ShopItem::Emp, &evo));
        }
        assert!(!w.buy(ShopItem::Emp, &evo));
        evo.ranks[Perk::EmpExpert.index()] = 1;
        assert!(w.buy(ShopItem::Emp, &evo));
    }

    #[test]
    fn rebuy_last_buys_what_it_can() {
        let evo = Evolution::default();
        let mut w = rich(150);
        w.buy(ShopItem::Rifle, &evo);
        w.buy(ShopItem::Kevlar, &evo);
        assert_eq!(w.take_cart().len(), 3, "rifle, its bullets and kevlar");
        assert!(w.cart.is_empty());
        w.scrap = 80;
        w.buy(ShopItem::Emp, &evo);
        w.rebuy_last(&evo);
        assert_eq!((w.scrap, w.cart.clone()), (10, vec![ShopItem::Rifle]));
        // An empty respawn keeps the previous build to fall back on.
        w.cart.clear();
        assert!(w.take_cart().is_empty());
        assert_eq!(w.last_cart, vec![ShopItem::Rifle, ShopItem::Kevlar]);
    }

    #[test]
    fn respawn_window() {
        assert!(!can_deploy(RESPAWN_MAX_TICKS));
        assert!(!can_deploy(RESPAWN_MAX_TICKS - RESPAWN_MIN_TICKS + 1));
        assert!(can_deploy(RESPAWN_MAX_TICKS - RESPAWN_MIN_TICKS));
        assert_eq!(dead_ticks(RESPAWN_MAX_TICKS), 0);
        assert_eq!(RESPAWN_MIN_TICKS, 4 * 64);
        assert_eq!(RESPAWN_MAX_TICKS, 8 * 64);
    }
}
