//! Which way a character faces, and the gloved fists painted onto held weapons. Characters
//! themselves are LPC sprites (see `crate::lpc`).

use super::*;

/// Which way a survivor sprite faces. `Side` faces right; flip it for left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    Down,
    Up,
    Side,
}

impl Facing {
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

/// A gloved fist wrapped around something two pixels thick, seen side-on: 3x4 pixels from
/// (x, y). Dark like the LPC characters' gloves; it gets its own dark edge where it overlaps
/// the weapon so it doesn't melt into it.
pub(super) fn paint_fist(c: &mut Canvas, x: i32, y: i32) {
    const GLOVE: Rgba = [0.24, 0.22, 0.21, 1.0];
    for (ex, ey) in (x - 1..=x + 3)
        .flat_map(|ex| [(ex, y - 1), (ex, y + 4)])
        .chain((y..=y + 3).flat_map(|ey| [(x - 1, ey), (x + 3, ey)]))
    {
        if c.get(ex, ey)[3] > 0.5 {
            c.put(ex, ey, OUTLINE);
        }
    }
    let dark = shade(GLOVE, 0.7);
    c.rect(x, y, x + 2, y + 3, GLOVE);
    c.rect(x, y, x + 1, y, shade(GLOVE, 1.4));
    c.rect(x, y + 3, x + 2, y + 3, dark);
    c.rect(x, y + 1, x, y + 3, dark);
    // Knuckle creases between the curled fingers.
    c.put(x + 2, y + 1, shade(dark, 0.88));
    c.put(x + 2, y + 2, shade(dark, 0.88));
    c.put(x + 1, y + 2, dark);
}
