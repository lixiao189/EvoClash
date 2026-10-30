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

/// Chance per think (1/16 s) that a fighting bot swaps the side it strafes on.
const STRAFE_FLIP: f32 = 0.03;
/// Thinks a strafe side is kept before it may flip again (~0.75 s). Without it a bot that
/// flips twice in a row is back where it started, which reads as shaking on the spot.
pub(super) const STRAFE_HOLD_THINKS: u16 = 12;

/// Distance bands for a fight. Held across thinks with separate enter and leave distances,
/// so a target hovering on a line does not reverse the bot's heading every think.
pub(super) const BACK: i8 = -1;
pub(super) const CIRCLE: i8 = 0;
pub(super) const CLOSE: i8 = 1;

pub(super) fn hold_band(
    held: i8,
    dist: f32,
    back_in: f32,
    back_out: f32,
    close_in: f32,
    close_out: f32,
) -> i8 {
    match held {
        BACK if dist > back_out => CIRCLE,
        CLOSE if dist < close_out => CIRCLE,
        BACK | CLOSE => held,
        _ if dist < back_in => BACK,
        _ if dist > close_in => CLOSE,
        _ => CIRCLE,
    }
}

/// Swap the strafe side at most once per hold.
pub(super) fn flip_strafe(strafe: &mut f32, hold: &mut u16, rng: &mut impl Rng) {
    if *hold > 0 {
        *hold -= 1;
        return;
    }
    if rng.random_bool(STRAFE_FLIP as f64) {
        *strafe = -*strafe;
        *hold = STRAFE_HOLD_THINKS;
    }
}

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
    (band, strafe_hold, heading): (&mut i8, &mut u16, &mut (i8, i8)),
    rng: &mut impl Rng,
) {
    let to_enemy = (enemy - at).normalize_or(Vec2::X);
    input.aim = aim_toward(to_enemy, aim_error, rng);
    let range = engage_range(weapon);
    flip_strafe(strafe, strafe_hold, rng);
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
        steer(input, dir, arena, obstacles, at, heading);
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
        *band = hold_band(
            *band,
            dist,
            range * 0.5,
            range * 0.575,
            range * 0.8,
            range * 0.72,
        );
        match *band {
            BACK => {
                let back = -to_enemy;
                if hazard(at + back * 56.0) || cornered {
                    to_enemy.perp()
                } else {
                    back
                }
            }
            CLOSE => to_enemy + strafe_dir * 0.2,
            _ => strafe_dir,
        }
    } else if let Some(min) = keep {
        *band = hold_band(*band, dist, min, min * 1.15, min + 80.0, min + 60.0);
        match *band {
            BACK => -to_enemy + strafe_dir * 0.3,
            CLOSE => to_enemy + strafe_dir * 0.2,
            _ => strafe_dir,
        }
    } else if weapon.melee().is_some() {
        let waypoint = next_waypoint(nav, arena, at, enemy, PLAYER_RADIUS);
        (waypoint - at).normalize_or_zero() + strafe_dir * 0.3
    } else {
        *band = hold_band(
            *band,
            dist,
            range * 0.35,
            range * 0.5,
            range * 0.85,
            range * 0.75,
        );
        match *band {
            BACK => -to_enemy + strafe_dir,
            CLOSE => {
                let waypoint = next_waypoint(nav, arena, at, enemy, PLAYER_RADIUS);
                (waypoint - at).normalize_or_zero() + strafe_dir * 0.3
            }
            _ => strafe_dir,
        }
    };
    steer(input, dir, arena, obstacles, at, heading);
    let shot = rng.random_bool(0.85);
    input.fire = locked_for > reaction
        && dist < range
        && (kite || shot)
        && !shot_hits_barrel(at, enemy, barrels);
    input.dash = dist < 260.0 && !kite && rng.random_bool(0.02);
    input.throw = emps > 0 && armor >= 50.0 && dist < 420.0 && rng.random_bool(0.1);
    if stealth_hurt {
        input.fire = false;
        input.throw = false;
        steer(input, -to_enemy, arena, obstacles, at, heading);
    }
    input.skill_b = skill_b_ready && dist < SKILL_RANGE;
}
