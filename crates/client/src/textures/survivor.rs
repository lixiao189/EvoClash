//! Player characters: bodies in four facings with walk frames, arms and fists.

use super::*;

/// Survivor canvas, in texture pixels. Feet touch the bottom; the sprite stands upright.
pub const SURVIVOR: UVec2 = UVec2::new(20, 30);
pub const WALK_FRAMES: usize = 4;

/// Which way a survivor sprite faces. `Side` faces right; flip it for left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    Down,
    Up,
    Side,
}

impl Facing {
    pub(super) const ALL: [Facing; 3] = [Facing::Down, Facing::Up, Facing::Side];

    pub(super) fn index(self) -> usize {
        match self {
            Facing::Down => 0,
            Facing::Up => 1,
            Facing::Side => 2,
        }
    }

    /// Pick a facing from an aim direction: mostly sideways reads as side-on.
    pub fn from_aim(aim: Vec2) -> (Facing, bool) {
        let flip = aim.x < 0.0;
        if aim.y.abs() < aim.x.abs() * 1.2 {
            (Facing::Side, flip)
        } else if aim.y > 0.0 {
            (Facing::Up, flip)
        } else {
            (Facing::Down, flip)
        }
    }
}

/// One animation frame: the jacket (tinted with the player's color) and everything else on top.
#[derive(Clone)]
pub struct SurvivorFrame {
    pub jacket: Handle<Image>,
    pub gear: Handle<Image>,
}

pub(super) const SKIN: Rgba = [0.93, 0.73, 0.57, 1.0];
pub(super) const SKIN_SHADE: Rgba = [0.76, 0.54, 0.42, 1.0];
pub(super) const HELMET: Rgba = [0.4, 0.42, 0.29, 1.0];
pub(super) const PANTS: Rgba = [0.42, 0.35, 0.25, 1.0];
pub(super) const BOOTS: Rgba = [0.25, 0.16, 0.1, 1.0];
pub(super) const PACK: Rgba = [0.42, 0.31, 0.19, 1.0];
pub(super) const ROLL: Rgba = [0.62, 0.52, 0.36, 1.0];
pub(super) const SCARF: Rgba = [0.75, 0.22, 0.17, 1.0];
pub(super) const LENS: Rgba = [0.35, 0.85, 0.9, 1.0];
pub(super) const STRAP: Rgba = [0.2, 0.14, 0.09, 1.0];

/// Paint one survivor frame. Frames 1 and 3 are mid-stride (opposite legs), 0 and 2 are passing.
pub(super) fn paint_survivor(facing: Facing, frame: usize) -> (Canvas, Canvas) {
    let (w, h) = (SURVIVOR.x, SURVIVOR.y);
    let mut jacket = Canvas::new(w, h);
    let mut gear = Canvas::new(w, h);
    let stride: i32 = match frame {
        1 => 1,
        3 => -1,
        _ => 0,
    };
    // The upper body dips a pixel on each stride.
    let bob = stride.abs();
    let cloth = rgb(0.93, 0.93, 0.93);

    // Legs and boots.
    match facing {
        Facing::Down | Facing::Up => {
            for (x, lift) in [(6, stride.max(0)), (11, (-stride).max(0))] {
                gear.rect(x, 21, x + 2, 25 - lift, PANTS);
                gear.rect(x + 2, 21, x + 2, 25 - lift, shade(PANTS, 0.78));
                gear.rect(x, 26 - lift, x + 2, 27 - lift, BOOTS);
            }
        }
        Facing::Side => {
            // Back leg darker; both swing around the hip at x = 9.
            for (dx, tone) in [(-stride * 2, 0.72), (stride * 2, 1.0)] {
                let x = 8 + dx;
                gear.rect(x, 21, x + 2, 25, shade(PANTS, tone));
                gear.rect(x, 26, x + 3, 27, shade(BOOTS, tone));
            }
        }
    }

    let y0 = bob; // shift for everything above the hips
    match facing {
        Facing::Down | Facing::Up => {
            // Torso with rounded shoulders, sleeves hanging at the sides.
            jacket.rect(5, 13 + y0, 14, 21, cloth);
            jacket.clear(5, 13 + y0);
            jacket.clear(14, 13 + y0);
            // Shoulders only: the arms reach forward to the weapon (see `paint_arm`).
            jacket.rect(4, 14 + y0, 4, 16 + y0, shade(cloth, 0.85));
            jacket.rect(15, 14 + y0, 15, 16 + y0, shade(cloth, 0.85));
            jacket.rect(13, 14 + y0, 14, 21, shade(cloth, 0.8));
            jacket.rect(6, 14 + y0, 6, 20, shade(cloth, 1.08));
            gear.rect(5, 21, 14, 21, STRAP);
        }
        Facing::Side => {
            jacket.rect(6, 13 + y0, 12, 21, cloth);
            jacket.rect(11, 13 + y0, 12, 21, shade(cloth, 0.8));
            jacket.clear(6, 13 + y0);
            // Shoulder seam; the arm itself reaches forward to the weapon.
            jacket.rect(9, 14 + y0, 11, 15 + y0, shade(cloth, 0.86));
            gear.rect(6, 21, 12, 21, STRAP);
        }
    }

    match facing {
        Facing::Down => {
            jacket.rect(7, 12 + y0, 12, 13 + y0, shade(cloth, 0.7));
            jacket.rect(9, 14 + y0, 10, 20, shade(cloth, 0.72));
            gear.rect(6, 13 + y0, 6, 20, STRAP);
            gear.rect(13, 13 + y0, 13, 20, STRAP);
            gear.rect(9, 21, 10, 21, rgb(0.78, 0.66, 0.3));
            paint_head_front(&mut gear, y0);
        }
        Facing::Up => {
            paint_head_back(&mut gear, y0);
            // Backpack with a blanket roll on top.
            gear.rect(6, 14 + y0, 13, 20, PACK);
            gear.rect(6, 14 + y0, 13, 15 + y0, shade(PACK, 0.78));
            gear.rect(12, 16 + y0, 13, 20, shade(PACK, 0.8));
            gear.rect(8, 17 + y0, 11, 18 + y0, shade(PACK, 1.2));
            gear.rect(5, 12 + y0, 14, 13 + y0, ROLL);
            gear.rect(5, 13 + y0, 14, 13 + y0, shade(ROLL, 0.75));
            gear.put(7, 12 + y0, STRAP);
            gear.put(12, 12 + y0, STRAP);
        }
        Facing::Side => {
            gear.rect(3, 13 + y0, 6, 20, PACK);
            gear.rect(3, 13 + y0, 3, 20, shade(PACK, 0.75));
            gear.rect(2, 12 + y0, 6, 13 + y0, ROLL);
            jacket.rect(7, 12 + y0, 11, 13 + y0, shade(cloth, 0.7));
            gear.line(7.0, 13.0 + y0 as f32, 7.0, 20.0, STRAP);
            paint_head_side(&mut gear, y0);
        }
    }
    jacket.bevel(9.5, 16.0, 7.0, 0.35);
    jacket.outline(OUTLINE);
    gear.outline(alpha(OUTLINE, 0.9));
    (jacket, gear)
}

pub(super) fn helmet_dome(c: &mut Canvas, x0: i32, x1: i32, y0: i32) {
    c.rect(x0 + 2, 1 + y0, x1 - 2, 1 + y0, HELMET);
    c.rect(x0 + 1, 2 + y0, x1 - 1, 2 + y0, HELMET);
    c.rect(x0, 3 + y0, x1, 5 + y0, HELMET);
    c.rect(x0 + 2, 2 + y0, x0 + 4, 2 + y0, shade(HELMET, 1.35));
    c.rect(x0 + 1, 3 + y0, x0 + 2, 3 + y0, shade(HELMET, 1.25));
}

pub(super) fn paint_head_front(c: &mut Canvas, y0: i32) {
    c.rect(6, 6 + y0, 13, 11 + y0, SKIN);
    c.rect(6, 6 + y0, 13, 6 + y0, SKIN_SHADE);
    c.rect(13, 7 + y0, 13, 11 + y0, SKIN_SHADE);
    helmet_dome(c, 5, 14, y0);
    c.rect(4, 6 + y0, 15, 6 + y0, shade(HELMET, 0.7));
    // Goggles pushed up on the helmet.
    c.rect(5, 4 + y0, 14, 4 + y0, STRAP);
    c.rect(7, 4 + y0, 8, 4 + y0, LENS);
    c.rect(11, 4 + y0, 12, 4 + y0, LENS);
    c.put(7, 4 + y0, rgb(0.9, 1.0, 1.0));
    c.put(11, 4 + y0, rgb(0.9, 1.0, 1.0));
    c.put(7, 8 + y0, OUTLINE);
    c.put(12, 8 + y0, OUTLINE);
    c.put(7, 7 + y0, SKIN_SHADE);
    c.put(12, 7 + y0, SKIN_SHADE);
    c.rect(9, 10 + y0, 10, 10 + y0, SKIN_SHADE);
    // Scarf over the chin.
    c.rect(6, 11 + y0, 13, 12 + y0, SCARF);
    c.rect(6, 12 + y0, 13, 12 + y0, shade(SCARF, 0.75));
}

pub(super) fn paint_head_back(c: &mut Canvas, y0: i32) {
    c.rect(8, 9 + y0, 11, 11 + y0, SKIN_SHADE);
    helmet_dome(c, 5, 14, y0);
    c.rect(5, 6 + y0, 14, 8 + y0, HELMET);
    c.rect(4, 8 + y0, 15, 8 + y0, shade(HELMET, 0.7));
    c.rect(5, 4 + y0, 14, 4 + y0, STRAP);
    c.rect(7, 10 + y0, 12, 11 + y0, SCARF);
    c.rect(11, 12 + y0, 12, 13 + y0, shade(SCARF, 0.8));
}

pub(super) fn paint_head_side(c: &mut Canvas, y0: i32) {
    c.rect(8, 6 + y0, 14, 11 + y0, SKIN);
    c.rect(8, 6 + y0, 10, 11 + y0, SKIN_SHADE);
    helmet_dome(c, 6, 14, y0);
    c.rect(6, 6 + y0, 9, 7 + y0, HELMET);
    // Brim sticks out forward.
    c.rect(6, 6 + y0, 16, 6 + y0, shade(HELMET, 0.7));
    c.rect(7, 4 + y0, 14, 4 + y0, STRAP);
    c.rect(12, 4 + y0, 13, 4 + y0, LENS);
    c.put(13, 4 + y0, rgb(0.9, 1.0, 1.0));
    c.put(12, 8 + y0, OUTLINE);
    c.put(15, 9 + y0, SKIN);
    c.rect(8, 11 + y0, 13, 12 + y0, SCARF);
    c.rect(6, 12 + y0, 7, 13 + y0, shade(SCARF, 0.8));
}

/// Forearm sleeve, pointing +X, cuff at the right where the fist takes over.
pub(super) fn paint_arm() -> Canvas {
    let mut c = Canvas::new(7, 5);
    let cloth = rgb(0.93, 0.93, 0.93);
    c.rect(1, 1, 5, 3, cloth);
    c.rect(1, 1, 4, 1, shade(cloth, 1.08));
    c.rect(1, 3, 5, 3, shade(cloth, 0.72));
    c.rect(5, 1, 5, 3, shade(cloth, 0.8));
    c.outline(OUTLINE);
    c
}

/// A fist wrapped around something two pixels thick, seen side-on: 3x4 pixels from (x, y).
/// It gets its own dark edge where it overlaps the weapon so it doesn't melt into it.
pub(super) fn paint_fist(c: &mut Canvas, x: i32, y: i32) {
    for (ex, ey) in (x - 1..=x + 3)
        .flat_map(|ex| [(ex, y - 1), (ex, y + 4)])
        .chain((y..=y + 3).flat_map(|ey| [(x - 1, ey), (x + 3, ey)]))
    {
        if c.get(ex, ey)[3] > 0.5 {
            c.put(ex, ey, OUTLINE);
        }
    }
    c.rect(x, y, x + 2, y + 3, SKIN);
    c.rect(x, y, x + 1, y, shade(SKIN, 1.08));
    c.rect(x, y + 3, x + 2, y + 3, SKIN_SHADE);
    c.rect(x, y + 1, x, y + 3, SKIN_SHADE);
    // Knuckle creases between the curled fingers.
    c.put(x + 2, y + 1, shade(SKIN_SHADE, 0.88));
    c.put(x + 2, y + 2, shade(SKIN_SHADE, 0.88));
    c.put(x + 1, y + 2, SKIN_SHADE);
}
