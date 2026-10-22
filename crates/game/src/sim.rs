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

#[derive(Resource, Deref)]
pub struct Arena(pub ArenaMap);

impl Default for Arena {
    fn default() -> Self {
        Self(ArenaMap::new())
    }
}

pub fn speed_mult(vitals: &Vitals) -> f32 {
    let emp = if vitals.emp_slow > 0 { EMP_SLOW_MULT } else { 1.0 };
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
