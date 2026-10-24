//! Deterministic gameplay rules shared by server (authority) and client (prediction).

use bevy::prelude::*;

use crate::map::ArenaMap;
use crate::net::TICK_HZ;
use crate::protocol::{Dash, PlayerInput, Vitals};

pub const DT: f32 = 1.0 / TICK_HZ as f32;

pub const MAX_HP: f32 = 100.0;
pub const PLAYER_RADIUS: f32 = 22.0;
pub const PLAYER_SPEED: f32 = 340.0;
pub const DASH_SPEED: f32 = 1150.0;
pub const DASH_TICKS: u16 = 9;
pub const DASH_COOLDOWN: u16 = 70;
pub const EMP_SLOW_MULT: f32 = 0.55;
pub const BOUNTY_STREAK: u32 = 3;
/// How much more damage you deal per tier your target's gear is above your weapon.
pub const UNDERDOG_BONUS_PER_TIER: f32 = 0.2;
pub const UNDERDOG_BONUS_MAX: f32 = 1.8;

/// How advanced someone's kit is: the better of their weapon and armor tiers.
pub fn gear_tier(vitals: &Vitals, loadout: &crate::protocol::Loadout) -> u8 {
    let armor = if vitals.armor_points > 0.0 {
        vitals.armor.tier()
    } else {
        0
    };
    loadout.weapon.stats().tier.max(armor)
}

/// Damage multiplier when a weaker weapon hits better-equipped gear: the stone age can
/// still bring down a power-armored railgunner, it just takes guts instead of luck.
pub fn underdog_mult(attacker_weapon_tier: u8, target_gear_tier: u8) -> f32 {
    let gap = target_gear_tier.saturating_sub(attacker_weapon_tier) as f32;
    (1.0 + UNDERDOG_BONUS_PER_TIER * gap).min(UNDERDOG_BONUS_MAX)
}

#[derive(Resource, Deref, Default)]
pub struct Arena(pub ArenaMap);

pub fn speed_mult(vitals: &Vitals) -> f32 {
    let emp = if vitals.emp_slow > 0 {
        EMP_SLOW_MULT
    } else {
        1.0
    };
    vitals.armor.speed_mult() * emp
}

/// Advance one player by one tick. Writes through `Mut` only when values change, so
/// prediction history and replication don't see spurious updates.
pub fn step_movement(
    mut pos: Mut<crate::protocol::Pos>,
    mut dash: Mut<Dash>,
    input: &PlayerInput,
    vitals: &Vitals,
    map: &ArenaMap,
) {
    if !vitals.alive {
        return;
    }
    let mut d = *dash;
    let move_dir = input.move_dir();

    if d.cooldown > 0 {
        d.cooldown -= 1;
    }
    if input.dash && d.cooldown == 0 && d.active == 0 {
        let dir = if move_dir != Vec2::ZERO {
            move_dir
        } else {
            input.aim.normalize_or(Vec2::X)
        };
        d.active = DASH_TICKS;
        d.cooldown = DASH_COOLDOWN;
        d.dir = dir;
    }

    let velocity = if d.active > 0 {
        d.active -= 1;
        d.dir * DASH_SPEED
    } else {
        move_dir * PLAYER_SPEED * speed_mult(vitals)
    };

    if d != *dash {
        *dash = d;
    }
    if velocity != Vec2::ZERO {
        let next = map.resolve_circle(pos.0 + velocity * DT, PLAYER_RADIUS);
        if next != pos.0 {
            pos.0 = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn underdog_bonus_scales_with_gap_and_caps() {
        assert_eq!(underdog_mult(2, 2), 1.0);
        assert_eq!(underdog_mult(3, 1), 1.0);
        assert!((underdog_mult(0, 2) - 1.4).abs() < 1e-6);
        assert_eq!(underdog_mult(0, 4), UNDERDOG_BONUS_MAX);
    }
}
