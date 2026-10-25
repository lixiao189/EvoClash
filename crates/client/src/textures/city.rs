//! City ground, floors, roofs, wall styles and props.

use super::*;

/// Short random walk of dark pixels, for cracks in tiles.
pub(super) fn crack(c: &mut Canvas, seed: u32, count: i32, len: i32, color: Rgba) {
    for i in 0..count {
        let (mut x, mut y) = (
            (hash(i, 0, seed) * c.w as f32) as i32,
            (hash(i, 1, seed) * c.h as f32) as i32,
        );
        for step in 0..len {
            c.put_wrap(x, y, color);
            let r = hash(i, step, seed + 1);
            if r < 0.45 {
                x += 1;
            } else if r < 0.7 {
                y += 1;
            } else if r < 0.85 {
                y -= 1;
            } else {
                x -= 1;
            }
        }
    }
}

/// Sickly soil around the craters: yellow-green, with glowing specks.
pub(super) fn paint_rad_ground() -> Canvas {
    let base = rgb(0.3, 0.32, 0.15);
    let mut c = Canvas::filled(64, 64, |x, y| {
        let (fx, fy) = (x as f32, y as f32);
        let broad = noise(fx / 16.0, fy / 16.0, 4, 110);
        let mid = noise(fx / 6.4, fy / 6.4, 10, 111);
        let k = 0.75 + broad * 0.25 + mid * 0.15 + (hash(x, y, 112) - 0.5) * 0.08;
        let scorch = (noise(fx / 10.67, fy / 10.67, 6, 113) - 0.55).max(0.0) * 1.6;
        mix(shade(base, k), rgb(0.16, 0.14, 0.1), scorch)
    });
    for i in 0..14 {
        let x = (hash(i, 0, 114) * 64.0) as i32;
        let y = (hash(i, 1, 114) * 64.0) as i32;
        c.put_wrap(x, y, rgb(0.6, 1.0, 0.3));
        c.put_wrap(x + 1, y, [0.6, 1.0, 0.3, 0.4]);
    }
    crack(&mut c, 115, 3, 16, [0.12, 0.12, 0.06, 0.6]);
    c
}

pub(super) fn paint_planks() -> Canvas {
    let wood = rgb(0.46, 0.31, 0.19);
    Canvas::filled(32, 32, |x, y| {
        let board = y / 8;
        let joint = (hash(board, 0, 120) * 32.0) as i32;
        let tone = 0.82 + hash(board, 1, 120) * 0.3;
        let grain = noise(x as f32 / 8.0, y as f32 * 1.0, 4, 121) * 0.16;
        let mut col = shade(wood, tone + grain);
        if y % 8 == 0 {
            col = shade(wood, 0.45);
        } else if y % 8 == 1 {
            col = shade(col, 1.12);
        }
        if x == joint {
            col = shade(wood, 0.5);
        }
        if (x == (joint + 2) % 32 || x == (joint + 30) % 32) && y % 8 == 4 {
            col = rgb(0.25, 0.22, 0.2);
        }
        col
    })
}

pub(super) fn paint_tiles() -> Canvas {
    let a = rgb(0.7, 0.7, 0.64);
    let b = rgb(0.52, 0.6, 0.58);
    let mut c = Canvas::filled(32, 32, |x, y| {
        if x % 8 == 0 || y % 8 == 0 {
            return rgb(0.28, 0.27, 0.25);
        }
        let (tx, ty) = (x / 8, y / 8);
        let col = if (tx + ty) % 2 == 0 { a } else { b };
        let grime = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 130) * 0.2;
        let mut col = shade(col, 0.82 + grime + (hash(tx, ty, 131) - 0.5) * 0.1);
        if x % 8 == 1 || y % 8 == 1 {
            col = shade(col, 1.1);
        }
        col
    });
    crack(&mut c, 132, 1, 8, [0.2, 0.2, 0.18, 0.8]);
    c
}

pub(super) fn paint_floor_concrete() -> Canvas {
    let slab = rgb(0.4, 0.4, 0.38);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let n = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 140);
        let col = shade(slab, 0.86 + n * 0.2 + (hash(x, y, 141) - 0.5) * 0.04);
        if x == 0 || y == 0 {
            shade(slab, 0.7)
        } else {
            col
        }
    });
    crack(&mut c, 142, 1, 10, [0.22, 0.22, 0.2, 0.7]);
    c
}

/// Khaki sandbags stacked in staggered rows (front face) or seen from above (cap).
pub(super) fn paint_sandbags(top: bool) -> Canvas {
    let bag = rgb(0.62, 0.55, 0.38);
    let row_h = if top { 16 } else { 8 };
    Canvas::filled(32, 32, |x, y| {
        let row = y / row_h;
        let lx = (x + if row % 2 == 0 { 0 } else { 8 }).rem_euclid(32) % 16;
        let ly = y % row_h;
        let dx = (lx as f32 + 0.5 - 8.0) / 8.0;
        let dy = (ly as f32 + 0.5 - row_h as f32 / 2.0) / (row_h as f32 / 2.0);
        let d = dx * dx + dy * dy;
        if d > 1.05 {
            return rgb(0.2, 0.17, 0.12);
        }
        let light = 1.15 - 0.35 * (ly as f32 / row_h as f32) - 0.15 * dx.abs();
        let mut col = shade(bag, light + (hash(x, y, 180) - 0.5) * 0.08);
        if lx == 8 && ly % 2 == 0 {
            col = shade(bag, 0.7);
        }
        col
    })
}

/// A wooden supply crate in three-quarter view: lid on top, braced front.
pub(super) fn paint_crate() -> Canvas {
    let mut c = Canvas::new(20, 22);
    let wood = rgb(0.6, 0.42, 0.24);
    // Lid.
    c.rect(1, 1, 18, 6, shade(wood, 1.2));
    for x in [6, 12] {
        c.rect(x, 1, x, 6, shade(wood, 0.8));
    }
    // Front planks.
    c.rect(1, 7, 18, 20, wood);
    for y in [11, 16] {
        c.rect(1, y, 18, y, shade(wood, 0.65));
    }
    // Frame and cross brace.
    let frame = shade(wood, 0.55);
    c.rect(1, 7, 18, 7, frame);
    c.rect(1, 20, 18, 20, frame);
    c.rect(1, 7, 2, 20, frame);
    c.rect(17, 7, 18, 20, frame);
    c.line(3.0, 19.0, 16.0, 8.0, shade(wood, 0.75));
    c.line(3.0, 8.0, 16.0, 19.0, shade(wood, 0.75));
    // Stenciled supply mark.
    c.rect(8, 12, 11, 15, rgb(0.85, 0.75, 0.35));
    c.rect(9, 13, 10, 14, wood);
    c.outline(OUTLINE);
    c
}

/// A rusty red fuel drum with a hazard band.
pub(super) fn paint_barrel() -> Canvas {
    let mut c = Canvas::new(16, 22);
    let red = rgb(0.68, 0.17, 0.12);
    for y in 3..21 {
        for x in 1..15 {
            let fx = (x as f32 + 0.5 - 6.0) / 7.0;
            let light = 1.2 - fx * fx * 0.6 - fx * 0.15;
            let rust = (noise(x as f32 / 3.0, y as f32 / 3.0, 8, 190) - 0.6).max(0.0) * 2.5;
            c.put(
                x,
                y,
                mix(shade(red, light), rgb(0.4, 0.22, 0.12), rust.min(0.8)),
            );
        }
    }
    for y in [7, 16] {
        for x in 1..15 {
            let p = c.get(x, y);
            c.put(x, y, shade(p, 0.6));
        }
    }
    // Hazard band.
    for x in 1..15 {
        let stripe = (x / 2) % 2 == 0;
        let col = if stripe {
            rgb(0.95, 0.78, 0.15)
        } else {
            rgb(0.12, 0.1, 0.08)
        };
        c.rect(x, 11, x, 12, col);
    }
    // Lid.
    c.ellipse(8.0, 3.5, 7.0, 2.5, shade(red, 0.75));
    c.ellipse(8.0, 3.3, 5.5, 1.6, shade(red, 1.1));
    c.put(11, 3, rgb(0.3, 0.3, 0.3));
    c.outline(OUTLINE);
    c
}

/// A squat steel safe; `open` shows the door swung wide and the shelf bare.
pub(super) fn paint_safe(open: bool) -> Canvas {
    let mut c = Canvas::new(22, 24);
    let steel = rgb(0.3, 0.35, 0.33);
    c.rect(1, 1, 20, 5, shade(steel, 1.3));
    c.rect(1, 6, 20, 21, steel);
    c.rect(2, 22, 4, 23, shade(steel, 0.6));
    c.rect(17, 22, 19, 23, shade(steel, 0.6));
    if open {
        c.rect(4, 8, 17, 19, rgb(0.06, 0.06, 0.07));
        c.rect(4, 14, 17, 14, rgb(0.25, 0.25, 0.25));
        // The door, swung toward the viewer on its left hinge.
        c.rect(0, 7, 3, 21, shade(steel, 1.15));
        c.rect(1, 12, 2, 14, rgb(0.75, 0.7, 0.5));
    } else {
        c.rect(3, 8, 18, 19, shade(steel, 0.85));
        c.rect(4, 9, 17, 18, shade(steel, 1.05));
        c.disc(13.5, 13.5, 3.0, rgb(0.7, 0.68, 0.6));
        c.disc(13.5, 13.5, 1.6, rgb(0.35, 0.34, 0.3));
        c.put(13, 10, rgb(0.9, 0.2, 0.15));
        c.rect(6, 12, 7, 15, rgb(0.75, 0.7, 0.5));
        c.rect(3, 9, 3, 10, rgb(0.15, 0.15, 0.15));
        c.rect(3, 17, 3, 18, rgb(0.15, 0.15, 0.15));
    }
    c.outline(OUTLINE);
    c
}
