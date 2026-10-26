//! Small UI badges: the match host's crown and the bot name tag.
//!
//! Authored one texel per art pixel, like the other procedural sprites. Drawn at [`PIXEL`]
//! they land at about 22×16 (crown) and 28×14 (tag) world pixels.

use super::*;

/// Flat gold crown, three separated points and one ruby. 11×8 texels.
pub(super) fn paint_host_crown() -> Canvas {
    let mut c = Canvas::new(11, 8);
    let hot = rgb(1.0, 0.97, 0.72);
    let hi = rgb(1.0, 0.9, 0.42);
    let gold = rgb(0.93, 0.7, 0.18);
    let mid = rgb(0.74, 0.5, 0.11);
    let band = rgb(0.48, 0.3, 0.06);
    let ruby = rgb(0.95, 0.15, 0.14);
    // Tips at x = 2, 5, 8. Shoulders are three wide and stop short of the cuts.
    for x in [2, 5, 8] {
        c.put(x, 1, hot);
    }
    for x in [1, 2, 3, 5, 7, 8, 9] {
        c.put(x, 2, hi);
    }
    for y in 3..=4 {
        c.rect(1, y, 9, y, gold);
    }
    c.rect(1, 5, 9, 5, mid);
    c.put(5, 5, ruby);
    c.rect(1, 6, 9, 6, band);
    c.outline(OUTLINE);
    // `outline` fills a one-pixel gap. Re-open the cuts so the three points stay apart.
    for (x, y) in [(3, 1), (4, 1), (6, 1), (7, 1), (4, 2), (6, 2)] {
        c.clear(x, y);
    }
    c
}

/// Charcoal rounded tag with white "BOT". 14×7 texels.
pub(super) fn paint_bot_tag() -> Canvas {
    let mut c = Canvas::new(14, 7);
    let fill = rgb(0.15, 0.16, 0.17);
    let bottom = rgb(0.07, 0.07, 0.08);
    let rim = rgb(0.03, 0.03, 0.04);
    for y in 0..7 {
        for x in 0..14 {
            // Chamfer the four corners so it reads as rounded at this size.
            if (y == 0 || y == 6) && (x == 0 || x == 13) {
                continue;
            }
            c.put(x, y, if y == 6 { bottom } else { fill });
        }
    }
    for x in 1..13 {
        c.put(x, 0, rim);
        c.put(x, 6, rim);
    }
    for y in 1..6 {
        c.put(0, y, rim);
        c.put(13, y, rim);
    }

    let ink = rgb(1.0, 1.0, 1.0);
    // 3×5 glyphs. A one-texel gap between them, inset so the rim stays clear.
    let glyphs: [[[u8; 3]; 5]; 3] = [
        [[1, 1, 0], [1, 0, 1], [1, 1, 0], [1, 0, 1], [1, 1, 0]], // B
        [[1, 1, 1], [1, 0, 1], [1, 0, 1], [1, 0, 1], [1, 1, 1]], // O
        [[1, 1, 1], [0, 1, 0], [0, 1, 0], [0, 1, 0], [0, 1, 0]], // T
    ];
    for (i, glyph) in glyphs.iter().enumerate() {
        let ox = 2 + i as i32 * 4;
        for (row, bits) in glyph.iter().enumerate() {
            for (col, on) in bits.iter().enumerate() {
                if *on == 1 {
                    c.put(ox + col as i32, 1 + row as i32, ink);
                }
            }
        }
    }
    c
}
