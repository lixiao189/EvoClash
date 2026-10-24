//! Bullets, melee swings, explosions, bars and the reticle.

use super::*;

pub(super) fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub(super) fn paint_bolt() -> Canvas {
    let (w, h) = (48u32, 12u32);
    Canvas::filled(w, h, |x, y| {
        let u = (x as f32 + 0.5) / w as f32;
        let v = ((y as f32 + 0.5) / h as f32 - 0.5).abs() * 2.0;
        // Streak narrows toward the tail; the head is a round, hot bead.
        let half = 0.15 + 0.35 * u;
        let streak = (1.0 - v / half).max(0.0).powf(1.2) * u.powf(1.4);
        let head = Vec2::new((u - 0.86) * 3.4, v).length();
        let bead = (1.0 - head).max(0.0).powf(1.5);
        [1.0, 1.0, 1.0, streak.max(bead).min(1.0)]
    })
}

pub(super) fn paint_slash() -> Canvas {
    let n = 96u32;
    let half = n as f32 / 2.0;
    Canvas::filled(n, n, |x, y| {
        let p = Vec2::new(x as f32 + 0.5 - half, half - (y as f32 + 0.5)) / half;
        let (r, ang) = (p.length(), p.to_angle().abs());
        let reach = 0.95 - 0.1 * (ang / 1.0);
        let band = (1.0 - (r - reach).abs() / (0.12 + 0.28 * (1.0 - ang / 1.1).max(0.0))).max(0.0);
        let taper = smooth((1.1 - ang) / 0.7);
        let grain = 0.75 + 0.25 * noise(x as f32 / 5.0, y as f32 / 5.0, 19, 21);
        [1.0, 1.0, 1.0, (band * taper * grain).min(1.0)]
    })
}

pub(super) fn paint_shock() -> Canvas {
    let n = 128u32;
    let half = n as f32 / 2.0;
    Canvas::filled(n, n, |x, y| {
        let d = Vec2::new(x as f32 + 0.5 - half, y as f32 + 0.5 - half).length() / half;
        let ring = (1.0 - (d - 0.85).abs() / 0.15).max(0.0).powf(1.6);
        let haze = (1.0 - d).max(0.0) * 0.12;
        [1.0, 1.0, 1.0, (ring + haze).min(1.0)]
    })
}

pub(super) fn paint_blast() -> Canvas {
    let n = 64u32;
    let half = n as f32 / 2.0;
    Canvas::filled(n, n, |x, y| {
        let d = Vec2::new(x as f32 + 0.5 - half, y as f32 + 0.5 - half).length() / half;
        let wobble = noise(x as f32 / 8.0, y as f32 / 8.0, 8, 33) - 0.5;
        let fine = noise(x as f32 / 3.0, y as f32 / 3.0, 21, 34) - 0.5;
        let edge = 0.74 + wobble * 0.45 + fine * 0.07;
        let a = smooth((edge - d) / 0.28);
        // Darker, sootier toward the rim.
        let k = 1.0 - 0.4 * d - 0.12 * (fine + 0.5);
        [k, k, k, a]
    })
}

pub(super) fn paint_bar() -> Canvas {
    let (w, h) = (16u32, 6u32);
    Canvas::filled(w, h, |x, y| {
        let corner = (x == 0 || x == w as i32 - 1) && (y == 0 || y == h as i32 - 1);
        if corner {
            return [0.0; 4];
        }
        // Lit from above, darker along the bottom edge.
        let k = [0.8, 1.0, 1.0, 0.9, 0.78, 0.6][y as usize];
        [k, k, k, 1.0]
    })
}

pub(super) fn paint_reticle() -> Canvas {
    let mut c = Canvas::new(20, 20);
    let white = rgb(1.0, 1.0, 1.0);
    for (a, b) in [(2, 5), (14, 17)] {
        c.rect(9, a, 10, b, white);
        c.rect(a, 9, b, 10, white);
    }
    c
}
