use super::bake::assets;
use super::*;

/// Width and height from a PNG header, without decoding it.
fn png_size(file: &str) -> (u32, u32) {
    let bytes = std::fs::read(assets().join(file)).unwrap();
    let be = |i: usize| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap());
    (be(16), be(20))
}

/// The baked files match the layout the renderer slices them with.
#[test]
fn sheets_match_the_layout() {
    for file in BODY_FILES.iter().chain([&TEAM_FILE, &RAIDER_FILE]) {
        assert_eq!(png_size(file), (COLUMNS * CELL, ROWS * CELL), "{file}");
    }
    for file in [CLUB_FILE, CLUB_BEHIND_FILE] {
        assert_eq!(
            png_size(file),
            (CLUB_COLUMNS * CLUB_CELL, ROWS * CLUB_CELL),
            "{file}"
        );
    }
}

#[test]
fn frames_stay_inside_their_sheets() {
    let actions = (0..=STRIDE_FRAMES)
        .map(Action::Walk)
        .chain((0..20).map(|i| Action::Swing {
            since: i as f32 * 0.02,
            total: 0.3,
        }))
        .chain((0..20).map(|i| Action::Fall(i as f32 * 0.1)));
    for action in actions {
        for club in [false, true] {
            let f = frame(action, club);
            assert!(f.body < COLUMNS, "{action:?}");
            assert!(f.club.is_none_or(|c| c < CLUB_COLUMNS), "{action:?}");
        }
    }
    assert_eq!(
        frame(Action::Walk(3), false),
        Frame {
            body: HOLD + 3,
            club: None
        }
    );
    assert_eq!(frame(Action::Fall(10.0), true).body, HURT + HURT_FRAMES - 1);
}

#[test]
fn each_facing_has_its_own_row() {
    let rows = [
        row(Facing::Up, false),
        row(Facing::Side, true),
        row(Facing::Down, true),
        row(Facing::Side, false),
    ];
    assert_eq!(rows, [0, 1, 2, 3]);
    // The gun is held at hip height, mirrored between left and right.
    assert!((gun_grip(1).x + gun_grip(3).x).abs() < 0.5);
    assert!((15.0..25.0).contains(&gun_grip(3).y));
}

/// The map tiles fit what the renderer assumes: wall faces as tall as their walls stand, roofs
/// with room inside their frame, everything a whole number of tiles wide.
#[test]
fn map_tiles_match_the_layout() {
    for (file, height) in WALL_FACES.iter().zip(WALL_FACE_HEIGHTS) {
        assert_eq!(png_size(file).1, height, "{file}");
    }
    for file in [ROOF_TAR, ROOF_CONCRETE] {
        let (w, h) = png_size(file);
        assert!(
            w as f32 > 2.0 * ROOF_BORDER && h as f32 > 2.0 * ROOF_BORDER,
            "{file}"
        );
    }
    let (w, h) = png_size(GRAVEL_PATCH);
    assert!(w as f32 > 2.0 * GROUND_EDGE && h as f32 > 2.0 * GROUND_EDGE);
    for file in [GROUND_DIRT, SIDEWALK, ROAD, WALL_TOP] {
        let (w, h) = png_size(file);
        assert!(w % 32 == 0 && h % 32 == 0, "{file}: {w}x{h}");
    }
}
