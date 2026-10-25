//! Every sprite file in `assets/lpc/` (shipped) and `assets/lpc-src/` (the layers they are baked
//! from) must be credited in `assets/CREDITS.md`: the art is CC-BY-SA and friends, so a file
//! nobody is credited for breaks its license.

use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn every_art_file_is_credited() {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let credits = std::fs::read_to_string(assets.join("CREDITS.md")).unwrap();
    assert!(
        assets.join("LICENSE-ART.md").exists(),
        "assets/LICENSE-ART.md is missing"
    );
    // The shipped sheets and the layers they are baked from.
    let mut shipped = Vec::new();
    files(&assets.join("lpc"), &mut shipped);
    files(&assets.join("lpc-src"), &mut shipped);
    let missing: Vec<String> = shipped
        .iter()
        .map(|p| {
            p.strip_prefix(&assets)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .filter(|p| p != "lpc/README.md")
        .filter(|p| !credits.contains(&format!("`{p}`")))
        .collect();
    assert!(
        missing.is_empty(),
        "add these files to assets/CREDITS.md (as `lpc/...` or `lpc-src/...`): {missing:?}"
    );
}
