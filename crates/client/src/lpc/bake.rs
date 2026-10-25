//! Builds the sheets in `assets/lpc/` from the art vendored in `assets/lpc-src/` (character
//! generator layers, and crops of the LPC wall, roof, terrain and street tilesets in `map/`):
//! `cargo test -p evoclash -- --ignored bake_lpc_sheets`. Rerun it after changing a layer, a
//! tint or a layout, and keep `assets/CREDITS.md` in step.

use std::path::{Path, PathBuf};

use super::*;

/// Decoded RGBA8 image.
#[derive(Clone)]
pub(crate) struct Rgba {
    pub w: u32,
    pub h: u32,
    pub px: Vec<[u8; 4]>,
}

impl Rgba {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![[0; 4]; (w * h) as usize],
        }
    }

    pub fn get(&self, x: i32, y: i32) -> [u8; 4] {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return [0; 4];
        }
        self.px[(y as u32 * self.w + x as u32) as usize]
    }

    /// Alpha-blend `c`, multiplied by `tint`, over a pixel.
    fn over(&mut self, x: i32, y: i32, c: [u8; 4], tint: [u8; 3]) {
        if c[3] == 0 || x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let d = &mut self.px[(y as u32 * self.w + x as u32) as usize];
        let a = c[3] as f32 / 255.0;
        let da = d[3] as f32 / 255.0;
        let out = a + da * (1.0 - a);
        for i in 0..3 {
            let s = c[i] as f32 * tint[i] as f32 / 255.0;
            d[i] = ((s * a + d[i] as f32 * da * (1.0 - a)) / out).round() as u8;
        }
        d[3] = (out * 255.0).round() as u8;
    }

    /// Draw the `w`x`h` region of `src` at (`sx`, `sy`) with its corner at (`dx`, `dy`).
    #[allow(clippy::too_many_arguments)]
    fn blit(
        &mut self,
        src: &Rgba,
        sx: i32,
        sy: i32,
        w: i32,
        h: i32,
        dx: i32,
        dy: i32,
        tint: [u8; 3],
    ) {
        for y in 0..h {
            for x in 0..w {
                self.over(dx + x, dy + y, src.get(sx + x, sy + y), tint);
            }
        }
    }
}

pub(crate) fn read(path: &Path) -> Rgba {
    let mut decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    ));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let bytes = &buf[..info.buffer_size()];
    let px = match info.color_type {
        png::ColorType::Rgba => bytes.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
        png::ColorType::Rgb => bytes.chunks(3).map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => {
            bytes.chunks(2).map(|c| [c[0], c[0], c[0], c[1]]).collect()
        }
        png::ColorType::Grayscale => bytes.iter().map(|&v| [v, v, v, 255]).collect(),
        other => panic!("{}: unexpected {other:?}", path.display()),
    };
    Rgba {
        w: info.width,
        h: info.height,
        px,
    }
}

fn write(img: &Rgba, path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), img.w, img.h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let data: Vec<u8> = img.px.iter().flatten().copied().collect();
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&data)
        .unwrap();
}

pub(crate) fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
}

/// The generator's white base layers, recolored for a dusty survivor.
const PANTS: [u8; 3] = [78, 78, 84];
const BOOTS: [u8; 3] = [92, 66, 46];
const SHIRT: [u8; 3] = [88, 92, 70];
const GLOVES: [u8; 3] = [60, 57, 54];
const PLAIN: [u8; 3] = [255; 3];
/// The armor layers, by outfit (none, scrap, kevlar, power).
const ARMOR: [Option<&str>; 4] = [
    None,
    Some("torso/armour/leather/male"),
    Some("torso/chainmail/male"),
    Some("torso/armour/plate/male"),
];
/// The layers that make up the legs.
const LEGS: [(&str, [u8; 3]); 2] = [
    ("legs/pants/male/{a}.png", PANTS),
    ("feet/boots/basic/male/{a}.png", BOOTS),
];
/// The hip-held gun pose is the spear thrust's frame 3; legs below this line keep walking.
const HOLD_FRAME: i32 = 3;
const WAIST: i32 = 47;

/// Loads layer sheets once each.
struct Layers(std::collections::HashMap<String, Rgba>);

impl Layers {
    fn get(&mut self, path: &str) -> &Rgba {
        self.0
            .entry(path.to_string())
            .or_insert_with(|| read(&assets().join("lpc-src").join(path)))
    }

    /// One `CELL` frame of the layers in `paths` (drawn in order), from animation `anim`.
    fn frame(&mut self, paths: &[(String, [u8; 3])], anim: &str, row: u32, col: i32) -> Rgba {
        let mut out = Rgba::new(CELL, CELL);
        for (path, tint) in paths {
            let sheet = self.get(&path.replace("{a}", anim));
            // The fall is drawn once, facing the camera.
            let src_row = if sheet.h == CELL { 0 } else { row as i32 };
            let c = CELL as i32;
            out.blit(sheet, col * c, src_row * c, c, c, 0, 0, *tint);
        }
        out
    }
}

/// Raiders: darker, blood-red cloth over leather, the bandana baked in.
const RAIDER_SHIRT: [u8; 3] = [92, 52, 44];
const RAIDER_PANTS: [u8; 3] = [64, 56, 48];
const RAIDER_BANDANA: [u8; 3] = [168, 38, 30];

fn raider_layers(row: u32) -> Vec<(String, [u8; 3])> {
    let mut layers = body_layers(1, row);
    for (path, tint) in &mut layers {
        if path.starts_with("torso/clothes") {
            *tint = RAIDER_SHIRT;
        } else if path.starts_with("legs/") {
            *tint = RAIDER_PANTS;
        }
    }
    let bandana = (
        "hat/cloth/bandana/adult/{a}.png".to_string(),
        RAIDER_BANDANA,
    );
    // Over the head but under the backpack when it covers the body.
    let at = layers.len() - usize::from(row == 0);
    layers.insert(at, bandana);
    layers
}

/// Body layers back to front; facing away, the backpack covers the body instead of hiding behind.
fn body_layers(outfit: usize, row: u32) -> Vec<(String, [u8; 3])> {
    let pack = ("backpack/squarepack/male/{a}/forest.png".to_string(), PLAIN);
    let mut layers = vec![
        ("body/bodies/male/{a}.png".to_string(), PLAIN),
        ("head/heads/human/male/{a}.png".to_string(), PLAIN),
        ("legs/pants/male/{a}.png".to_string(), PANTS),
        ("feet/boots/basic/male/{a}.png".to_string(), BOOTS),
        (
            "torso/clothes/longsleeve/longsleeve/male/{a}.png".to_string(),
            SHIRT,
        ),
    ];
    if let Some(armor) = ARMOR[outfit] {
        layers.push((format!("{armor}/{{a}}.png"), PLAIN));
    }
    layers.push(("arms/hands/gloves/male/{a}.png".to_string(), GLOVES));
    layers.push((
        "facial/masks/plain/adult/{a}/charcoal.png".to_string(),
        PLAIN,
    ));
    if row == 0 {
        layers.push(pack);
    } else {
        layers.insert(0, pack);
    }
    layers
}

/// A whole body-layout sheet (see `COLUMNS`) of the layers `layers(row)` gives.
fn character_sheet(src: &mut Layers, layers: impl Fn(u32) -> Vec<(String, [u8; 3])>) -> Rgba {
    let mut sheet = Rgba::new(COLUMNS * CELL, ROWS * CELL);
    let c = CELL as i32;
    for row in 0..ROWS {
        let paths = layers(row);
        let put = |sheet: &mut Rgba, frame: &Rgba, col: u32| {
            sheet.blit(frame, 0, 0, c, c, col as i32 * c, row as i32 * c, PLAIN);
        };
        for k in 0..=STRIDE_FRAMES as u32 {
            let walk = src.frame(&paths, "walk", row, k as i32);
            put(&mut sheet, &walk, WALK + k);
            // Upper body from the hip-held pose, legs still walking. Below the waist only the
            // trousers and boots come from the walk: its swinging hands would hang at the sides.
            let legs = src.frame(
                &LEGS.map(|(p, t)| (p.to_string(), t)),
                "walk",
                row,
                k as i32,
            );
            let mut hold = src.frame(&paths, "thrust", row, HOLD_FRAME);
            for y in WAIST..c {
                for x in 0..c {
                    hold.px[(y * c + x) as usize] = if legs.get(x, y)[3] > 0 {
                        walk.get(x, y)
                    } else {
                        [0; 4]
                    };
                }
            }
            put(&mut sheet, &hold, HOLD + k);
        }
        for k in 0..SLASH_FRAMES {
            put(
                &mut sheet,
                &src.frame(&paths, "slash", row, k as i32),
                SLASH + k,
            );
        }
        for k in 0..HURT_FRAMES {
            put(
                &mut sheet,
                &src.frame(&paths, "hurt", row, k as i32),
                HURT + k,
            );
        }
    }
    sheet
}

/// The club sheet from one set of mace layers (`walk`, `slash`, `hurt` sheet paths).
fn club_sheet(src: &mut Layers, walk: &str, slash: &str, hurt: &str) -> Rgba {
    let mut sheet = Rgba::new(CLUB_COLUMNS * CLUB_CELL, ROWS * CLUB_CELL);
    let (c, cc) = (CELL as i32, CLUB_CELL as i32);
    // Body-sized frames sit in the middle of a club cell; the swing's frames are cropped to it.
    let inset = (cc - c) / 2;
    for row in 0..ROWS as i32 {
        let walk = src.get(walk).clone();
        for k in 0..=STRIDE_FRAMES as i32 {
            let dx = (CLUB_WALK as i32 + k) * cc + inset;
            sheet.blit(&walk, k * c, row * c, c, c, dx, row * cc + inset, PLAIN);
        }
        let slash = src.get(slash).clone();
        let big = slash.h as i32 / ROWS as i32;
        let crop = (big - cc) / 2;
        for k in 0..SLASH_FRAMES as i32 {
            let (sx, sy) = (k * big + crop, row * big + crop);
            let dx = (CLUB_SLASH as i32 + k) * cc;
            sheet.blit(&slash, sx, sy, cc, cc, dx, row * cc, PLAIN);
        }
        let hurt = src.get(hurt).clone();
        for k in 0..HURT_FRAMES as i32 {
            let dx = (CLUB_HURT as i32 + k) * cc + inset;
            sheet.blit(&hurt, k * c, 0, c, c, dx, row * cc + inset, PLAIN);
        }
    }
    sheet
}

/// A 3x3 patch of a terrain's fill tiles. The source strip holds four 32 px variants: plain,
/// speckled, plain, and a dark blotch that would look busy if it came up on every tile.
fn ground(fill: &Rgba) -> Rgba {
    const PATCH: [[i32; 3]; 3] = [[0, 1, 2], [2, 3, 0], [1, 0, 2]];
    let mut out = Rgba::new(96, 96);
    for (ty, row) in PATCH.iter().enumerate() {
        for (tx, v) in row.iter().enumerate() {
            out.blit(
                fill,
                v * 32,
                0,
                32,
                32,
                tx as i32 * 32,
                ty as i32 * 32,
                PLAIN,
            );
        }
    }
    out
}

/// A wall face `height` px tall from a 4x3 wall block: two middle columns, keeping the top
/// trim and the baseboard and dropping the middle of the wall.
fn wall_face(block: &Rgba, height: i32) -> Rgba {
    let top = height * 3 / 10;
    let bottom = height - top;
    let mut out = Rgba::new(64, height as u32);
    out.blit(block, 32, 0, 64, top, 0, 0, PLAIN);
    out.blit(
        block,
        32,
        block.h as i32 - bottom,
        64,
        bottom,
        0,
        top,
        PLAIN,
    );
    out
}

fn crop(src: &Rgba, x: i32, y: i32, w: i32, h: i32) -> Rgba {
    let mut out = Rgba::new(w as u32, h as u32);
    out.blit(src, x, y, w, h, 0, 0, PLAIN);
    out
}

/// Wolves: the side views face right (walk row 3, run row 4) and left (rows 9, 10) in 64x32
/// frames on the right half; front and back views are 32x64 frames on the left half (walk row
/// 3, run row 4: five facing the camera, then five facing away).
fn wolf_sheet(src: &Rgba) -> Rgba {
    let c = CELL as i32;
    let mut out = Rgba::new(WOLF_COLUMNS * CELL, ROWS * CELL);
    for (col0, side_rows, upright_row) in [(WOLF_WALK, (3, 9), 3), (WOLF_RUN, (4, 10), 4)] {
        for k in 0..WOLF_FRAMES as i32 {
            let x = (col0 as i32 + k) * c;
            // Up (facing away) and down (facing the camera): centered, feet at the bottom.
            out.blit(src, (5 + k) * 32, upright_row * c, 32, c, x + 16, 0, PLAIN);
            out.blit(src, k * 32, upright_row * c, 32, c, x + 16, 2 * c, PLAIN);
            // Left and right: standing on the bottom of the frame.
            let (right, left) = side_rows;
            out.blit(src, 320 + k * c, left * 32, c, 32, x, c + 32, PLAIN);
            out.blit(src, 320 + k * c, right * 32, c, 32, x, 3 * c + 32, PLAIN);
        }
    }
    out
}

/// Ghouls: the zombie body and head in trousers and grubby bandages, walking.
fn ghoul_sheet(src: &mut Layers) -> Rgba {
    let layers = [
        ("body/bodies/zombie/walk/zombie.png", PLAIN),
        ("head/heads/zombie/adult/walk.png", PLAIN),
        ("legs/pants/male/walk.png", [70, 64, 56]),
        ("torso/bandage/male/walk/white.png", [170, 150, 110]),
    ];
    let c = CELL as i32;
    let mut out = Rgba::new(GHOUL_COLUMNS * CELL, ROWS * CELL);
    for (path, tint) in layers {
        let sheet = src.get(path).clone();
        out.blit(
            &sheet,
            0,
            0,
            GHOUL_COLUMNS as i32 * c,
            ROWS as i32 * c,
            0,
            0,
            tint,
        );
    }
    out
}

fn bake_creatures(src: &mut Layers, out: &Path) {
    let file = |dir: &str, name: &str| read(&assets().join("lpc-src").join(dir).join(name));
    for (name, target) in ["wolf-brown.png", "wolf-grey.png"]
        .into_iter()
        .zip(WOLF_FILES)
    {
        write(&wolf_sheet(&file("creatures", name)), &out.join(target));
    }
    write(&ghoul_sheet(src), &out.join(GHOUL_FILE));
    write(&file("creatures", "slime.png"), &out.join(SLIME_FILE));
    write(&file("creatures", "golem-walk.png"), &out.join(GOLEM_FILE));
    for (name, target) in [
        ("crate.png", CRATE_FILE),
        ("barrel.png", BARREL_FILE),
        ("safe.png", SAFE_FILE),
        ("safe-open.png", SAFE_OPEN_FILE),
    ] {
        write(&file("props", name), &out.join(target));
    }
}

fn bake_map(out: &Path) {
    let src = |name: &str| read(&assets().join("lpc-src/map").join(name));
    write(&ground(&src("ground-dirt.png")), &out.join(GROUND_DIRT));
    write(&src("ground-gravel-blob.png"), &out.join(GRAVEL_PATCH));
    // The sidewalk's plain flagstones (2x2 tiles) from the Modern Streets terrain.
    write(
        &crop(&src("sidewalk_terrain.png"), 192, 160, 64, 64),
        &out.join(SIDEWALK),
    );
    // Asphalt with a couple of cracks in a 4x4 patch.
    let base = src("base_road_tile.png");
    let cracks = src("cracks_transparent.png");
    let mut road = Rgba::new(128, 128);
    for y in (0..128).step_by(32) {
        for x in (0..128).step_by(32) {
            road.blit(&base, 0, 0, 32, 32, x, y, PLAIN);
        }
    }
    road.blit(&cracks, 160, 0, 32, 32, 0, 32, PLAIN);
    road.blit(&cracks, 160, 32, 32, 32, 64, 96, PLAIN);
    write(&road, &out.join(ROAD));
    for ((file, block), height) in WALL_FACES
        .iter()
        .zip(["wall-brick.png", "wall-corrugated.png", "wall-cobble.png"])
        .zip(WALL_FACE_HEIGHTS)
    {
        write(&wall_face(&src(block), height as i32), &out.join(file));
    }
    let tar = src("roof-tar.png");
    write(&crop(&tar, 48, 40, 32, 32), &out.join(WALL_TOP));
    write(&tar, &out.join(ROOF_TAR));
    write(&src("roof-concrete.png"), &out.join(ROOF_CONCRETE));
}

#[test]
#[ignore]
fn bake_lpc_sheets() {
    let mut src = Layers(Default::default());
    let out = assets();
    for (outfit, file) in BODY_FILES.iter().enumerate() {
        let sheet = character_sheet(&mut src, |row| body_layers(outfit, row));
        write(&sheet, &out.join(file));
    }
    let team = character_sheet(&mut src, |_| {
        vec![("hat/cloth/bandana/adult/{a}.png".to_string(), PLAIN)]
    });
    write(&team, &out.join(TEAM_FILE));
    write(
        &character_sheet(&mut src, raider_layers),
        &out.join(RAIDER_FILE),
    );
    let mace = "weapon/blunt/mace";
    let front = club_sheet(
        &mut src,
        &format!("{mace}/walk/mace.png"),
        &format!("{mace}/attack_slash/mace.png"),
        &format!("{mace}/hurt/mace.png"),
    );
    write(&front, &out.join(CLUB_FILE));
    let behind = club_sheet(
        &mut src,
        &format!("{mace}/universal_behind/walk/mace.png"),
        &format!("{mace}/attack_slash/behind/mace.png"),
        &format!("{mace}/universal_behind/hurt/mace.png"),
    );
    write(&behind, &out.join(CLUB_BEHIND_FILE));
    bake_creatures(&mut src, &out);
    bake_map(&out);
}
