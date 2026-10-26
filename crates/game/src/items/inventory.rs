//! The backpack: a typed hotbar (two guns, a melee weapon, throwables), a bag of spare weapons
//! and an ammo pouch with one stack per `AmmoKind`. Guns never hold rounds themselves, so one
//! that runs dry stays in the backpack until more ammo turns up.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{AmmoKind, Item, WeaponKind};
use crate::evolution::Evolution;

/// Hotbar gun slots (keys 1 and 2).
pub const GUN_SLOTS: usize = 2;
/// Hotbar slot of the melee weapon (key 3): the fallback when no gun can fire.
pub const MELEE_SLOT: u8 = 2;
/// Hotbar slot of the throwables (key 4): firing with it selected throws an EMP.
pub const THROW_SLOT: u8 = 3;
pub const HOTBAR_SLOTS: u8 = 4;
/// Spare weapons, rearranged onto the hotbar with the backpack panel (B).
pub const BAG_SLOTS: usize = 6;
/// Slot ids used by [`InventoryMove`]: the hotbar (0..4), then the bag.
pub const SLOT_COUNT: u8 = HOTBAR_SLOTS + BAG_SLOTS as u8;
/// EMPs carried before 电磁专家.
pub const MAX_EMPS: u8 = 3;

/// A combatant's backpack. Replicated to everyone, since the weapon in hand is drawn.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Loadout {
    pub guns: [Option<WeaponKind>; GUN_SLOTS],
    pub melee: WeaponKind,
    /// The throwable slot's stack.
    pub emps: u8,
    pub bag: [Option<WeaponKind>; BAG_SLOTS],
    /// Rounds per `AmmoKind::index`.
    pub ammo: [u16; AmmoKind::ALL.len()],
    /// Selected hotbar slot, `0..HOTBAR_SLOTS`.
    pub selected: u8,
}

impl Default for Loadout {
    fn default() -> Self {
        Self {
            guns: [None; GUN_SLOTS],
            melee: WeaponKind::Stick,
            emps: 0,
            bag: [None; BAG_SLOTS],
            ammo: [0; AmmoKind::ALL.len()],
            selected: MELEE_SLOT,
        }
    }
}

/// Client → server: swap the contents of two backpack slots (ids as in [`Loadout::slot`]).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct InventoryMove {
    pub from: u8,
    pub to: u8,
}

impl Loadout {
    pub fn rounds(&self, kind: AmmoKind) -> u16 {
        self.ammo[kind.index()]
    }

    /// Whether `weapon` can attack right now: melee always, a gun if its stack covers a shot.
    pub fn usable(&self, weapon: WeaponKind) -> bool {
        weapon
            .ammo()
            .is_none_or(|kind| self.rounds(kind) >= weapon.ammo_cost())
    }

    /// The weapon in slot `id` (hotbar 0..4, then the bag); the throwable slot holds none.
    pub fn slot(&self, id: u8) -> Option<WeaponKind> {
        match id {
            0..2 => self.guns[id as usize],
            MELEE_SLOT => Some(self.melee),
            THROW_SLOT => None,
            _ => self
                .bag
                .get((id - HOTBAR_SLOTS) as usize)
                .copied()
                .flatten(),
        }
    }

    /// The weapon in hand: the selected gun if it can fire, otherwise the melee weapon.
    pub fn weapon(&self) -> WeaponKind {
        match self.slot(self.selected) {
            Some(gun) if self.usable(gun) => gun,
            _ => self.melee,
        }
    }

    /// Firing throws an EMP instead of attacking.
    pub fn throwing(&self) -> bool {
        self.selected == THROW_SLOT && self.emps > 0
    }

    /// Every weapon carried apart from the melee weapon.
    pub fn guns(&self) -> impl Iterator<Item = WeaponKind> + '_ {
        self.guns.iter().chain(&self.bag).flatten().copied()
    }

    pub fn owns(&self, weapon: WeaponKind) -> bool {
        weapon == self.melee || self.guns().any(|w| w == weapon)
    }

    /// Highest tier of anything carried (for the underdog bonus and giant slayer).
    pub fn best_tier(&self) -> u8 {
        self.guns()
            .chain([self.melee])
            .map(|w| w.spec().tier())
            .max()
            .unwrap_or(0)
    }

    pub fn has_room(&self) -> bool {
        self.guns.contains(&None) || self.bag.contains(&None)
    }

    /// The hotbar slot to fall back to: the highest-tier gun that can fire, else melee.
    pub fn best_slot(&self) -> u8 {
        (0..GUN_SLOTS as u8)
            .filter_map(|i| self.guns[i as usize].map(|w| (i, w)))
            .filter(|(_, w)| self.usable(*w))
            .max_by_key(|(i, w)| (w.spec().tier(), std::cmp::Reverse(*i)))
            .map_or(MELEE_SLOT, |(i, _)| i)
    }

    /// The selection can't attack: a dry gun, an empty gun slot or no EMPs left.
    pub fn stuck(&self) -> bool {
        match self.selected {
            THROW_SLOT => self.emps == 0,
            MELEE_SLOT => false,
            _ => self.slot(self.selected).is_none_or(|w| !self.usable(w)),
        }
    }

    /// When the selection is [`stuck`](Self::stuck), switch to the best weapon that can attack.
    pub fn auto_switch(&mut self) {
        if self.stuck() {
            self.selected = self.best_slot();
        }
    }

    /// Take the rounds for one trigger pull of `weapon`. False if the stack can't cover it.
    pub fn spend(&mut self, weapon: WeaponKind) -> bool {
        let Some(kind) = weapon.ammo() else {
            return true;
        };
        let stack = &mut self.ammo[kind.index()];
        let Some(left) = stack.checked_sub(weapon.ammo_cost()) else {
            return false;
        };
        *stack = left;
        true
    }

    /// Add rounds up to the pouch cap; returns how many didn't fit.
    pub fn add_ammo(&mut self, kind: AmmoKind, rounds: u16, evo: &Evolution) -> u16 {
        let cap = evo.ammo_cap(kind);
        let stack = &mut self.ammo[kind.index()];
        let fits = rounds.min(cap.saturating_sub(*stack));
        *stack += fits;
        rounds - fits
    }

    /// Put a weapon in the first free hotbar gun slot, else the bag. With no room left it
    /// replaces the selected gun (the first gun slot if a gun isn't selected) and returns it.
    pub fn add_weapon(&mut self, weapon: WeaponKind) -> Option<WeaponKind> {
        if weapon.melee().is_some() {
            let old = std::mem::replace(&mut self.melee, weapon);
            return (old != WeaponKind::Stick).then_some(old);
        }
        if let Some(free) = self.guns.iter_mut().find(|s| s.is_none()) {
            *free = Some(weapon);
            return None;
        }
        if let Some(free) = self.bag.iter_mut().find(|s| s.is_none()) {
            *free = Some(weapon);
            return None;
        }
        let at = if (self.selected as usize) < GUN_SLOTS {
            self.selected as usize
        } else {
            0
        };
        self.guns[at].replace(weapon)
    }

    /// Swap two slots, if each can hold what lands in it: hotbar gun slots take guns, the bag
    /// takes anything, and the melee and throwable slots stay put.
    pub fn move_slots(&mut self, from: u8, to: u8) -> bool {
        let movable = |id: u8| id < SLOT_COUNT && id != MELEE_SLOT && id != THROW_SLOT;
        if from == to || !movable(from) || !movable(to) {
            return false;
        }
        let fits = |id: u8, w: Option<WeaponKind>| {
            id >= HOTBAR_SLOTS || w.is_none_or(|w| w.ranged().is_some())
        };
        let (a, b) = (self.slot(from), self.slot(to));
        if !fits(to, a) || !fits(from, b) {
            return false;
        }
        *self.slot_mut(from) = b;
        *self.slot_mut(to) = a;
        true
    }

    fn slot_mut(&mut self, id: u8) -> &mut Option<WeaponKind> {
        match id {
            0..2 => &mut self.guns[id as usize],
            _ => &mut self.bag[(id - HOTBAR_SLOTS) as usize],
        }
    }

    /// Select a hotbar slot.
    pub fn select(&mut self, slot: u8) {
        if slot < HOTBAR_SLOTS {
            self.selected = slot;
        }
    }

    /// Everything in the backpack, as it drops on death (the stick stays behind).
    pub fn contents(&self) -> Vec<Item> {
        let mut items: Vec<Item> = self.guns().map(Item::Weapon).collect();
        if self.melee != WeaponKind::Stick {
            items.push(Item::Weapon(self.melee));
        }
        for kind in AmmoKind::ALL {
            let rounds = self.rounds(kind);
            if rounds > 0 {
                items.push(Item::Ammo { kind, rounds });
            }
        }
        items.extend((0..self.emps).map(|_| Item::Emp));
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(guns: &[WeaponKind]) -> Loadout {
        let mut loadout = Loadout::default();
        for gun in guns {
            assert_eq!(loadout.add_weapon(*gun), None);
        }
        loadout
    }

    #[test]
    fn a_dry_gun_is_kept_and_the_stick_takes_over() {
        let evo = Evolution::default();
        let mut loadout = with(&[WeaponKind::Rifle]);
        loadout.select(0);
        assert_eq!(loadout.weapon(), WeaponKind::Stick);
        loadout.add_ammo(AmmoKind::Bullet, 2, &evo);
        assert_eq!(loadout.weapon(), WeaponKind::Rifle);
        assert!(loadout.spend(WeaponKind::Rifle));
        assert!(loadout.spend(WeaponKind::Rifle));
        assert!(!loadout.spend(WeaponKind::Rifle));
        assert_eq!(loadout.weapon(), WeaponKind::Stick);
        assert_eq!(loadout.guns[0], Some(WeaponKind::Rifle));
    }

    #[test]
    fn guns_draw_from_their_own_stack() {
        let evo = Evolution::default();
        let mut loadout = with(&[WeaponKind::Laser, WeaponKind::Railgun]);
        loadout.add_ammo(AmmoKind::Cell, 13, &evo);
        loadout.add_ammo(AmmoKind::Bullet, 5, &evo);
        assert!(loadout.spend(WeaponKind::Railgun));
        assert_eq!(loadout.rounds(AmmoKind::Cell), 7);
        assert!(loadout.spend(WeaponKind::Railgun));
        // One cell left: enough for the laser, not the railgun.
        assert!(!loadout.usable(WeaponKind::Railgun));
        assert!(loadout.usable(WeaponKind::Laser));
        assert_eq!(loadout.rounds(AmmoKind::Bullet), 5);
    }

    #[test]
    fn auto_switch_picks_the_best_gun_with_ammo_else_melee() {
        let evo = Evolution::default();
        let mut loadout = with(&[WeaponKind::Bow, WeaponKind::Rifle]);
        loadout.add_ammo(AmmoKind::Arrow, 5, &evo);
        loadout.add_ammo(AmmoKind::Bullet, 1, &evo);
        loadout.select(1);
        assert!(loadout.spend(WeaponKind::Rifle));
        loadout.auto_switch();
        assert_eq!(loadout.selected, 0, "falls back to the bow");
        loadout.ammo = [0; 4];
        loadout.auto_switch();
        assert_eq!(loadout.selected, MELEE_SLOT);
        // Higher tier wins, and only hotbar guns count.
        let mut loadout = with(&[WeaponKind::Bow, WeaponKind::PipeShotgun, WeaponKind::Laser]);
        loadout.add_ammo(AmmoKind::Arrow, 5, &evo);
        loadout.add_ammo(AmmoKind::Cell, 5, &evo);
        assert_eq!(loadout.best_slot(), 0);
        loadout.select(THROW_SLOT);
        loadout.auto_switch();
        assert_eq!(loadout.selected, 0, "no EMPs left to throw");
    }

    #[test]
    fn pouch_caps_grow_with_ammo_recycler() {
        let mut loadout = Loadout::default();
        let evo = Evolution::default();
        let cap = AmmoKind::Shell.base_cap();
        assert_eq!(loadout.add_ammo(AmmoKind::Shell, cap + 7, &evo), 7);
        assert_eq!(loadout.rounds(AmmoKind::Shell), cap);
    }

    #[test]
    fn a_full_backpack_swaps_with_the_selected_gun() {
        let mut loadout = with(&[
            WeaponKind::Bow,
            WeaponKind::Rifle,
            WeaponKind::Laser,
            WeaponKind::PipeShotgun,
            WeaponKind::Bow,
            WeaponKind::Rifle,
            WeaponKind::Laser,
            WeaponKind::PipeShotgun,
        ]);
        assert!(!loadout.has_room());
        loadout.select(1);
        assert_eq!(
            loadout.add_weapon(WeaponKind::Railgun),
            Some(WeaponKind::Rifle)
        );
        assert_eq!(loadout.guns[1], Some(WeaponKind::Railgun));
        // With the stick in hand the first gun slot gives way.
        loadout.select(MELEE_SLOT);
        assert_eq!(loadout.add_weapon(WeaponKind::Rifle), Some(WeaponKind::Bow));
        assert_eq!(loadout.guns[0], Some(WeaponKind::Rifle));
    }

    #[test]
    fn moves_keep_guns_on_gun_slots_and_pin_melee_and_throwables() {
        let mut loadout = with(&[WeaponKind::Bow, WeaponKind::Rifle, WeaponKind::Laser]);
        assert_eq!(loadout.bag[0], Some(WeaponKind::Laser));
        assert!(loadout.move_slots(HOTBAR_SLOTS, 0));
        assert_eq!(loadout.guns[0], Some(WeaponKind::Laser));
        assert_eq!(loadout.bag[0], Some(WeaponKind::Bow));
        // Into an empty bag slot.
        assert!(loadout.move_slots(1, HOTBAR_SLOTS + 5));
        assert_eq!(loadout.guns[1], None);
        assert!(!loadout.move_slots(MELEE_SLOT, HOTBAR_SLOTS + 1));
        assert!(!loadout.move_slots(0, THROW_SLOT));
        assert!(!loadout.move_slots(0, SLOT_COUNT));
    }

    #[test]
    fn contents_hold_everything_but_the_stick() {
        let evo = Evolution::default();
        let mut loadout = with(&[WeaponKind::Bow, WeaponKind::Rifle, WeaponKind::Laser]);
        loadout.add_ammo(AmmoKind::Bullet, 30, &evo);
        loadout.emps = 2;
        let items = loadout.contents();
        assert_eq!(items.len(), 3 + 1 + 2);
        assert!(items.contains(&Item::Ammo {
            kind: AmmoKind::Bullet,
            rounds: 30
        }));
        assert!(!items.contains(&Item::Weapon(WeaponKind::Stick)));
    }
}
