//! Keep interpolated bots, monsters and remote players moving through a dropped snapshot.
//!
//! Lightyear clamps when the presentation time runs past the newest confirmed sample, which
//! freezes the sprite until the next packet and then teleports it. This system re-samples
//! `ConfirmedHistory<Pos>` itself (it does not trust the live `Pos`: we overwrite that, and
//! lightyear skips the write when the interpolated value equals what we left there) and, while
//! the history has not been advanced to the interpolation instant, coasts along the last
//! velocity. `push_unchanged` moves the newest anchor up to the server's complete tick when an
//! NPC is actually standing still, so a stop is not a gap and is not extrapolated.
//!
//! A new packet pulls the authoritative sample backwards. The error is stored in `offset` and
//! decayed, so the picture eases onto the corrected position instead of popping.

use bevy::prelude::*;
use evoclash_game::protocol::{Monster, Player, Pos, Vitals};
use evoclash_game::sim::{DASH_SPEED, DT};
use lightyear::prelude::{ConfirmedHistory, Interpolated, InterpolationTimeline};

/// Coast at most this far past the newest sample (~94ms). Longer than that we are guessing.
const EXTRAP_MAX_TICKS: f32 = 6.0;
/// And at most this many pixels, so a fast body cannot slide across a doorway on one lost packet.
const EXTRAP_MAX_DIST: f32 = 24.0;
/// How fast a correction melts away. ~3× this (0.18s) and the error is down to ~5%.
const SMOOTH_TAU: f32 = 0.06;
/// A correction bigger than this is a respawn or teleport: take it, don't ease it.
const SNAP_DIST: f32 = 64.0;
/// Per tick. Dash is the fastest speed in `sim`; twice that is a teleport, not motion.
const MAX_EXTRAP_SPEED: f32 = 2.0 * DASH_SPEED * DT;

/// Where we drew this body last frame, and the correction still being eased out.
#[derive(Component, Clone, Copy)]
pub(super) struct Smoothing {
    shown: Vec2,
    offset: Vec2,
    extrapolating: bool,
    /// Tick of the newest anchor last frame. A new one mid-coast moves the coast's base.
    newest: u32,
}

/// One history anchor, oldest first. A `push_unchanged` anchor is included: its tick is what
/// says "the server has confirmed this position up to here".
pub(super) struct Anchor {
    tick: u32,
    pos: Vec2,
}

pub(super) fn smooth_remotes(
    time: Res<Time>,
    timeline: Res<InterpolationTimeline>,
    mut commands: Commands,
    mut anchors: Local<Vec<Anchor>>,
    mut bodies: Query<
        (
            Entity,
            &ConfirmedHistory<Pos>,
            &mut Pos,
            Option<&Vitals>,
            Option<&mut Smoothing>,
        ),
        (With<Interpolated>, Or<(With<Player>, With<Monster>)>),
    >,
) {
    if !timeline.is_synced() {
        return;
    }
    let interp_tick = timeline.now.tick().0;
    let overstep = timeline.now.overstep().to_f32();
    let dt = time.delta_secs();
    for (entity, history, mut pos, vitals, smoothing) in &mut bodies {
        collect_anchors(history, &mut anchors);
        let Some(newest) = anchors.last().map(|anchor| anchor.tick) else {
            continue;
        };
        let Some((raw, extrapolating)) = presentation_raw(
            &anchors,
            interp_tick,
            overstep,
            vitals.is_none_or(|vitals| vitals.alive),
        ) else {
            continue;
        };
        let shown = match smoothing {
            Some(mut state) => step_smoothing(&mut state, raw, extrapolating, newest, dt),
            None => {
                let mut state = Smoothing {
                    shown: raw,
                    offset: Vec2::ZERO,
                    extrapolating: false,
                    newest,
                };
                let shown = step_smoothing(&mut state, raw, extrapolating, newest, dt);
                commands.entity(entity).insert(state);
                shown
            }
        };
        if pos.0 != shown {
            pos.0 = shown;
        }
    }
}

fn collect_anchors(history: &ConfirmedHistory<Pos>, anchors: &mut Vec<Anchor>) {
    anchors.clear();
    // The iterator resolves `SameAsPrecedent`, so an unchanged anchor keeps its own tick.
    for (tick, pos) in history {
        anchors.push(Anchor {
            tick: tick.0,
            pos: pos.0,
        });
    }
}

/// `(raw, extrapolating)`. `None` when the history has no position yet.
fn presentation_raw(
    anchors: &[Anchor],
    interp_tick: u32,
    overstep: f32,
    alive: bool,
) -> Option<(Vec2, bool)> {
    let newest = anchors.last()?;
    // Ticks past the newest anchor. Subtract as integers first: an f32 tick count loses the
    // overstep after a few hours of server uptime. An anchor sitting on the current tick
    // (a stop advanced by `push_unchanged`) is not a gap.
    let behind = (i64::from(interp_tick) - i64::from(newest.tick)) as f32 + overstep;
    if behind <= 0.0 || !alive {
        return Some((interpolate(anchors, interp_tick, overstep), false));
    }
    Some((extrapolate(anchors, behind), true))
}

/// Linear blend matching `interpolation_fraction`: bracket by the integer tick, then add
/// the overstep. Past the newest anchor this holds the last position (what lightyear clamps to).
fn interpolate(anchors: &[Anchor], interp_tick: u32, overstep: f32) -> Vec2 {
    let mut start_i = None;
    for (i, anchor) in anchors.iter().enumerate() {
        if anchor.tick <= interp_tick {
            start_i = Some(i);
        } else {
            break;
        }
    }
    let Some(start_i) = start_i else {
        return anchors[0].pos;
    };
    let start = &anchors[start_i];
    let Some(end) = anchors.get(start_i + 1) else {
        return start.pos;
    };
    if end.tick <= start.tick {
        return start.pos;
    }
    let t = ((interp_tick - start.tick) as f32 + overstep) / (end.tick - start.tick) as f32;
    start.pos.lerp(end.pos, t)
}

/// `p1 + v * behind` (ticks past the newest anchor), capped.
/// A step faster than [`MAX_EXTRAP_SPEED`] is a teleport: hold `p1` instead of coasting.
fn extrapolate(anchors: &[Anchor], behind: f32) -> Vec2 {
    let Some(p1) = anchors.last() else {
        return Vec2::ZERO;
    };
    let Some(p0) = anchors.iter().rev().nth(1) else {
        return p1.pos;
    };
    let dt = p1.tick as f32 - p0.tick as f32;
    if dt <= 0.0 {
        return p1.pos;
    }
    let velocity = (p1.pos - p0.pos) / dt;
    if velocity.length() > MAX_EXTRAP_SPEED {
        return p1.pos;
    }
    let behind = behind.clamp(0.0, EXTRAP_MAX_TICKS);
    let mut delta = velocity * behind;
    let distance = delta.length();
    if distance > EXTRAP_MAX_DIST {
        delta = delta / distance * EXTRAP_MAX_DIST;
    }
    p1.pos + delta
}

/// Ease `offset` toward zero. When a coast ends, or a late packet moves its base, seed
/// `offset` with the jump so `shown` starts where it already was. A normal run of samples
/// only decays `offset`; it never grows.
fn step_smoothing(
    state: &mut Smoothing,
    raw: Vec2,
    extrapolating: bool,
    newest: u32,
    dt: f32,
) -> Vec2 {
    if state.extrapolating && (!extrapolating || newest != state.newest) {
        state.offset = state.shown - raw;
    }
    state.newest = newest;
    if state.offset.length_squared() > SNAP_DIST * SNAP_DIST {
        state.offset = Vec2::ZERO;
    }
    state.offset *= (-dt / SMOOTH_TAU).exp();
    state.extrapolating = extrapolating;
    state.shown = raw + state.offset;
    state.shown
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightyear::prelude::{ConfirmedHistory, Tick};

    fn anchor(tick: u32, x: f32) -> Anchor {
        Anchor {
            tick,
            pos: Vec2::new(x, 0.0),
        }
    }

    #[test]
    fn stopped_anchor_is_not_extrapolated() {
        // Was moving, then the server proved the position held through tick 20.
        let mut history = ConfirmedHistory::<Pos>::default();
        history.insert_present(Tick(10), Pos(Vec2::new(0.0, 0.0)));
        history.insert_present(Tick(12), Pos(Vec2::new(30.0, 0.0)));
        assert!(history.push_unchanged(Tick(20)).is_some());
        let mut anchors = Vec::new();
        collect_anchors(&history, &mut anchors);

        let (raw, extrapolating) = presentation_raw(&anchors, 20, 0.0, true).unwrap();
        assert!(!extrapolating);
        assert_eq!(raw, Vec2::new(30.0, 0.0));
    }

    #[test]
    fn gap_extrapolates_along_velocity_within_caps() {
        // 5 px/tick, two ticks past the last sample: 10 px, under both caps.
        let coasting = [anchor(10, 0.0), anchor(12, 10.0)];
        let (raw, extrapolating) = presentation_raw(&coasting, 14, 0.0, true).unwrap();
        assert!(extrapolating);
        assert!((raw.x - 20.0).abs() < 1e-4);

        // 20 px/tick for 10 ticks would be 200 px. Tick cap (6) then distance cap (24).
        let fast = [anchor(0, 0.0), anchor(1, 20.0)];
        let (capped, extrapolating) = presentation_raw(&fast, 11, 0.0, true).unwrap();
        assert!(extrapolating);
        assert!((capped.x - (20.0 + EXTRAP_MAX_DIST)).abs() < 1e-3);

        // Faster than 2× dash: a respawn, not motion.
        let teleported = [anchor(0, 0.0), anchor(1, 1_000.0)];
        let (held, extrapolating) = presentation_raw(&teleported, 4, 0.0, true).unwrap();
        assert!(extrapolating);
        assert_eq!(held.x, 1_000.0);

        // A corpse stays on the last confirmed point.
        let (dead, extrapolating) = presentation_raw(&coasting, 14, 0.0, false).unwrap();
        assert!(!extrapolating);
        assert_eq!(dead.x, 10.0);
    }

    #[test]
    fn returning_sample_eases_instead_of_popping() {
        let mut state = Smoothing {
            shown: Vec2::new(100.0, 0.0),
            offset: Vec2::ZERO,
            extrapolating: true,
            newest: 0,
        };
        let raw = Vec2::new(80.0, 0.0);
        let held = step_smoothing(&mut state, raw, false, 1, 0.0);
        assert!(
            (held.x - 100.0).abs() < 1e-4,
            "handoff must not pop, got {held}"
        );
        let settled = step_smoothing(&mut state, raw, false, 1, 3.0 * SMOOTH_TAU);
        assert!(
            (settled.x - 80.0).abs() < 1.5,
            "about 3τ should land on the new sample, got {settled}"
        );
    }

    #[test]
    fn late_packet_mid_coast_eases_instead_of_popping() {
        let mut state = Smoothing {
            shown: Vec2::new(100.0, 0.0),
            offset: Vec2::ZERO,
            extrapolating: true,
            newest: 10,
        };
        // Still starved, but a newer anchor moved the coast's base back by 8 px.
        let shown = step_smoothing(&mut state, Vec2::new(92.0, 0.0), true, 12, 0.0);
        assert!((shown.x - 100.0).abs() < 1e-4, "got {shown}");
    }

    #[test]
    fn overstep_survives_a_long_uptime() {
        // ~39 hours at 64 Hz: an f32 tick count rounds a 0.3-tick overstep to zero.
        let base = 9_000_000;
        let moving = [anchor(base - 2, 0.0), anchor(base, 10.0)];
        let (raw, extrapolating) = presentation_raw(&moving, base, 0.3, true).unwrap();
        assert!(extrapolating);
        assert!((raw.x - 11.5).abs() < 1e-3, "got {raw}");
    }

    #[test]
    fn large_correction_snaps() {
        let mut state = Smoothing {
            shown: Vec2::ZERO,
            offset: Vec2::ZERO,
            extrapolating: true,
            newest: 0,
        };
        let shown = step_smoothing(
            &mut state,
            Vec2::new(SNAP_DIST + 10.0, 0.0),
            false,
            1,
            0.016,
        );
        assert!((shown.x - (SNAP_DIST + 10.0)).abs() < 1e-3);
        assert_eq!(state.offset, Vec2::ZERO);
    }
}
