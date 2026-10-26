//! Armor and consumable pickups.

use super::*;

pub(super) fn paint_armor(kind: ArmorKind) -> Canvas {
    let mut c = Canvas::new(16, 16);
    let (base, trim) = match kind {
        ArmorKind::Kevlar => (rgb(0.3, 0.36, 0.3), rgb(0.2, 0.24, 0.2)),
        ArmorKind::PowerArmor => (rgb(0.8, 0.62, 0.22), rgb(0.35, 0.36, 0.4)),
        _ => (rgb(0.48, 0.4, 0.33), rgb(0.55, 0.32, 0.18)),
    };
    // Vest: torso with a neck cut-out and two shoulder straps.
    c.rect(3, 4, 12, 13, base);
    c.rect(2, 2, 5, 5, base);
    c.rect(10, 2, 13, 5, base);
    for y in 2..6 {
        for x in 6..10 {
            c.px[(y * 16 + x) as usize] = [0.0; 4];
        }
    }
    c.rect(3, 9, 12, 9, trim);
    c.bevel(8.0, 8.0, 7.0, 0.7);
    match kind {
        ArmorKind::Kevlar => {
            c.rect(4, 10, 6, 12, trim);
            c.rect(9, 10, 11, 12, trim);
        }
        ArmorKind::PowerArmor => {
            c.rect(6, 6, 9, 8, rgb(0.3, 0.9, 1.0));
            c.rect(1, 2, 3, 6, trim);
            c.rect(12, 2, 14, 6, trim);
        }
        _ => {
            for (x, y) in [(4, 5), (11, 5), (4, 12), (11, 12), (7, 11)] {
                c.put(x, y, rgb(0.75, 0.7, 0.62));
            }
        }
    }
    c.outline(OUTLINE);
    c
}

pub(super) fn paint_medkit() -> Canvas {
    let mut c = Canvas::new(14, 13);
    let white = rgb(0.92, 0.9, 0.86);
    c.rect(1, 3, 12, 11, white);
    c.rect(1, 10, 12, 11, shade(white, 0.75));
    c.rect(5, 1, 8, 2, rgb(0.4, 0.4, 0.4));
    c.rect(6, 4, 7, 9, rgb(0.85, 0.15, 0.12));
    c.rect(4, 6, 9, 7, rgb(0.85, 0.15, 0.12));
    c.outline(OUTLINE);
    c
}

pub(super) fn paint_ammo() -> Canvas {
    let mut c = Canvas::new(14, 11);
    let olive = rgb(0.34, 0.4, 0.22);
    c.rect(1, 3, 12, 9, olive);
    c.rect(1, 3, 12, 3, shade(olive, 1.3));
    c.rect(1, 9, 12, 9, shade(olive, 0.7));
    c.rect(1, 5, 12, 5, shade(olive, 0.8));
    // Handle and a row of brass cartridge tips peeking out of the lid.
    c.rect(5, 1, 8, 2, rgb(0.4, 0.4, 0.38));
    let brass = rgb(0.85, 0.65, 0.2);
    for x in [3, 5, 7, 9] {
        c.rect(x, 6, x + 1, 8, brass);
        c.put(x, 6, shade(brass, 1.35));
    }
    c.outline(OUTLINE);
    c
}

pub(super) fn paint_emp() -> Canvas {
    let mut c = Canvas::new(13, 13);
    c.disc(6.5, 7.0, 5.0, rgb(0.22, 0.24, 0.28));
    c.bevel(6.5, 7.0, 5.0, 0.9);
    c.rect(2, 6, 10, 7, rgb(0.3, 0.75, 1.0));
    c.put(6, 6, rgb(0.85, 1.0, 1.0));
    c.rect(5, 1, 7, 2, rgb(0.55, 0.55, 0.5));
    c.put(8, 0, rgb(0.75, 0.7, 0.4));
    c.outline(OUTLINE);
    c
}
