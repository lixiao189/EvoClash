use super::*;

#[test]
fn hash_is_in_unit_range_and_stable() {
    for i in 0..1000 {
        let h = hash(i, -i, 7);
        assert!((0.0..1.0).contains(&h));
        assert_eq!(h, hash(i, -i, 7));
    }
}

#[test]
fn noise_tiles_seamlessly() {
    for y in 0..8 {
        let a = noise(0.0, y as f32 * 0.5, 4, 1);
        let b = noise(4.0, y as f32 * 0.5, 4, 1);
        assert!((a - b).abs() < 1e-6);
    }
}

/// 24-bit BMP, bottom-up rows, each sheet pixel blown up `zoom` times.
fn write_bmp(sheet: &Canvas, zoom: i32, path: &str) {
    let (w, h) = (sheet.w as i32, sheet.h as i32);
    let (zw, zh) = (w * zoom, h * zoom);
    let stride = (zw * 3 + 3) & !3;
    let mut out = Vec::new();
    let size = 54 + stride * zh;
    out.extend(b"BM");
    out.extend((size as u32).to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(54u32.to_le_bytes());
    out.extend(40u32.to_le_bytes());
    out.extend(zw.to_le_bytes());
    out.extend(zh.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(24u16.to_le_bytes());
    out.extend([0u8; 24]);
    for y in (0..zh).rev() {
        let mut line = Vec::new();
        for x in 0..zw {
            let p = sheet.get(x / zoom, y / zoom);
            line.extend([2, 1, 0].map(|i| (p[i].clamp(0.0, 1.0) * 255.0) as u8));
        }
        line.resize(stride as usize, 0);
        out.extend(line);
    }
    std::fs::write(path, out).unwrap();
}

/// Dev aid: `EVOCLASH_SPRITE_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview`
/// writes every survivor walk frame (rows: down, up, side) plus the monsters, blown up 8x.
#[test]
#[ignore]
fn preview_survivor() {
    let Ok(path) = std::env::var("EVOCLASH_SPRITE_PREVIEW") else {
        return;
    };
    let tint = [0.35, 0.6, 0.95];
    let zoom = 8;
    let (cw, ch) = (SURVIVOR.x as i32 + 8, 40);
    let (w, h) = (cw * WALK_FRAMES as i32 + 44, ch * 3);
    let mut sheet = Canvas::filled(w as u32, h as u32, |_, _| rgb(0.36, 0.22, 0.16));
    let blit = |sheet: &mut Canvas, src: &Canvas, dx: i32, dy: i32, tint: [f32; 3]| {
        for y in 0..src.h as i32 {
            for x in 0..src.w as i32 {
                let p = src.get(x, y);
                sheet.put(
                    dx + x,
                    dy + y,
                    [p[0] * tint[0], p[1] * tint[1], p[2] * tint[2], p[3]],
                );
            }
        }
    };
    for (row, facing) in Facing::ALL.iter().enumerate() {
        for frame in 0..WALK_FRAMES {
            let (jacket, gear) = paint_survivor(*facing, frame);
            let (ox, oy) = (frame as i32 * cw + 4, row as i32 * ch + 4);
            blit(&mut sheet, &jacket, ox, oy, tint);
            blit(&mut sheet, &gear, ox, oy, [1.0; 3]);
        }
    }
    let x = cw * WALK_FRAMES as i32 + 4;
    blit(&mut sheet, &paint_roach(0), x, 6, [1.0; 3]);
    blit(&mut sheet, &paint_roach(1), x, 24, [1.0; 3]);
    blit(&mut sheet, &paint_pod(), x, 44, [1.0; 3]);
    for v in 0..5 {
        blit(
            &mut sheet,
            &paint_decor(v),
            x + 2 + (v % 2) * 18,
            84 + (v / 2) * 12,
            [1.0; 3],
        );
    }
    write_bmp(&sheet, zoom, &path);
}

/// Draw `src` at `scale` world pixels per texel, centered at world `at` (y up), rotated by `angle`.
/// One sheet pixel is one world pixel.
fn draw_sprite(
    sheet: &mut Canvas,
    src: &Canvas,
    at: Vec2,
    angle: f32,
    flip: (bool, bool),
    scale: Vec2,
    tint: [f32; 4],
) {
    let (sin, cos) = angle.sin_cos();
    let half = Vec2::new(src.w as f32, src.h as f32) * scale / 2.0;
    let r = half.length().ceil() as i32 + 1;
    for dy in -r..=r {
        for dx in -r..=r {
            let d = Vec2::new(dx as f32 + 0.5, dy as f32 + 0.5);
            // World offset is (d.x, -d.y); undo rotation.
            let w = Vec2::new(d.x, -d.y);
            let l = Vec2::new(w.x * cos + w.y * sin, -w.x * sin + w.y * cos);
            let mut u = (l.x + half.x) / scale.x;
            let mut v = (half.y - l.y) / scale.y;
            if flip.0 {
                u = src.w as f32 - u;
            }
            if flip.1 {
                v = src.h as f32 - v;
            }
            if u < 0.0 || v < 0.0 {
                continue;
            }
            let p = src.get(u as i32, v as i32);
            if p[3] > 0.0 {
                sheet.put(
                    (at.x + dx as f32) as i32,
                    (-at.y + dy as f32) as i32,
                    [
                        p[0] * tint[0],
                        p[1] * tint[1],
                        p[2] * tint[2],
                        p[3] * tint[3],
                    ],
                );
            }
        }
    }
}

/// The baked LPC sheets (scrap-armor outfit), as canvases.
struct Lpc {
    body: Canvas,
    team: Canvas,
    club: Canvas,
    club_behind: Canvas,
}

fn load_lpc() -> Lpc {
    let canvas = |file: &str| {
        let img = crate::lpc::bake::read(&crate::lpc::bake::assets().join(file));
        let mut c = Canvas::new(img.w, img.h);
        for (dst, p) in c.px.iter_mut().zip(&img.px) {
            *dst = p.map(|v| v as f32 / 255.0);
        }
        c
    };
    Lpc {
        body: canvas(crate::lpc::BODY_FILES[1]),
        team: canvas(crate::lpc::TEAM_FILE),
        club: canvas(crate::lpc::CLUB_FILE),
        club_behind: canvas(crate::lpc::CLUB_BEHIND_FILE),
    }
}

fn cell(sheet: &Canvas, size: u32, columns: u32, index: u32) -> Canvas {
    let (x0, y0) = (
        (index % columns * size) as i32,
        (index / columns * size) as i32,
    );
    Canvas::filled(size, size, |x, y| sheet.get(x0 + x, y0 + y))
}

/// A player standing at `feet` on stride frame `stride`, holding `kind` along `aim`, `since`
/// seconds after attacking, posed the way render/players.rs does it: LPC sheets with our guns.
#[allow(clippy::too_many_arguments)]
fn draw_armed(
    sheet: &mut Canvas,
    lpc: &Lpc,
    kind: WeaponKind,
    feet: Vec2,
    aim: Vec2,
    since: f32,
    stride: usize,
) {
    use crate::lpc::{self, Action};
    let tint = [0.35, 0.6, 0.95, 1.0];
    let one = Vec2::ONE;
    let (facing, flip) = Facing::from_aim(aim);
    let row = lpc::row(facing, flip);
    let melee = kind.melee().is_some();
    let swing = crate::render::SWING_SECS;
    let action = if melee && since < swing {
        Action::Swing {
            since,
            total: swing,
        }
    } else {
        Action::Walk(stride)
    };
    let frame = lpc::frame(action, melee);
    let center = feet + Vec2::Y * (lpc::FEET - lpc::CELL as f32 / 2.0);
    let body = |c: &Canvas| cell(c, lpc::CELL, lpc::COLUMNS, row * lpc::COLUMNS + frame.body);
    let club = |c: &Canvas, col: u32| {
        cell(
            c,
            lpc::CLUB_CELL,
            lpc::CLUB_COLUMNS,
            row * lpc::CLUB_COLUMNS + col,
        )
    };

    let (_, weapon, grip) = paint_weapon(kind);
    let size = Vec2::new(weapon.w as f32, weapon.h as f32);
    let attack = crate::render::attack_pose(kind, aim, since);
    let held = Vec2::from_angle(aim.to_angle() + attack.swing);
    let left = held.x < 0.0;
    let gun_at = held_layout(
        feet + lpc::gun_grip(row) + attack.push,
        held,
        size,
        grip,
        left,
        1.0,
    );
    let gun = |sheet: &mut Canvas| {
        if !melee {
            draw_sprite(
                sheet,
                &weapon,
                gun_at,
                held.to_angle(),
                (false, left),
                one,
                [1.0; 4],
            );
        }
    };
    if row == 0 {
        gun(sheet);
    }
    if let Some(col) = frame.club {
        draw_sprite(
            sheet,
            &club(&lpc.club_behind, col),
            center,
            0.0,
            (false, false),
            one,
            [1.0; 4],
        );
    }
    draw_sprite(
        sheet,
        &body(&lpc.body),
        center,
        0.0,
        (false, false),
        one,
        [1.0; 4],
    );
    draw_sprite(
        sheet,
        &body(&lpc.team),
        center,
        0.0,
        (false, false),
        one,
        tint,
    );
    if let Some(col) = frame.club {
        draw_sprite(
            sheet,
            &club(&lpc.club, col),
            center,
            0.0,
            (false, false),
            one,
            [1.0; 4],
        );
    }
    if row != 0 {
        gun(sheet);
    }
    if !melee && attack.flash_alpha > 0.0 {
        let muzzle = gun_at + held * (size.x / 2.0 - 1.0);
        let flash = paint_muzzle();
        let scale = attack.flash / Vec2::new(flash.w as f32, flash.h as f32);
        let c = kind.spec().color().mix(&Color::WHITE, 0.45).to_srgba();
        draw_sprite(
            sheet,
            &flash,
            muzzle + held * attack.flash.x / 2.0,
            held.to_angle(),
            (false, false),
            scale,
            [c.red, c.green, c.blue, attack.flash_alpha],
        );
    }
}

fn checker(w: u32, h: u32) -> Canvas {
    Canvas::filled(w, h, |x, y| {
        if (x / 8 + y / 8) % 2 == 0 {
            rgb(0.78, 0.62, 0.36)
        } else {
            rgb(0.74, 0.58, 0.33)
        }
    })
}

const ALL_WEAPONS: [WeaponKind; 6] = [
    WeaponKind::Stick,
    WeaponKind::Bow,
    WeaponKind::PipeShotgun,
    WeaponKind::Rifle,
    WeaponKind::Laser,
    WeaponKind::Railgun,
];

/// Dev aid: `EVOCLASH_ARMED_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_armed`
/// poses a survivor holding every weapon at several aim angles, the way the renderer does.
#[test]
#[ignore]
fn preview_armed() {
    let Ok(path) = std::env::var("EVOCLASH_ARMED_PREVIEW") else {
        return;
    };
    let angles = [0.0f32, 35.0, 80.0, 145.0, 180.0, -45.0, -90.0, -140.0];
    let (cw, ch) = (96, 96);
    let lpc = load_lpc();
    let mut sheet = checker(cw * angles.len() as u32, ch * ALL_WEAPONS.len() as u32);
    for (row, kind) in ALL_WEAPONS.iter().enumerate() {
        for (col, deg) in angles.iter().enumerate() {
            let feet = Vec2::new(
                col as f32 * cw as f32 + 48.0,
                -(row as f32 * ch as f32 + 84.0),
            );
            let aim = Vec2::from_angle(deg.to_radians());
            draw_armed(&mut sheet, &lpc, *kind, feet, aim, f32::INFINITY, 0);
        }
    }
    write_bmp(&sheet, 4, &path);
}

/// Dev aid: `EVOCLASH_ATTACK_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_attacks`
/// steps through a swing and every gun's recoil and muzzle flash, aiming right then left
/// (one column per 1/30 s after the strike).
#[test]
#[ignore]
fn preview_attacks() {
    let Ok(path) = std::env::var("EVOCLASH_ATTACK_PREVIEW") else {
        return;
    };
    let steps = 10;
    let (cw, ch) = (96, 96);
    let lpc = load_lpc();
    let mut sheet = checker(cw * steps, ch * 2 * ALL_WEAPONS.len() as u32);
    for (k, kind) in ALL_WEAPONS.iter().enumerate() {
        for (side, aim) in [Vec2::X, Vec2::new(-1.0, 0.3).normalize()]
            .iter()
            .enumerate()
        {
            let row = k * 2 + side;
            for col in 0..steps {
                let feet = Vec2::new(
                    col as f32 * cw as f32 + 48.0,
                    -(row as f32 * ch as f32 + 80.0),
                );
                draw_armed(&mut sheet, &lpc, *kind, feet, *aim, col as f32 / 30.0, 0);
            }
        }
    }
    write_bmp(&sheet, 3, &path);
}

/// Dev aid: `EVOCLASH_POSES_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_poses`
/// lays out a character sheet: facing up, down, left and right (four columns each), rows
/// walking with the club, swinging it, walking with a rifle, and firing it.
#[test]
#[ignore]
fn preview_poses() {
    let Ok(path) = std::env::var("EVOCLASH_POSES_PREVIEW") else {
        return;
    };
    let lpc = load_lpc();
    let dirs = [90.0f32, -90.0, 180.0, 0.0];
    let (cw, ch) = (64, 76);
    let mut sheet = checker(cw * 16, ch * 4);
    for row in 0..4 {
        for (d, deg) in dirs.iter().enumerate() {
            for k in 0..4 {
                let col = d * 4 + k;
                let feet = Vec2::new(
                    col as f32 * cw as f32 + 32.0,
                    -(row as f32 * ch as f32 + 68.0),
                );
                let aim = Vec2::from_angle(deg.to_radians());
                let (kind, since, stride) = match row {
                    0 => (WeaponKind::Stick, f32::INFINITY, 1 + 2 * k),
                    1 => (WeaponKind::Stick, 0.02 + 0.07 * k as f32, 0),
                    2 => (WeaponKind::Rifle, f32::INFINITY, 1 + 2 * k),
                    _ => (WeaponKind::Rifle, 0.015 * k as f32, 0),
                };
                draw_armed(&mut sheet, &lpc, kind, feet, aim, since, stride);
            }
        }
    }
    write_bmp(&sheet, 3, &path);
}

/// Dev aid: `EVOCLASH_CITY_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_city`
/// lays out the city tiles (2x2 repeats, to check seams) and the props.
#[test]
#[ignore]
fn preview_city() {
    let Ok(path) = std::env::var("EVOCLASH_CITY_PREVIEW") else {
        return;
    };
    let tiles = [
        paint_pavement(),
        paint_asphalt(),
        paint_oily_concrete(),
        paint_rad_ground(),
        paint_planks(),
        paint_tiles(),
        paint_floor_concrete(),
        paint_roof(Roof::Tin),
        paint_roof(Roof::Tar),
        paint_roof(Roof::Concrete),
        paint_metal(),
        paint_metal_top(),
        paint_rock(false),
        paint_rock(true),
        paint_sandbags(false),
        paint_sandbags(true),
    ];
    let props = [
        paint_crate(),
        paint_barrel(),
        paint_safe(false),
        paint_safe(true),
    ];
    let cell = 132;
    let mut sheet = Canvas::filled(cell * 4, cell * 4 + 30, |_, _| rgb(0.36, 0.22, 0.16));
    for (i, tile) in tiles.iter().enumerate() {
        let (ox, oy) = ((i % 4) as i32 * cell as i32, (i / 4) as i32 * cell as i32);
        for y in 0..128 {
            for x in 0..128 {
                let p = tile.get(x % tile.w as i32, y % tile.h as i32);
                sheet.put(ox + x, oy + y, p);
            }
        }
    }
    for (i, prop) in props.iter().enumerate() {
        let (ox, oy) = (i as i32 * 30 + 4, cell as i32 * 4 + 4);
        for y in 0..prop.h as i32 {
            for x in 0..prop.w as i32 {
                sheet.put(ox + x, oy + y, prop.get(x, y));
            }
        }
    }
    write_bmp(&sheet, 3, &path);
}

/// Dev aid: bolt, slash, shock ring and fireball (tinted) over ground color, blown up 3x.
#[test]
#[ignore]
fn preview_effects() {
    let Ok(path) = std::env::var("EVOCLASH_SPRITE_PREVIEW") else {
        return;
    };
    let mut sheet = Canvas::filled(400, 120, |_, _| rgb(0.36, 0.22, 0.16));
    let tints = [
        (paint_bolt(), rgb(1.0, 0.9, 0.4), 0, 10),
        (paint_muzzle(), rgb(1.0, 0.85, 0.55), 0, 40),
        (paint_slash(1.1, 0.7), rgb(1.0, 0.92, 0.75), 60, 10),
        (paint_shock(), rgb(1.0, 0.6, 0.2), 160, 10),
        (paint_blast(), rgb(1.0, 0.5, 0.12), 290, 10),
    ];
    for (c, tint, ox, oy) in tints {
        for y in 0..c.h as i32 {
            for x in 0..c.w as i32 {
                let p = c.get(x, y);
                let (px, py) = (ox + x, oy + y);
                if px >= 400 || py >= 120 {
                    continue;
                }
                let under = sheet.get(px, py);
                let src = [p[0] * tint[0], p[1] * tint[1], p[2] * tint[2], 1.0];
                sheet.put(px, py, mix(under, src, p[3]));
            }
        }
    }
    write_bmp(&sheet, 3, &path);
}

/// Dev aid: `EVOCLASH_SPRITE_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_monsters`
/// shows every monster next to a survivor for scale (2x world size), blown up 3x.
#[test]
#[ignore]
fn preview_monsters() {
    let Ok(path) = std::env::var("EVOCLASH_SPRITE_PREVIEW") else {
        return;
    };
    let mut sheet = Canvas::filled(560, 300, |x, y| {
        if (x / 8 + y / 8) % 2 == 0 {
            rgb(0.36, 0.22, 0.16)
        } else {
            rgb(0.33, 0.2, 0.15)
        }
    });
    // Everything here is drawn at world scale: PIXEL (2) sheet pixels per texture pixel.
    let draw = |sheet: &mut Canvas, src: &Canvas, x: i32, feet: i32, tint: [f32; 3]| {
        for y in 0..src.h as i32 * 2 {
            for xx in 0..src.w as i32 * 2 {
                let p = src.get(xx / 2, y / 2);
                if p[3] > 0.0 {
                    sheet.put(
                        x + xx,
                        feet - src.h as i32 * 2 + y,
                        [p[0] * tint[0], p[1] * tint[1], p[2] * tint[2], p[3]],
                    );
                }
            }
        }
    };
    let (jacket, gear) = paint_survivor(Facing::Side, 0);
    draw(&mut sheet, &jacket, 4, 80, [0.35, 0.6, 0.95]);
    draw(&mut sheet, &gear, 4, 80, [1.0; 3]);
    let gold = [1.35, 1.1, 0.5];
    let row1 = [
        paint_roach(0),
        paint_pod(),
        paint_dog(0),
        paint_dog(1),
        paint_ghoul(0),
        paint_spitter(),
        paint_crawler(),
    ];
    let mut x = 50;
    for (i, c) in row1.iter().enumerate() {
        draw(&mut sheet, c, x, 80, if i == 3 { gold } else { [1.0; 3] });
        x += c.w as i32 * 2 + 4;
    }
    let row2 = [
        paint_raider(0),
        paint_raider(1),
        paint_drone(0),
        paint_drone(1),
        paint_acid(),
    ];
    let mut x = 4;
    for c in &row2 {
        draw(&mut sheet, c, x, 150, [1.0; 3]);
        x += c.w as i32 * 2 + 4;
    }
    draw(&mut sheet, &paint_behemoth(0), 4, 290, [1.0; 3]);
    draw(&mut sheet, &paint_behemoth(1), 160, 290, [1.0, 0.5, 0.42]);
    write_bmp(&sheet, 3, &path);
}

/// Dev aid: 自动炮塔 (base with the head in four directions) and 陷阱 over ground color, 6x.
#[test]
#[ignore]
fn preview_skills() {
    let Ok(path) = std::env::var("EVOCLASH_SPRITE_PREVIEW") else {
        return;
    };
    let mut sheet = Canvas::filled(150, 40, |_, _| rgb(0.36, 0.22, 0.16));
    let mut blit = |c: &Canvas, ox: i32, oy: i32, flip: bool| {
        for y in 0..c.h as i32 {
            for x in 0..c.w as i32 {
                let sx = if flip { c.w as i32 - 1 - x } else { x };
                sheet.put(ox + x, oy + y, c.get(sx, y));
            }
        }
    };
    let (base, head) = (paint_turret_base(), paint_turret_head());
    for (i, flip) in [false, true].into_iter().enumerate() {
        let ox = 4 + 34 * i as i32;
        blit(&base, ox + 3, 16, false);
        blit(&head, ox, 8, flip);
    }
    blit(&paint_trap(), 80, 14, false);
    blit(&paint_trap(), 110, 14, false);
    write_bmp(&sheet, 6, &path);
}

#[test]
fn sprites_have_opaque_pixels() {
    for canvas in [
        paint_turret_base(),
        paint_turret_head(),
        paint_trap(),
        paint_survivor(Facing::Side, 1).0,
        paint_roach(0),
        paint_pod(),
        paint_dog(1),
        paint_ghoul(0),
        paint_spitter(),
        paint_crawler(),
        paint_raider(2),
        paint_drone(1),
        paint_behemoth(1),
        paint_acid(),
        paint_medkit(),
        paint_reticle(),
        paint_bar(),
    ] {
        assert!(canvas.px.iter().any(|p| p[3] > 0.9));
    }
    // Soft effects never reach full opacity but must not be blank.
    for canvas in [
        paint_bolt(),
        paint_slash(1.1, 0.5),
        paint_muzzle(),
        paint_shock(),
        paint_blast(),
    ] {
        assert!(canvas.px.iter().any(|p| p[3] > 0.5));
    }
}
