//! Ground, vault and wall tiles, clutter and decals.

use super::*;

pub(super) fn paint_ground() -> Canvas {
    let base = rgb(0.36, 0.22, 0.16);
    let mut c = Canvas::filled(64, 64, |x, y| {
        let (fx, fy) = (x as f32, y as f32);
        let broad = noise(fx / 16.0, fy / 16.0, 4, 1);
        let mid = noise(fx / 6.4, fy / 6.4, 10, 2);
        let grain = hash(x, y, 3) - 0.5;
        let k = 0.78 + broad * 0.22 + mid * 0.14 + grain * 0.07;
        // Patches of ashy gray dust.
        let ash = (noise(fx / 10.67, fy / 10.67, 6, 4) - 0.55).max(0.0) * 1.6;
        // Patches of violet ash, like the rust-red badlands they settle on.
        mix(shade(base, k), rgb(0.27, 0.2, 0.25), ash)
    });
    // Pebbles.
    for i in 0..26 {
        let x = (hash(i, 0, 10) * 64.0) as i32;
        let y = (hash(i, 1, 10) * 64.0) as i32;
        let light = hash(i, 2, 10) > 0.5;
        let col = if light {
            rgb(0.5, 0.34, 0.25)
        } else {
            rgb(0.2, 0.12, 0.1)
        };
        c.put_wrap(x, y, col);
        if hash(i, 3, 10) > 0.6 {
            c.put_wrap(x + 1, y, col);
            c.put_wrap(x, y + 1, shade(col, 0.7));
        }
    }
    // Hairline cracks: short random walks.
    for i in 0..2 {
        let (mut x, mut y) = (
            (hash(i, 0, 20) * 64.0) as i32,
            (hash(i, 1, 20) * 64.0) as i32,
        );
        for step in 0..18 {
            c.put_wrap(x, y, [0.16, 0.09, 0.07, 0.45]);
            let r = hash(i, step, 21);
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
    c
}

pub(super) fn paint_vault_floor() -> Canvas {
    let plate = rgb(0.25, 0.25, 0.24);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let n = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 30);
        let mut col = shade(plate, 0.9 + n * 0.18 + (hash(x, y, 31) - 0.5) * 0.05);
        // Rust bloom.
        let rust = (noise(x as f32 / 5.33, y as f32 / 5.33, 6, 32) - 0.6).max(0.0) * 2.2;
        col = mix(col, rgb(0.42, 0.24, 0.12), rust);
        if x == 0 || y == 0 {
            col = shade(plate, 0.45);
        } else if x == 1 || y == 1 {
            col = shade(col, 1.25);
        }
        col
    });
    for (x, y) in [(4, 4), (27, 4), (4, 27), (27, 27)] {
        c.put(x, y, rgb(0.42, 0.42, 0.4));
        c.put(x + 1, y + 1, rgb(0.12, 0.12, 0.12));
    }
    // Grating slots in the middle of each plate.
    for slot in 0..4 {
        let y = 10 + slot * 4;
        c.rect(9, y, 22, y + 1, rgb(0.08, 0.08, 0.08));
        c.rect(9, y + 2, 22, y + 2, shade(plate, 1.2));
    }
    c
}

pub(super) fn paint_trefoil() -> Canvas {
    let mut c = Canvas::new(48, 48);
    let ink = rgb(0.95, 0.75, 0.12);
    for y in 0..48 {
        for x in 0..48 {
            let d = Vec2::new(x as f32 + 0.5 - 24.0, y as f32 + 0.5 - 24.0);
            let r = d.length();
            let a = (d.to_angle() + PI / 2.0).rem_euclid(TAU / 3.0);
            let blade = (7.0..21.0).contains(&r) && a < TAU / 6.0;
            if r < 4.5 || blade {
                c.put(x, y, ink);
            } else if (22.0..23.5).contains(&r) {
                c.put(x, y, alpha(ink, 0.8));
            }
        }
    }
    c
}

pub(super) fn paint_wall() -> Canvas {
    let concrete = rgb(0.42, 0.39, 0.35);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let row = y / 8;
        let offset = if row % 2 == 0 { 0 } else { 8 };
        let bx = (x + offset).rem_euclid(32) / 16;
        let (lx, ly) = ((x + offset).rem_euclid(16), y % 8);
        if lx == 0 || ly == 0 {
            return rgb(0.16, 0.15, 0.14);
        }
        let tone = 0.82 + hash(bx, row, 40) * 0.26;
        let grain = (hash(x, y, 41) - 0.5) * 0.08;
        let mut col = shade(concrete, tone + grain);
        if ly == 1 || lx == 1 {
            col = shade(col, 1.18);
        } else if ly == 7 || lx == 15 {
            col = shade(col, 0.72);
        }
        // Soot and moss creeping up.
        let grime = (noise(x as f32 / 8.0, y as f32 / 8.0, 4, 42) - 0.55).max(0.0) * 1.8;
        mix(col, rgb(0.2, 0.22, 0.14), grime)
    });
    for i in 0..3 {
        let (mut x, mut y) = (
            (hash(i, 0, 43) * 32.0) as i32,
            (hash(i, 1, 43) * 32.0) as i32,
        );
        for step in 0..6 {
            c.put_wrap(x, y, rgb(0.13, 0.12, 0.11));
            x += 1;
            y += if hash(i, step, 44) > 0.5 { 1 } else { 0 };
        }
    }
    c
}

pub(super) fn paint_wall_top() -> Canvas {
    let slab = rgb(0.6, 0.55, 0.48);
    let mut c = Canvas::filled(32, 32, |x, y| {
        let n = noise(x as f32 / 8.0, y as f32 / 8.0, 4, 45);
        let grain = (hash(x, y, 46) - 0.5) * 0.06;
        let mut col = shade(slab, 0.88 + n * 0.2 + grain);
        // Seams between cast slabs.
        if x % 16 == 0 || y % 16 == 0 {
            col = shade(slab, 0.68);
        }
        let moss = (noise(x as f32 / 5.33, y as f32 / 5.33, 6, 47) - 0.62).max(0.0) * 2.4;
        mix(col, rgb(0.42, 0.5, 0.3), moss)
    });
    for i in 0..3 {
        let (mut x, mut y) = (
            (hash(i, 0, 48) * 32.0) as i32,
            (hash(i, 1, 48) * 32.0) as i32,
        );
        for step in 0..7 {
            c.put_wrap(x, y, rgb(0.3, 0.27, 0.24));
            x += 1;
            y += if hash(i, step, 49) > 0.5 { 1 } else { -1 };
        }
    }
    c
}

/// Knee-high clutter. Bottom row is where it touches the ground.
pub(super) fn paint_decor(variant: i32) -> Canvas {
    match variant {
        // Dead grass tuft.
        0 => {
            let mut c = Canvas::new(12, 10);
            let straw = rgb(0.72, 0.58, 0.32);
            for (i, x) in [2.0, 4.0, 5.5, 7.0, 9.0].into_iter().enumerate() {
                let lean = (i as f32 - 2.0) * 0.8;
                c.line(
                    x,
                    9.0,
                    x + lean,
                    2.0 + (i % 2) as f32 * 2.0,
                    shade(straw, 0.8 + i as f32 * 0.08),
                );
            }
            c
        }
        // Scraggly dead bush.
        1 => {
            let mut c = Canvas::new(16, 13);
            let twig = rgb(0.38, 0.26, 0.2);
            c.line(8.0, 12.0, 8.0, 6.0, twig);
            c.line(8.0, 8.0, 3.0, 3.0, twig);
            c.line(8.0, 7.0, 13.0, 2.0, twig);
            c.line(8.0, 6.0, 7.0, 1.0, twig);
            c.line(5.0, 5.0, 3.0, 6.0, twig);
            c.line(11.0, 4.0, 14.0, 5.0, twig);
            for (x, y) in [(3, 2), (13, 1), (7, 0), (14, 5), (2, 6)] {
                c.put(x, y, rgb(0.55, 0.62, 0.3));
            }
            c.outline(alpha(OUTLINE, 0.7));
            c
        }
        // Boulder.
        2 => {
            let mut c = Canvas::new(14, 10);
            c.ellipse(7.0, 6.0, 6.0, 4.0, rgb(0.5, 0.42, 0.38));
            c.ellipse(9.0, 4.5, 3.0, 2.5, rgb(0.5, 0.42, 0.38));
            c.bevel(7.0, 5.0, 6.0, 1.0);
            c.outline(OUTLINE);
            c
        }
        // Bleached skull.
        3 => {
            let mut c = Canvas::new(10, 9);
            let bone = rgb(0.9, 0.86, 0.76);
            c.disc(5.0, 4.0, 3.6, bone);
            c.rect(3, 6, 7, 7, bone);
            c.rect(3, 4, 3, 4, OUTLINE);
            c.rect(6, 4, 6, 4, OUTLINE);
            c.put(4, 7, shade(bone, 0.6));
            c.put(6, 7, shade(bone, 0.6));
            c.outline(OUTLINE);
            c
        }
        // Rusted tire half-buried.
        _ => {
            let mut c = Canvas::new(14, 9);
            c.ellipse(7.0, 6.0, 6.0, 4.5, rgb(0.15, 0.13, 0.13));
            c.ellipse(7.0, 6.0, 3.0, 2.0, [0.0; 4]);
            for y in 0..9 {
                for x in 0..14 {
                    let d = Vec2::new((x as f32 + 0.5 - 7.0) / 3.0, (y as f32 + 0.5 - 6.0) / 2.0);
                    if d.length_squared() <= 1.0 {
                        c.clear(x, y);
                    }
                }
            }
            c.rect(0, 7, 13, 8, rgb(0.36, 0.22, 0.16));
            c.outline(alpha(OUTLINE, 0.8));
            c
        }
    }
}

/// Rubble piles, scorch marks and oil stains scattered over the floor.
pub(super) fn paint_decal(variant: i32) -> Canvas {
    let mut c = Canvas::new(24, 24);
    let seed = 50 + variant as u32;
    match variant % 3 {
        0 => {
            for i in 0..9 {
                let x = 5.0 + hash(i, 0, seed) * 14.0;
                let y = 5.0 + hash(i, 1, seed) * 14.0;
                let r = 1.2 + hash(i, 2, seed) * 2.4;
                let tone = 0.3 + hash(i, 3, seed) * 0.18;
                c.disc(x, y, r, rgb(tone, tone * 0.95, tone * 0.88));
                c.put(
                    x as i32,
                    y as i32 - 1,
                    rgb(tone * 1.4, tone * 1.35, tone * 1.25),
                );
            }
            c.outline(alpha(OUTLINE, 0.6));
        }
        1 => {
            for y in 0..24 {
                for x in 0..24 {
                    let d = Vec2::new(x as f32 - 11.5, y as f32 - 11.5).length() / 11.0;
                    let n = noise(x as f32 / 4.0, y as f32 / 4.0, 6, seed);
                    let a = ((1.0 - d) * 1.4 - n * 0.6).clamp(0.0, 0.75);
                    c.put(x, y, [0.04, 0.035, 0.03, a]);
                }
            }
        }
        _ => {
            for i in 0..5 {
                let x = 7.0 + hash(i, 0, seed) * 10.0;
                let y = 7.0 + hash(i, 1, seed) * 10.0;
                let (rx, ry) = (3.0 + hash(i, 2, seed) * 4.0, 2.0 + hash(i, 3, seed) * 3.0);
                c.ellipse(x, y, rx, ry, [0.07, 0.07, 0.09, 0.55]);
            }
            c.put(10, 9, [0.3, 0.25, 0.4, 0.5]);
            c.put(14, 13, [0.2, 0.35, 0.35, 0.5]);
        }
    }
    c
}

/// Radial falloff, used for drop shadows and loot glows.
pub(super) fn paint_soft_disc(size: u32, color: [f32; 3], falloff: f32) -> Canvas {
    let half = size as f32 / 2.0;
    Canvas::filled(size, size, |x, y| {
        let d = Vec2::new(x as f32 + 0.5 - half, y as f32 + 0.5 - half).length() / half;
        let a = (1.0 - d).max(0.0).powf(falloff);
        [color[0], color[1], color[2], a]
    })
}
