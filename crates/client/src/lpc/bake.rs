//! Builds the sheets in `assets/lpc/` from the generator layers vendored in `assets/lpc-src/`:
//! `cargo test -p evoclash -- --ignored bake_lpc_sheets`. Rerun it after changing a layer, a
//! tint or the sheet layout, and keep `assets/CREDITS.md` in step.

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
}
