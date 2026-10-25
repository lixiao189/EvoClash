//! Monsters, their projectiles and splats, and the boss.

use super::*;

/// Side view, facing right. `frame` alternates which legs are planted.
pub(super) fn paint_roach(frame: i32) -> Canvas {
    let mut c = Canvas::new(26, 15);
    let shell = rgb(0.55, 0.3, 0.14);
    let leg = rgb(0.3, 0.17, 0.09);
    // Legs: alternate tripods lean forward and back.
    for (i, x) in [6.0, 11.0, 16.0].into_iter().enumerate() {
        let lean = if (i as i32 + frame) % 2 == 0 {
            2.0
        } else {
            -2.0
        };
        c.line(x, 9.0, x + lean * 0.5, 11.5, leg);
        c.line(x + lean * 0.5, 11.5, x + lean, 13.0, leg);
    }
    // Antennae sweep up and forward.
    c.line(21.0, 6.0, 23.0, 2.0, rgb(0.45, 0.3, 0.17));
    c.line(23.0, 2.0, 25.0, 1.0, rgb(0.45, 0.3, 0.17));
    c.line(20.0, 6.0, 21.0, 1.0, rgb(0.45, 0.3, 0.17));
    // Abdomen, wing case and head.
    c.ellipse(11.0, 7.5, 9.0, 3.8, shell);
    c.ellipse(10.0, 6.0, 7.5, 2.5, shade(shell, 1.25));
    c.line(4.0, 7.0, 17.0, 6.0, shade(shell, 0.7));
    c.ellipse(19.5, 8.0, 2.8, 2.6, shade(shell, 0.7));
    c.put(20, 7, rgb(1.0, 0.3, 0.2));
    // Radioactive sores.
    for (x, y) in [(7, 6), (12, 5), (14, 8)] {
        c.put(x, y, rgb(0.6, 1.0, 0.35));
    }
    c.outline(OUTLINE);
    c
}

/// Upright bulb on a stalk, roots splayed over the ground, maw facing the viewer.
pub(super) fn paint_pod() -> Canvas {
    let mut c = Canvas::new(32, 36);
    let flesh = rgb(0.55, 0.28, 0.46);
    let root = rgb(0.32, 0.17, 0.24);
    for (dx, len) in [(-9.0, 4.0), (-5.0, 5.0), (5.0, 5.0), (9.0, 4.0), (0.0, 3.0)] {
        c.line(16.0, 29.0, 16.0 + dx, 29.0 + len, root);
    }
    c.rect(13, 23, 18, 30, shade(flesh, 0.7));
    c.rect(13, 23, 14, 30, shade(flesh, 0.55));
    // Lumpy bulb.
    c.ellipse(16.0, 13.0, 12.5, 11.0, flesh);
    for i in 0..8 {
        let p = Vec2::from_angle(i as f32 / 8.0 * TAU) * Vec2::new(10.5, 9.0);
        c.disc(16.0 + p.x, 13.0 + p.y, 3.0, flesh);
    }
    c.bevel(16.0, 13.0, 12.0, 0.9);
    // Glowing veins.
    for i in 0..5 {
        let a = i as f32 / 5.0 * TAU - 0.4;
        let (d1, d2) = (Vec2::from_angle(a) * 5.0, Vec2::from_angle(a + 0.35) * 11.0);
        c.line(
            16.0 + d1.x,
            13.0 + d1.y,
            16.0 + d2.x,
            13.0 + d2.y,
            rgb(0.58, 0.92, 0.3),
        );
    }
    for (x, y) in [(8.0, 9.0), (24.0, 10.0), (11.0, 19.0), (22.0, 19.0)] {
        c.disc(x, y, 1.3, rgb(0.15, 0.06, 0.1));
        c.put(x as i32, y as i32, rgb(0.8, 1.0, 0.4));
    }
    // Maw.
    c.ellipse(16.0, 15.0, 5.0, 4.0, rgb(0.62, 0.95, 0.28));
    c.ellipse(16.0, 15.0, 3.8, 2.8, rgb(0.14, 0.04, 0.09));
    c.put(14, 14, rgb(0.9, 1.0, 0.6));
    c.outline(OUTLINE);
    c
}

pub(super) fn paint_spore() -> Canvas {
    let half = 8.0;
    Canvas::filled(16, 16, |x, y| {
        let d = Vec2::new(x as f32 + 0.5 - half, y as f32 + 0.5 - half).length() / half;
        let core = (1.0 - d * 1.6).max(0.0);
        let body: f32 = if d < 0.62 { 1.0 } else { 0.0 };
        let halo = (1.0 - d).max(0.0).powf(2.0) * 0.6;
        let a = body.max(halo);
        alpha(mix(rgb(0.5, 0.9, 0.2), rgb(0.95, 1.0, 0.7), core), a)
    })
}

pub(super) fn paint_splat() -> Canvas {
    let mut c = Canvas::new(28, 28);
    let goo = [0.32, 0.45, 0.12, 0.85];
    c.disc(14.0, 14.0, 6.0, goo);
    for i in 0..11 {
        let a = hash(i, 0, 70) * TAU;
        let r = 4.0 + hash(i, 1, 70) * 8.0;
        let p = Vec2::from_angle(a) * r;
        c.disc(14.0 + p.x, 14.0 + p.y, 1.0 + hash(i, 2, 70) * 2.2, goo);
    }
    c.disc(12.0, 12.0, 2.0, [0.55, 0.75, 0.25, 0.8]);
    c
}

/// Armored beetle seen from above, facing +X: dull plates behind, a bright front plate.
pub(super) fn paint_crawler() -> Canvas {
    let mut c = Canvas::new(40, 28);
    let shell = rgb(0.34, 0.33, 0.31);
    let leg = rgb(0.2, 0.19, 0.18);
    for i in 0..4 {
        let x = 10.0 + i as f32 * 6.0;
        c.line(x, 6.0, x + 3.0, 1.5, leg);
        c.line(x, 22.0, x + 3.0, 26.5, leg);
    }
    c.ellipse(19.0, 14.0, 15.0, 11.0, shell);
    c.ellipse(17.0, 12.0, 11.0, 6.5, shade(shell, 1.18));
    // Overlapping back plates.
    for x in [8.0, 13.0, 18.0, 23.0] {
        c.line(x, 5.0, x, 23.0, shade(shell, 0.55));
        c.line(x + 1.0, 6.0, x + 1.0, 22.0, shade(shell, 1.3));
    }
    c.line(5.0, 14.0, 26.0, 14.0, shade(shell, 0.6));
    // Rust streaks.
    for (x, y) in [(9, 8), (15, 18), (21, 9), (11, 19)] {
        c.rect(x, y, x + 1, y + 1, rgb(0.5, 0.3, 0.16));
    }
    // Lighter, thicker front plate so the armored side reads.
    let plate = rgb(0.74, 0.76, 0.82);
    c.ellipse(31.0, 14.0, 7.0, 9.5, shade(plate, 0.8));
    c.ellipse(31.5, 13.0, 5.5, 7.5, plate);
    c.line(33.0, 6.0, 33.0, 22.0, shade(plate, 0.7));
    for y in [7, 11, 17, 21] {
        c.put(30, y, shade(plate, 0.55));
    }
    c.put(35, 9, rgb(1.0, 0.3, 0.2));
    c.put(35, 19, rgb(1.0, 0.3, 0.2));
    c.outline(OUTLINE);
    c
}

/// Small quad-rotor seen from a slight angle; the rotors are translucent discs.
pub(super) fn paint_drone(frame: i32) -> Canvas {
    let mut c = Canvas::new(28, 20);
    let hull = rgb(0.38, 0.4, 0.46);
    for (x, y) in [(4.0, 5.0), (24.0, 5.0), (4.0, 13.0), (24.0, 13.0)] {
        c.line(14.0, 10.0, x, y, shade(hull, 0.6));
        c.disc(x, y, 1.6, shade(hull, 0.5));
    }
    c.ellipse(14.0, 10.0, 6.0, 4.5, hull);
    c.ellipse(13.0, 8.5, 4.0, 2.0, shade(hull, 1.3));
    // Camera eye.
    c.disc(17.0, 11.0, 2.0, rgb(0.15, 0.05, 0.05));
    c.disc(17.0, 11.0, 1.1, rgb(1.0, 0.25, 0.2));
    c.outline(OUTLINE);
    // Spinning blades: blur disc with a bright blade streak whose angle flips per frame.
    for (x, y) in [(4.0, 5.0), (24.0, 5.0), (4.0, 13.0), (24.0, 13.0)] {
        c.ellipse(x, y - 1.0, 4.2, 1.8, [0.8, 0.85, 0.9, 0.35]);
        let (dx, dy) = if frame == 0 { (3.8, 0.4) } else { (2.6, 1.3) };
        c.line(
            x - dx,
            y - 1.0 - dy,
            x + dx,
            y - 1.0 + dy,
            [0.9, 0.95, 1.0, 0.9],
        );
    }
    c
}

/// Solid arrowhead pointing +X.
pub(super) fn paint_arrow() -> Canvas {
    let mut c = Canvas::new(20, 20);
    for x in 0..16 {
        let half = ((16 - x) as f32 * 0.6).min(9.0);
        c.rect(x, 10 - half as i32, x, 9 + half as i32, rgb(1.0, 1.0, 1.0));
    }
    c.outline(OUTLINE);
    c
}

/// Opaque acid glob with a bright highlight, dark rim so it reads on any ground.
pub(super) fn paint_acid() -> Canvas {
    let mut c = Canvas::new(14, 14);
    c.disc(7.0, 7.0, 5.5, rgb(0.34, 0.62, 0.1));
    c.disc(7.0, 7.5, 4.2, rgb(0.55, 0.92, 0.2));
    c.disc(5.5, 5.5, 1.8, rgb(0.9, 1.0, 0.6));
    c.put(9, 9, rgb(0.35, 0.65, 0.12));
    c.outline(OUTLINE);
    c
}
