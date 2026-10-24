//! Deployables from path skills: turrets and traps.

use super::*;

/// Tripod and ammo box of the 自动炮塔; the head sits on the post at the top.
pub(super) fn paint_turret_base() -> Canvas {
    let mut c = Canvas::new(20, 16);
    let steel = rgb(0.32, 0.34, 0.33);
    // Splayed legs.
    c.line(9.5, 6.0, 2.0, 15.0, shade(steel, 0.8));
    c.line(10.5, 6.0, 18.0, 15.0, shade(steel, 0.8));
    c.line(10.0, 6.0, 10.0, 15.0, shade(steel, 0.65));
    c.rect(1, 14, 3, 15, shade(steel, 0.6));
    c.rect(17, 14, 19, 15, shade(steel, 0.6));
    // Post and an olive ammo can strapped to it.
    c.rect(8, 0, 11, 7, steel);
    c.rect(5, 7, 14, 11, rgb(0.33, 0.38, 0.2));
    c.rect(5, 7, 14, 7, rgb(0.45, 0.5, 0.3));
    c.rect(9, 8, 10, 10, rgb(0.75, 0.65, 0.3));
    c.outline(OUTLINE);
    c
}

/// Gun head, centered on its pivot, barrel pointing +X.
pub(super) fn paint_turret_head() -> Canvas {
    let mut c = Canvas::new(26, 12);
    let steel = rgb(0.4, 0.43, 0.42);
    // Barrel with a muzzle brake.
    c.rect(13, 5, 23, 6, shade(steel, 0.7));
    c.rect(22, 4, 24, 7, shade(steel, 0.55));
    // Housing, sensor eye and a cooling fin.
    c.ellipse(11.5, 6.0, 7.0, 4.5, steel);
    c.bevel(11.5, 6.0, 7.0, 0.8);
    c.rect(7, 1, 12, 2, shade(steel, 0.8));
    c.disc(14.0, 5.0, 1.3, rgb(1.0, 0.25, 0.15));
    c.put(13, 4, rgb(1.0, 0.8, 0.7));
    c.outline(OUTLINE);
    c
}

/// Open bear trap: two toothed jaws around a pressure plate, chained to a stake.
pub(super) fn paint_trap() -> Canvas {
    let mut c = Canvas::new(22, 16);
    let iron = rgb(0.45, 0.42, 0.38);
    c.ellipse(11.0, 8.0, 9.5, 6.5, iron);
    for x in 0..22 {
        for y in 0..16 {
            let d = Vec2::new((x as f32 + 0.5 - 11.0) / 7.5, (y as f32 + 0.5 - 8.0) / 4.5);
            if d.length() < 1.0 {
                c.clear(x, y);
            }
        }
    }
    // Teeth on both jaws.
    for x in [5, 8, 11, 14, 17] {
        c.put(x, 4, shade(iron, 1.3));
        c.put(x, 11, shade(iron, 1.3));
    }
    // Spring bars and the plate.
    c.rect(0, 7, 2, 8, shade(iron, 0.7));
    c.rect(19, 7, 21, 8, shade(iron, 0.7));
    c.disc(11.0, 8.0, 2.4, rgb(0.6, 0.5, 0.3));
    c.put(10, 7, rgb(0.85, 0.75, 0.5));
    c.outline(OUTLINE);
    c
}
