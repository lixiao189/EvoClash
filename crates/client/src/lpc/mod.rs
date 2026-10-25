//! Player sprites from the Liberated Pixel Cup character generator: how the baked sheets are
//! laid out, which frame a pose shows, and where a gun is gripped.
//!
//! The sheets in `assets/lpc/` are baked from the layers vendored in `assets/lpc-src/` by an
//! ignored test (`bake.rs`): `cargo test -p evoclash -- --ignored bake_lpc_sheets`. LPC art is
//! drawn at one texture pixel per world pixel, unlike the procedural sprites (`PIXEL`).

#[cfg(test)]
pub(crate) mod bake;
#[cfg(test)]
mod tests;

use bevy::math::Vec2;
use evoclash_game::items::ArmorKind;

use crate::textures::Facing;

/// A body frame, in pixels. Sheet rows face up, left, down and right.
pub const CELL: u32 = 64;
/// A club frame: the swing reaches past the body frame (at most ±46 px across).
pub const CLUB_CELL: u32 = 128;
/// Body sheet columns: standing then the 8-step stride, the 6-frame swing, the same stride
/// holding a gun at the hip, and 6 frames of falling over (the same on every row).
pub const WALK: u32 = 0;
pub const SLASH: u32 = 9;
pub const HOLD: u32 = 15;
pub const HURT: u32 = 24;
pub const COLUMNS: u32 = 30;
pub const ROWS: u32 = 4;
/// Club sheet columns: stride, swing, falling.
pub const CLUB_WALK: u32 = 0;
pub const CLUB_SLASH: u32 = 9;
pub const CLUB_HURT: u32 = 15;
pub const CLUB_COLUMNS: u32 = 21;
pub const STRIDE_FRAMES: usize = 8;
pub const SLASH_FRAMES: u32 = 6;
pub const HURT_FRAMES: u32 = 6;
/// How long the fall takes, and how long the body stays down before it disappears.
pub const HURT_SECS: f32 = 0.5;
pub const DEATH_SECS: f32 = 1.2;

/// Where the feet touch the ground, in pixels from the top of a frame.
pub const FEET: f32 = 62.0;
/// Top of the head above the feet, in world pixels.
pub const HEIGHT: f32 = 48.0;

/// One body sheet per armor; the armor is part of the outfit, not a separate layer.
pub const BODY_FILES: [&str; 4] = [
    "lpc/survivor.png",
    "lpc/survivor-scrap.png",
    "lpc/survivor-kevlar.png",
    "lpc/survivor-power.png",
];
/// Raiders: a fixed outfit with their red bandana baked in, laid out like the body sheets.
pub const RAIDER_FILE: &str = "lpc/raider.png";
/// The bandana alone, white, tinted with the player's color.
pub const TEAM_FILE: &str = "lpc/survivor-team.png";
/// The club in front of the body, and the part of it that swings behind.
pub const CLUB_FILE: &str = "lpc/club.png";
pub const CLUB_BEHIND_FILE: &str = "lpc/club-behind.png";

/// Creatures, 64 px frames, rows facing up, left, down and right like the characters.
/// Dogs are wolves (two coats): 5 walking then 5 running frames.
pub const WOLF_FILES: [&str; 2] = ["lpc/wolf.png", "lpc/wolf-grey.png"];
pub const WOLF_WALK: u32 = 0;
pub const WOLF_RUN: u32 = 5;
pub const WOLF_FRAMES: u32 = 5;
pub const WOLF_COLUMNS: u32 = 10;
/// Ghouls are zombies: standing, then the 8-step stride.
pub const GHOUL_FILE: &str = "lpc/ghoul.png";
pub const GHOUL_COLUMNS: u32 = 9;
/// Spitters are slimes, drawn twice as big: row 0 sits (6 frames), row 1 hops (8).
pub const SLIME_FILE: &str = "lpc/slime.png";
pub const SLIME_COLUMNS: u32 = 8;
pub const SLIME_FEET: f32 = 46.0;
/// The behemoth is a golem, drawn twice as big: 7 walking frames.
pub const GOLEM_FILE: &str = "lpc/golem.png";
pub const GOLEM_COLUMNS: u32 = 7;
/// How big spitters and the behemoth are drawn, in world pixels per texture pixel.
pub const BIG_SCALE: f32 = 2.0;
/// Props, drawn standing on their spot.
pub const CRATE_FILE: &str = "lpc/crate.png";
pub const BARREL_FILE: &str = "lpc/barrel.png";
pub const SAFE_FILE: &str = "lpc/safe.png";
pub const SAFE_OPEN_FILE: &str = "lpc/safe-open.png";

/// Map tiles, drawn at one texture pixel per world pixel and repeated over each area.
pub const GROUND_DIRT: &str = "lpc/ground-dirt.png";
pub const SIDEWALK: &str = "lpc/sidewalk.png";
/// A gravel patch whose edges fray into the dirt, stretched over an area with its edges kept
/// whole: `GROUND_EDGE` pixels on every side.
pub const GRAVEL_PATCH: &str = "lpc/ground-gravel-patch.png";
pub const GROUND_EDGE: f32 = 32.0;
pub const ROAD: &str = "lpc/road.png";
/// Wall faces for brick, metal and rock walls, repeated sideways; their heights in pixels.
pub const WALL_FACES: [&str; 3] = [
    "lpc/wall-brick.png",
    "lpc/wall-metal.png",
    "lpc/wall-rock.png",
];
pub const WALL_FACE_HEIGHTS: [u32; 3] = [64, 64, 40];
/// The top of a wall, seen from above.
pub const WALL_TOP: &str = "lpc/wall-top.png";
/// Flat roofs (dark planks, pale concrete), stretched over a building with their frame kept
/// whole: `ROOF_BORDER` pixels on every side.
pub const ROOF_TAR: &str = "lpc/roof-tar.png";
pub const ROOF_CONCRETE: &str = "lpc/roof-concrete.png";
pub const ROOF_BORDER: f32 = 16.0;

/// Which body sheet a combatant wears.
pub fn outfit(armor: ArmorKind, armor_points: f32) -> usize {
    if armor_points <= 0.0 {
        return 0;
    }
    match armor {
        ArmorKind::None => 0,
        ArmorKind::Scrap => 1,
        ArmorKind::Kevlar => 2,
        ArmorKind::PowerArmor => 3,
    }
}

/// Sheet row for a facing (`flip` means a side facing looks left).
pub fn row(facing: Facing, flip: bool) -> u32 {
    match (facing, flip) {
        (Facing::Up, _) => 0,
        (Facing::Side, true) => 1,
        (Facing::Down, _) => 2,
        (Facing::Side, false) => 3,
    }
}

/// Where a gun's rear grip sits, in world pixels from the feet: the rear hand of the hip-held
/// pose on each row (facing away, the hands are hidden in front of the body).
pub fn gun_grip(row: u32) -> Vec2 {
    let (x, y) = match row {
        0 => (38.4, 41.6),
        1 => (40.1, 43.2),
        2 => (25.6, 41.6),
        _ => (22.9, 43.2),
    };
    Vec2::new(x + 0.5 - CELL as f32 / 2.0, FEET - (y + 0.5))
}

/// What a combatant is doing, for picking a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Stride frame: 0 standing, 1..=8 walking. With a gun, the stride of the hip-held pose.
    Walk(usize),
    /// Seconds into a club swing that lasts `total`.
    Swing { since: f32, total: f32 },
    /// Seconds since dying.
    Fall(f32),
}

/// The frame to show: body sheet column, and club sheet column when a club is out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub body: u32,
    pub club: Option<u32>,
}

pub fn frame(action: Action, club: bool) -> Frame {
    let step = |since: f32, total: f32, frames: u32| {
        ((since.max(0.0) / total * frames as f32) as u32).min(frames - 1)
    };
    match action {
        Action::Fall(since) => {
            let k = step(since, HURT_SECS, HURT_FRAMES);
            Frame {
                body: HURT + k,
                club: club.then_some(CLUB_HURT + k),
            }
        }
        Action::Swing { since, total } => {
            let k = step(since, total, SLASH_FRAMES);
            Frame {
                body: SLASH + k,
                club: Some(CLUB_SLASH + k),
            }
        }
        Action::Walk(k) if club => Frame {
            body: WALK + k as u32,
            club: Some(CLUB_WALK + k as u32),
        },
        Action::Walk(k) => Frame {
            body: HOLD + k as u32,
            club: None,
        },
    }
}
