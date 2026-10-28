use super::*;

pub(super) fn rows(done: &[&MatchRecord]) -> Vec<TargetRow> {
    let mut rows = Vec::with_capacity(8);
    rows.push(med_of_med(
        7,
        "每条命开箱+保险柜",
        "soft",
        "5–10",
        done,
        |m| m.lives.iter().map(|l| l.opens as f64).collect(),
        |v| (5.0..=10.0).contains(v),
        "次",
    ));
    let (t1, t2) = tier_times(done);
    rows.push(pair_row(
        8,
        "首次 T1 / T2",
        "soft",
        "T1 ≤ 45 秒，T2 ≤ 2:30",
        t1,
        t2,
        |a, b| a <= 45.0 && b <= 150.0,
    ));
    rows.push(med_row(
        9,
        "终局内圈占比",
        "soft",
        "≥ 35%",
        &mut done
            .iter()
            .filter(|m| m.finale_alive > 0)
            .map(|m| 100.0 * m.finale_inner as f64 / m.finale_alive as f64)
            .collect::<Vec<_>>(),
        |v| *v >= 35.0,
        "%",
    ));
    rows.push(rate_row(
        10,
        "地堡 20 秒内拾取 T4",
        "soft",
        "≥ 90%",
        done.iter().map(|m| m.vault_t4_hits).sum(),
        done.iter().map(|m| m.vault_opens).sum(),
        |v| *v >= 90.0,
    ));
    rows.push(med_row(
        11,
        "T4 持有时长",
        "hard",
        "40–90 秒",
        &mut done
            .iter()
            .flat_map(|m| m.t4_hold_ticks.iter().map(|t| secs(*t)))
            .collect::<Vec<_>>(),
        |v| (40.0..=90.0).contains(v),
        "秒",
    ));
    rows.push(rate_row(
        12,
        "冠军拿到过 T4",
        "soft",
        "50–80%",
        done.iter()
            .filter(|m| {
                m.winner_id
                    .is_some_and(|id| m.players.iter().any(|p| p.client_id == id && p.had_t4))
            })
            .count() as u32,
        done.len() as u32,
        |v| (50.0..=80.0).contains(v),
    ));
    rows
}

fn tier_times(done: &[&MatchRecord]) -> (Option<f64>, Option<f64>) {
    let mut t1: Vec<f64> = done
        .iter()
        .flat_map(|m| m.lives.iter().filter_map(|l| l.first_t1))
        .map(|t| t as f64 / TICKS_PER_SEC as f64)
        .collect();
    let mut t2: Vec<f64> = done
        .iter()
        .flat_map(|m| m.lives.iter().filter_map(|l| l.first_t2))
        .map(|t| t as f64 / TICKS_PER_SEC as f64)
        .collect();
    (median(&mut t1), median(&mut t2))
}
