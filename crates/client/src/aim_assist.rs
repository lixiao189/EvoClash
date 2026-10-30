//! Aim assist for ranged weapons: the cursor's direction is bent a little toward the best target
//! inside a narrow cone, never locked onto it.
//!
//! [`assist`] is pure and knows nothing about entities; `input::buffer_input` collects the aim
//! points of the bodies this client can see and passes them in. The assisted direction is what
//! goes on the wire, so the server fires along it and the client's own tracers use it too.

use bevy::prelude::*;
use evoclash_game::items::WeaponKind;

/// Half-angle of the cone a target must be inside to be picked up, radians.
pub const HALF_ANGLE: f32 = 8.0 * std::f32::consts::PI / 180.0;

/// How far around the cone the aim is turned. The taper `1 - (angle / HALF_ANGLE)^2` is
/// multiplied by this, so a target straight on the cursor is hit exactly and the pull fades to nothing at
/// the edge of the cone: it guides, it does not snap.
pub const STRENGTH: f32 = 0.7;

/// Reach used for a gun that reports no usable projectile range, px.
pub const FALLBACK_RANGE: f32 = 700.0;

/// The assist reach of the weapon in hand: `None` for anything that is not a gun (melee, an
/// empty hotbar slot that falls back to the melee weapon, the throwable slot), so nothing is
/// assisted there.
pub fn reach(weapon: WeaponKind) -> Option<f32> {
    weapon.ranged().map(|gun| {
        let range = gun.range();
        if range.is_finite() && range > 0.0 {
            range
        } else {
            FALLBACK_RANGE
        }
    })
}

/// `cursor_dir` bent toward the closest-by-angle of `candidates` (aim points in world space)
/// that sits within `range` of `origin` and [`HALF_ANGLE`] of the cursor. Unchanged (normalized)
/// when nothing qualifies.
pub fn assist(
    cursor_dir: Vec2,
    origin: Vec2,
    candidates: impl IntoIterator<Item = Vec2>,
    range: f32,
) -> Vec2 {
    let aim = cursor_dir.normalize_or(Vec2::X);
    // (|angle|, signed angle) toward the best target so far.
    let mut best: Option<(f32, f32)> = None;
    for target in candidates {
        let offset = target - origin;
        let dist = offset.length();
        if dist <= f32::EPSILON || dist > range {
            continue;
        }
        let angle = aim.angle_to(offset);
        let abs = angle.abs();
        if abs > HALF_ANGLE {
            continue;
        }
        if best.is_none_or(|(closest, _)| abs < closest) {
            best = Some((abs, angle));
        }
    }
    let Some((abs, angle)) = best else {
        return aim;
    };
    let t = 1.0 - (abs / HALF_ANGLE).powi(2);
    Vec2::from_angle(aim.to_angle() + angle * t * STRENGTH).normalize_or(aim)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::PI;

    use super::*;

    const ORIGIN: Vec2 = Vec2::ZERO;
    const RANGE: f32 = 700.0;

    /// A point `deg` off `+X`, `dist` px away.
    fn at(deg: f32, dist: f32) -> Vec2 {
        Vec2::from_angle(deg * PI / 180.0) * dist
    }

    /// Signed angle from `+X` to `v`, in degrees.
    fn deg(v: Vec2) -> f32 {
        v.to_angle() * 180.0 / PI
    }

    #[test]
    fn outside_the_cone_is_unchanged() {
        let aim = assist(Vec2::X, ORIGIN, [at(10.0, 200.0), at(-20.0, 200.0)], RANGE);
        assert_eq!(aim, Vec2::X);
    }

    #[test]
    fn inside_the_cone_bends_toward_the_target() {
        // 4° off is halfway through the cone: the taper is 1 - 0.5^2 = 0.75.
        let aim = assist(Vec2::X, ORIGIN, [at(4.0, 300.0)], RANGE);
        assert!((deg(aim) - 4.0 * 0.75 * STRENGTH).abs() < 0.01, "{aim:?}");
        // Pointing straight at a target, the aim is already fully on it.
        let cursor = Vec2::from_angle(0.6);
        let on = assist(cursor, ORIGIN, [cursor * 300.0], RANGE);
        assert!((on - cursor).length() < 1e-6, "{on:?}");
        // Just inside the edge it barely moves: the pull has faded to almost nothing.
        let edge = assist(Vec2::X, ORIGIN, [at(7.9, 300.0)], RANGE);
        assert!(deg(edge) > 0.0, "{edge:?}");
        assert!(deg(edge) < 7.9 * 0.05, "{edge:?}");
    }

    #[test]
    fn the_smallest_angle_wins() {
        let aim = assist(Vec2::X, ORIGIN, [at(-6.0, 100.0), at(2.0, 400.0)], RANGE);
        assert!(deg(aim) > 0.0 && deg(aim) < 2.0, "{aim:?}");
    }

    #[test]
    fn out_of_range_is_not_assisted() {
        let aim = assist(Vec2::X, ORIGIN, [at(1.0, RANGE + 1.0)], RANGE);
        assert_eq!(aim, Vec2::X);
        let near = assist(Vec2::X, ORIGIN, [at(1.0, RANGE - 1.0)], RANGE);
        assert!(deg(near) > 0.0, "{near:?}");
    }

    #[test]
    fn only_guns_have_reach() {
        assert!(reach(WeaponKind::Stick).is_none());
        assert!(reach(WeaponKind::Rifle).is_some_and(|r| r > 0.0));
        assert!(reach(WeaponKind::Bow).is_some_and(|r| r > 0.0));
    }
}
