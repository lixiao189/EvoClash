//! How a bot moves once it has decided to fight.

#[cfg(test)]
use evoclash_game::protocol::CRAWLER_SHELL_COS;
use evoclash_game::protocol::MonsterKind;
use rand::Rng;

use super::scavenge::shot_hits_barrel;
use super::think::{aim_toward, engage_range, steer};
use super::*;

/// Cosine of 100°: past this the bot is on the crawler's side or rear, where the shell is thin.
const REAR_COS: f32 = -0.17364818;

pub(super) fn on_rear(facing: f32, crawler_to_bot: Vec2) -> bool {
    let dir = crawler_to_bot.normalize_or_zero();
    dir != Vec2::ZERO && dir.dot(Vec2::from_angle(facing)) < REAR_COS
}

/// True when a hit from `crawler_to_bot` lands on the thick front.
#[cfg(test)]
pub(super) fn on_shell(facing: f32, crawler_to_bot: Vec2) -> bool {
    let dir = crawler_to_bot.normalize_or_zero();
    dir != Vec2::ZERO && dir.dot(Vec2::from_angle(facing)) >= CRAWLER_SHELL_COS
}

#[allow(clippy::too_many_arguments)]
pub(super) fn press(
    input: &mut PlayerInput,
    weapon: WeaponKind,
    stick: bool,
    at: Vec2,
    enemy: Vec2,
    dist: f32,
    kind: Option<MonsterKind>,
    facing: Option<f32>,
    cornered: bool,
    locked_for: u16,
    reaction: u16,
    aim_error: f32,
    strafe: &mut f32,
    armor: f32,
    emps: u8,
    stealth_hurt: bool,
    skill_b_ready: bool,
    barrels: &[Vec2],
    hazard: impl Fn(Vec2) -> bool,
    nav: &evoclash_game::nav::NavGrid,
    arena: &Arena,
    obstacles: &[(Vec2, f32)],
    rng: &mut impl Rng,
) {
    let to_enemy = (enemy - at).normalize_or(Vec2::X);
    input.aim = aim_toward(to_enemy, aim_error, rng);
    let range = engage_range(weapon);
    if rng.random_bool(0.03) {
        *strafe = -*strafe;
    }
    // Bite reach is `radius + PLAYER_RADIUS + 4` in monsters/attacks.rs (30+22+4).
    const CRAWLER_REACH: f32 = 56.0;
    const CRAWLER_ORBIT: f32 = CRAWLER_REACH + 60.0;
    if kind == Some(MonsterKind::Crawler)
        && !cornered
        && facing.is_some_and(|face| !on_rear(face, at - enemy))
    {
        let face = facing.unwrap_or(0.0);
        let to_me = at - enemy;
        let sign = Vec2::from_angle(face).perp().dot(to_me).signum();
        let sign = if sign == 0.0 { 1.0 } else { sign };
        let mut dir = to_me.normalize_or_zero().perp() * sign;
        if dist < CRAWLER_ORBIT {
            dir -= to_me.normalize_or_zero();
        }
        steer(input, dir, arena, obstacles, at);
        let shot = rng.random_bool(0.85);
        input.fire = !stick && locked_for > reaction && dist < range && shot;
        return;
    }
    let kite = !stick && matches!(kind, Some(MonsterKind::Roach | MonsterKind::Dog));
    let keep = if stick {
        None
    } else {
        match kind {
            Some(MonsterKind::Ghoul) => Some(160.0),
            Some(MonsterKind::Spitter) => Some(220.0),
            _ => None,
        }
    };
    let strafe_dir = to_enemy.perp() * *strafe;
    let dir = if kite {
        let lo = range * 0.5;
        let hi = range * 0.8;
        if dist < lo {
            let back = -to_enemy;
            if hazard(at + back * 56.0) || cornered {
                to_enemy.perp()
            } else {
                back
            }
        } else if dist > hi {
            to_enemy + strafe_dir * 0.2
        } else {
            strafe_dir
        }
    } else if let Some(min) = keep {
        if dist < min {
            -to_enemy + strafe_dir * 0.3
        } else if dist > min + 80.0 {
            to_enemy + strafe_dir * 0.2
        } else {
            strafe_dir
        }
    } else if weapon.melee().is_some() || dist > range * 0.85 {
        let waypoint = next_waypoint(nav, arena, at, enemy, PLAYER_RADIUS);
        (waypoint - at).normalize_or_zero() + strafe_dir * 0.3
    } else if dist < range * 0.35 {
        -to_enemy + strafe_dir
    } else {
        strafe_dir
    };
    steer(input, dir, arena, obstacles, at);
    let shot = rng.random_bool(0.85);
    input.fire = locked_for > reaction
        && dist < range
        && (kite || shot)
        && !shot_hits_barrel(at, enemy, barrels);
    input.dash = dist < 260.0 && !kite && rng.random_bool(0.04);
    input.throw = emps > 0 && armor >= 50.0 && dist < 420.0 && rng.random_bool(0.1);
    if stealth_hurt {
        input.fire = false;
        input.throw = false;
        steer(input, -to_enemy, arena, obstacles, at);
    }
    input.skill_b = skill_b_ready && dist < SKILL_RANGE;
}
