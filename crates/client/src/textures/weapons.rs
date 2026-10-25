//! Weapons, as loot icons and held with fists on them.

use super::*;

/// Paint a weapon facing +X. Returns the loot icon, the held version with fists on it, and the
/// rear grip in texture pixels.
pub(super) fn paint_weapon(kind: WeaponKind) -> (Canvas, Canvas, Vec2) {
    let (base, fists): (Canvas, &[(i32, i32)]) = match kind {
        WeaponKind::Stick => {
            // Splintered club, tape-wrapped handle, a couple of bent nails.
            let mut c = Canvas::new(20, 9);
            let wood = rgb(0.6, 0.42, 0.22);
            c.rect(1, 3, 9, 4, wood);
            c.rect(1, 3, 9, 3, shade(wood, 1.22));
            c.rect(1, 4, 9, 4, shade(wood, 0.72));
            c.rect(1, 2, 1, 5, shade(wood, 0.65));
            let tape = rgb(0.72, 0.68, 0.58);
            for x in [2, 4, 6] {
                c.rect(x, 3, x, 4, tape);
                c.put(x, 4, shade(tape, 0.75));
            }
            for x in 9..=18 {
                let t = (x - 9) as f32 / 9.0;
                let half = 1.0 + t * 1.3;
                let (y0, y1) = ((3.5 - half).round() as i32, (3.5 + half).round() as i32 - 1);
                c.rect(x, y0, x, y1, wood);
                c.put(x, y0, shade(wood, 1.25));
                c.put(x, y1, shade(wood, 0.7));
            }
            c.rect(18, 2, 18, 5, shade(wood, 0.85));
            for (x, y) in [(11, 3), (14, 4), (16, 2)] {
                c.put(x, y, shade(wood, 0.62));
            }
            let nail = rgb(0.72, 0.72, 0.74);
            for (x, y, dx) in [(12, 1, 1), (15, 0, -1), (14, 6, 1)] {
                c.put(x, y, nail);
                c.put(x + dx, y + if y > 3 { 1 } else { -1 }, shade(nail, 0.7));
            }
            (c, &[(3, 2)])
        }
        WeaponKind::Bow => {
            // Recurve bow at full draw: the rear hand pinches the nock at the chest, the fore
            // hand grips the wrapped handle 15 px ahead (the span of the character's two hands).
            let mut c = Canvas::new(27, 21);
            let wood = rgb(0.64, 0.44, 0.22);
            for y in 1..=19 {
                let t = (y as f32 - 10.0) / 9.0;
                let x = (15.0 + 4.5 * (1.0 - t * t)).round() as i32;
                c.put(x, y, wood);
                if t.abs() < 0.6 {
                    c.put(x - 1, y, shade(wood, 1.25));
                } else if t.abs() > 0.88 {
                    c.put(x + 1, y, shade(wood, 0.8));
                }
            }
            c.rect(18, 8, 19, 12, rgb(0.36, 0.22, 0.13));
            c.put(18, 9, rgb(0.5, 0.33, 0.2));
            c.put(18, 11, rgb(0.5, 0.33, 0.2));
            let string = rgb(0.9, 0.88, 0.78);
            c.line(15.0, 1.5, 3.5, 10.5, string);
            c.line(15.0, 19.5, 3.5, 10.5, string);
            let shaft = rgb(0.78, 0.66, 0.45);
            c.rect(2, 10, 23, 10, shaft);
            let head = rgb(0.62, 0.64, 0.66);
            c.rect(24, 10, 25, 10, head);
            c.put(24, 9, shade(head, 1.2));
            c.put(24, 11, shade(head, 0.7));
            let feather = rgb(0.82, 0.25, 0.2);
            c.rect(1, 9, 2, 9, feather);
            c.rect(1, 11, 2, 11, shade(feather, 0.75));
            // Draw hand first: it is the rear grip the sprite pivots on.
            (c, &[(2, 9), (17, 8)])
        }
        WeaponKind::PipeShotgun => {
            // Scrap pipe on a wooden stock, held together with tape and a hose clamp.
            let mut c = Canvas::new(23, 9);
            let wood = rgb(0.5, 0.33, 0.18);
            c.rect(1, 3, 2, 7, wood);
            c.rect(3, 3, 7, 5, wood);
            c.rect(1, 3, 7, 3, shade(wood, 1.25));
            c.rect(1, 7, 2, 7, shade(wood, 0.65));
            c.rect(3, 5, 7, 5, shade(wood, 0.72));
            c.put(1, 5, shade(wood, 0.6));
            let iron = rgb(0.42, 0.4, 0.38);
            c.rect(8, 2, 11, 5, iron);
            c.rect(8, 2, 11, 2, shade(iron, 1.35));
            c.rect(8, 5, 11, 5, shade(iron, 0.7));
            c.put(10, 3, rgb(0.55, 0.3, 0.16));
            c.rect(9, 6, 10, 6, shade(iron, 0.7));
            c.put(8, 6, shade(iron, 0.7));
            let pipe = rgb(0.55, 0.55, 0.52);
            c.rect(11, 2, 20, 4, pipe);
            c.rect(11, 2, 20, 2, shade(pipe, 1.35));
            c.rect(11, 4, 20, 4, shade(pipe, 0.66));
            for (x, y) in [(13, 3), (18, 4), (16, 2)] {
                c.put(x, y, rgb(0.6, 0.36, 0.2));
            }
            c.rect(20, 1, 21, 5, shade(pipe, 0.8));
            c.rect(20, 1, 21, 1, shade(pipe, 1.2));
            c.put(21, 3, OUTLINE);
            let tape = rgb(0.68, 0.7, 0.66);
            c.rect(17, 2, 17, 4, tape);
            c.put(17, 4, shade(tape, 0.72));
            (c, &[(5, 3), (14, 2)])
        }
        WeaponKind::Rifle => {
            // Scoped rifle: stock, pistol grip, curved mag, vented handguard, muzzle brake.
            let mut c = Canvas::new(27, 11);
            let poly = rgb(0.32, 0.27, 0.2);
            c.rect(1, 3, 1, 7, shade(poly, 0.6));
            c.rect(2, 3, 4, 6, poly);
            c.rect(5, 4, 7, 5, poly);
            c.rect(2, 3, 7, 3, shade(poly, 1.3));
            c.rect(2, 6, 4, 6, shade(poly, 0.72));
            let metal = rgb(0.25, 0.27, 0.26);
            c.rect(7, 3, 15, 5, metal);
            c.rect(7, 3, 15, 3, shade(metal, 1.6));
            c.rect(7, 5, 15, 5, shade(metal, 0.7));
            c.rect(12, 4, 13, 4, shade(metal, 1.9));
            c.rect(7, 6, 8, 8, shade(metal, 0.8));
            c.put(9, 6, shade(metal, 0.6));
            c.rect(11, 6, 12, 7, shade(metal, 0.85));
            c.rect(12, 8, 13, 8, shade(metal, 0.85));
            c.put(11, 6, shade(metal, 1.3));
            let scope = rgb(0.14, 0.15, 0.15);
            c.rect(8, 1, 13, 2, scope);
            c.rect(8, 1, 12, 1, shade(scope, 2.0));
            c.put(13, 1, rgb(0.55, 0.95, 1.0));
            c.put(13, 2, rgb(0.2, 0.55, 0.65));
            let guard = rgb(0.36, 0.33, 0.26);
            c.rect(16, 3, 21, 5, guard);
            c.rect(16, 3, 21, 3, shade(guard, 1.3));
            c.rect(16, 5, 21, 5, shade(guard, 0.7));
            for x in [17, 19, 21] {
                c.put(x, 4, shade(guard, 0.55));
            }
            c.rect(22, 4, 24, 4, metal);
            c.put(23, 3, shade(metal, 1.4));
            c.rect(25, 3, 25, 5, shade(metal, 1.2));
            (c, &[(6, 4), (17, 3)])
        }
        WeaponKind::Laser => {
            // Pre-war energy carbine: white shell, glowing cell window, finned emitter.
            let mut c = Canvas::new(23, 10);
            let shell = rgb(0.86, 0.88, 0.9);
            c.rect(2, 2, 15, 5, shell);
            c.rect(1, 3, 1, 5, shade(shell, 0.85));
            c.rect(2, 2, 15, 2, shade(shell, 1.12));
            c.rect(2, 5, 15, 5, shade(shell, 0.68));
            c.clear(2, 2);
            c.rect(3, 3, 3, 4, rgb(0.95, 0.55, 0.15));
            let cell = rgb(0.15, 0.85, 0.9);
            c.rect(6, 3, 13, 4, shade(cell, 0.6));
            for x in [7, 9, 11, 13] {
                c.put(x, 3, rgb(0.75, 1.0, 1.0));
                c.put(x - 1, 4, cell);
            }
            c.rect(4, 6, 6, 7, rgb(0.3, 0.32, 0.36));
            let fin = rgb(0.2, 0.42, 0.48);
            for x in [16, 18] {
                c.rect(x, 1, x, 6, fin);
                c.put(x, 1, shade(fin, 1.5));
            }
            c.rect(17, 3, 19, 4, shade(fin, 0.75));
            c.rect(20, 3, 21, 4, rgb(0.55, 1.0, 1.0));
            c.put(21, 3, rgb(0.95, 1.0, 1.0));
            (c, &[(4, 4), (11, 4)])
        }
        WeaponKind::Railgun => {
            // Twin rails with glowing coils and a capacitor pack.
            let mut c = Canvas::new(31, 12);
            let body = rgb(0.27, 0.24, 0.32);
            c.rect(1, 4, 6, 7, body);
            c.rect(1, 4, 6, 4, shade(body, 1.35));
            c.rect(1, 7, 6, 7, shade(body, 0.7));
            c.rect(6, 2, 12, 8, shade(body, 1.1));
            c.rect(6, 2, 12, 2, shade(body, 1.6));
            c.rect(6, 8, 12, 8, shade(body, 0.75));
            c.rect(8, 4, 10, 5, rgb(0.75, 0.4, 1.0));
            c.put(8, 4, rgb(1.0, 0.85, 1.0));
            let rail = rgb(0.5, 0.48, 0.56);
            c.rect(12, 2, 28, 2, rail);
            c.rect(12, 8, 28, 8, shade(rail, 0.75));
            c.rect(12, 3, 26, 7, rgb(0.14, 0.08, 0.2));
            c.rect(12, 5, 26, 5, rgb(0.55, 0.25, 0.8));
            for x in [14, 18, 22] {
                c.rect(x, 1, x + 1, 9, rgb(0.62, 0.32, 0.9));
                c.rect(x, 1, x + 1, 1, rgb(1.0, 0.82, 1.0));
                c.rect(x + 1, 2, x + 1, 9, rgb(0.42, 0.2, 0.62));
            }
            c.rect(27, 4, 29, 6, rgb(0.85, 0.6, 1.0));
            c.put(29, 5, rgb(1.0, 0.95, 1.0));
            (c, &[(6, 5), (10, 6)])
        }
    };
    let mut icon = base.clone();
    icon.outline(OUTLINE);
    let mut held = base;
    for &(x, y) in fists {
        paint_fist(&mut held, x, y);
    }
    held.outline(OUTLINE);
    let (x, y) = fists[0];
    (icon, held, Vec2::new(x as f32 + 1.5, y as f32 + 2.0))
}
