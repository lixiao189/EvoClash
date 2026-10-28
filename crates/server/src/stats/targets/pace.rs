use super::*;

pub(super) fn rows(done: &[&MatchRecord]) -> Vec<TargetRow> {
    let mut rows = Vec::with_capacity(8);
    rows.push(med_row(
        1,
        "首次玩家击杀",
        "hard",
        "45-90 秒",
        &mut done
            .iter()
            .map(|m| {
                m.kills
                    .iter()
                    .filter(|k| k.cause == "player")
                    .map(|k| secs(k.tick))
                    .min_by(f64::total_cmp)
                    .unwrap_or(8.0 * 60.0)
            })
            .collect::<Vec<_>>(),
        |v| (45.0..=90.0).contains(v),
        "秒",
    ));
    rows.push(med_row(
        2,
        "每分钟玩家击杀",
        "hard",
        "2.5-5",
        &mut done
            .iter()
            .map(|m| player_kills(m) as f64 / 8.0)
            .collect::<Vec<_>>(),
        |v| (2.5..=5.0).contains(v),
        "/分",
    ));
    rows.push(med_row(
        3,
        "终局÷前3分钟击杀速率",
        "soft",
        "≥ 1.8",
        &mut done.iter().filter_map(|m| pace(m)).collect::<Vec<_>>(),
        |v| *v >= 1.8,
        "倍",
    ));
    rows.push(med_of_med(
        4,
        "单条命存活",
        "soft",
        "90-180 秒",
        done,
        |m| {
            m.lives
                .iter()
                .map(|l| secs(l.end.saturating_sub(l.start)))
                .collect()
        },
        |v| (90.0..=180.0).contains(v),
        "秒",
    ));
    rows.push(p90_row(
        5,
        "单条命最长无交战 p90",
        "soft",
        "≤ 75 秒",
        &mut done
            .iter()
            .flat_map(|m| m.lives.iter().map(|l| secs(l.quiet_ticks)))
            .collect::<Vec<_>>(),
        |v| *v <= 75.0,
        "秒",
    ));
    rows.push(skip(
        6,
        "真人死亡到再上场",
        "soft",
        "≤ 20 秒；45 秒自动出发 ≤ 10%",
        "需要真人",
    ));
    rows
}

fn pace(m: &MatchRecord) -> Option<f64> {
    let early = m
        .kills
        .iter()
        .filter(|k| k.cause == "player" && k.tick <= 3 * 60 * TICKS_PER_SEC)
        .count() as f64;
    let late = m
        .kills
        .iter()
        .filter(|k| k.cause == "player" && k.tick >= 6 * 60 * TICKS_PER_SEC)
        .count() as f64;
    if early == 0.0 {
        return None;
    }
    Some((late / 2.0) / (early / 3.0))
}
