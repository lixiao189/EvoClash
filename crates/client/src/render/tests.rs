use super::*;

fn area(runs: &[WallRun]) -> f32 {
    runs.iter().map(|r| (r.x1 - r.x0) * (r.y1 - r.y0)).sum()
}

#[test]
fn overlapping_corner_becomes_disjoint_runs() {
    // An L: a horizontal bar whose end overlaps a vertical one.
    let walls = [
        Rect::new(-20.0, 0.0, 100.0, 10.0),
        Rect::new(-30.0, -100.0, -10.0, 5.0),
    ];
    let runs = wall_runs(&walls);
    // Union area: 120*10 + 20*105 - overlap(10*5).
    assert!(
        (area(&runs) - (1200.0 + 2100.0 - 50.0)).abs() < 0.1,
        "{runs:?}"
    );
    for (i, a) in runs.iter().enumerate() {
        for b in &runs[i + 1..] {
            let overlap_x = a.x0 < b.x1 - EPS && b.x0 < a.x1 - EPS;
            let overlap_y = a.y0 < b.y1 - EPS && b.y0 < a.y1 - EPS;
            assert!(!(overlap_x && overlap_y), "{a:?} overlaps {b:?}");
        }
    }
}

#[test]
fn separate_walls_stay_whole() {
    let walls = [
        Rect::new(0.0, 0.0, 50.0, 10.0),
        Rect::new(100.0, 0.0, 150.0, 10.0),
    ];
    assert_eq!(wall_runs(&walls).len(), 2);
}
