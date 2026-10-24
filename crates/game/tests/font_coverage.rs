//! Every Chinese string the game shows must render with the bundled UI font (Noto Sans SC,
//! subset to GB2312 plus the sources' characters, see crates/client/assets/README.md).
//! This reads the font's cmap and checks every non-ASCII character in the workspace sources,
//! so a new string outside the subset fails here instead of drawing as tofu.

use std::path::Path;

use evoclash_game::protocol::{Monster, MonsterKind};

const FONT: &[u8] = include_bytes!("../../client/assets/NotoSansSC-Subset.ttf");

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_be_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Codepoint -> has a glyph, from a format 12 or format 4 Unicode cmap subtable.
struct Cmap<'a> {
    table: &'a [u8],
    format: u16,
}

impl<'a> Cmap<'a> {
    fn load(font: &'a [u8]) -> Self {
        let tables = u16_at(font, 4) as usize;
        let cmap = (0..tables)
            .map(|i| 12 + 16 * i)
            .find(|&rec| &font[rec..rec + 4] == b"cmap")
            .map(|rec| u32_at(font, rec + 8) as usize)
            .expect("font has no cmap");
        let subtables = u16_at(font, cmap + 2) as usize;
        let mut best: Option<(u16, usize)> = None;
        for i in 0..subtables {
            let rec = cmap + 4 + 8 * i;
            let (platform, encoding) = (u16_at(font, rec), u16_at(font, rec + 2));
            let offset = cmap + u32_at(font, rec + 4) as usize;
            let format = u16_at(font, offset);
            let unicode = platform == 0 || (platform == 3 && matches!(encoding, 1 | 10));
            if unicode && (format == 12 || (format == 4 && best.is_none())) {
                best = Some((format, offset));
            }
        }
        let (format, offset) = best.expect("font has no unicode cmap");
        Self {
            table: &font[offset..],
            format,
        }
    }

    fn has(&self, c: char) -> bool {
        let cp = c as u32;
        let t = self.table;
        if self.format == 12 {
            let groups = u32_at(t, 12) as usize;
            return (0..groups).any(|g| {
                let at = 16 + 12 * g;
                (u32_at(t, at)..=u32_at(t, at + 4)).contains(&cp)
            });
        }
        if cp > 0xffff {
            return false;
        }
        let cp = cp as u16;
        let segs = u16_at(t, 6) as usize / 2;
        let ends = 14;
        let starts = ends + 2 * segs + 2;
        let deltas = starts + 2 * segs;
        let ranges = deltas + 2 * segs;
        for s in 0..segs {
            if cp > u16_at(t, ends + 2 * s) {
                continue;
            }
            let start = u16_at(t, starts + 2 * s);
            if cp < start {
                return false;
            }
            let delta = u16_at(t, deltas + 2 * s);
            let range = u16_at(t, ranges + 2 * s) as usize;
            let glyph = if range == 0 {
                cp.wrapping_add(delta)
            } else {
                let at = ranges + 2 * s + range + 2 * (cp - start) as usize;
                match u16_at(t, at) {
                    0 => 0,
                    g => g.wrapping_add(delta),
                }
            };
            return glyph != 0;
        }
        false
    }
}

fn missing(cmap: &Cmap, text: &str) -> Vec<char> {
    text.chars()
        .filter(|c| !c.is_ascii() && !c.is_whitespace() && !cmap.has(*c))
        .collect()
}

#[test]
fn monster_names_render() {
    let cmap = Cmap::load(FONT);
    assert!(cmap.has('辐') && !cmap.has('\u{1F600}') && !cmap.has('\u{9F98}'));
    for kind in MonsterKind::ALL {
        for elite in [false, true] {
            let name = Monster { kind, elite }.display_name();
            assert!(missing(&cmap, &name).is_empty(), "{name}");
        }
    }
}

fn scan(dir: &Path, cmap: &Cmap, bad: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(&path, cmap, bad);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let miss = missing(cmap, line);
                if !miss.is_empty() {
                    bad.push(format!("{}:{}: {miss:?}", path.display(), n + 1));
                }
            }
        }
    }
}

#[test]
fn every_source_character_renders() {
    let cmap = Cmap::load(FONT);
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut bad = Vec::new();
    for krate in ["game", "server", "client"] {
        scan(&crates.join(krate).join("src"), &cmap, &mut bad);
    }
    assert!(
        bad.is_empty(),
        "characters missing from the UI font:\n{}",
        bad.join("\n")
    );
}
