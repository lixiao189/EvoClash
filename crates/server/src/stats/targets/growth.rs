use super::*;

pub(super) fn rows(done: &[&MatchRecord]) -> Vec<TargetRow> {
    let mut rows = Vec::with_capacity(8);
    rows.push(med_of_med(
        39,
        "终局等级中位数",
        "soft",
        "11-13",
        done,
        |m| m.players.iter().map(|p| p.level as f64).collect(),
        |v| (11.0..=13.0).contains(v),
        "级",
    ));
    rows.push(med_of_med(
        40,
        "每条命废料",
        "soft",
        "45-70",
        done,
        |m| m.lives.iter().map(|l| l.scrap as f64).collect(),
        |v| (45.0..=70.0).contains(v),
        "",
    ));
    rows.push(paths(done));
    rows
}

fn paths(done: &[&MatchRecord]) -> TargetRow {
    let mut sums = [0.0; 3];
    let mut ns = [0u32; 3];
    for m in done {
        for p in &m.players {
            let i = match p.path.as_str() {
                "变异体" => 0,
                "改造人" => 1,
                "拾荒者" => 2,
                _ => continue,
            };
            sums[i] += f64::from(p.rank);
            ns[i] += 1;
        }
    }
    if ns.contains(&0) {
        return measured(
            41,
            "三路线平均名次差",
            "hard",
            "≤ 0.5 名",
            None,
            None,
            Some("有路线没人点".into()),
        );
    }
    let means: Vec<f64> = (0..3).map(|i| sums[i] / ns[i] as f64).collect();
    let mut gap: f64 = 0.0;
    for a in 0..3 {
        for b in a + 1..3 {
            gap = gap.max((means[a] - means[b]).abs());
        }
    }
    measured(
        41,
        "三路线平均名次差",
        "hard",
        "≤ 0.5 名",
        Some(json_num(gap)),
        Some(gap <= 0.5),
        None,
    )
}
