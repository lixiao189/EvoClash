use super::*;

pub(super) fn rows(done: &[&MatchRecord]) -> Vec<TargetRow> {
    let mut rows = Vec::with_capacity(8);
    rows.push(scrap_row(done));
    rows.push(rate_row(
        34,
        "野怪/巨兽/环境致死",
        "hard",
        "10–25%",
        done.iter()
            .flat_map(|m| m.kills.iter())
            .filter(|k| k.cause != "player")
            .count() as u32,
        done.iter().map(|m| m.kills.len() as u32).sum(),
        |v| (10.0..=25.0).contains(v),
    ));
    rows.push(ttk_monsters(done));
    rows.push(rate_row(
        36,
        "巨兽被击杀率",
        "soft",
        "60–85%",
        done.iter().filter(|m| m.boss_killed).count() as u32,
        done.iter().filter(|m| m.boss_appeared).count() as u32,
        |v| (60.0..=85.0).contains(v),
    ));
    rows.push(med_row(
        37,
        "巨兽出现到死亡",
        "soft",
        "90–150 秒",
        &mut done
            .iter()
            .filter(|m| m.boss_killed)
            .filter_map(|m| Some(secs(m.boss_end_tick?.saturating_sub(m.boss_appear_tick?))))
            .collect::<Vec<_>>(),
        |v| (90.0..=150.0).contains(v),
        "秒",
    ));
    rows.push(rate_row(
        38,
        "巨兽最高伤害者不是第一名",
        "soft",
        "≥ 55%",
        done.iter()
            .filter(|m| m.boss_top_not_leader == Some(true))
            .count() as u32,
        done.iter()
            .filter(|m| m.boss_top_not_leader.is_some())
            .count() as u32,
        |v| *v >= 55.0,
    ));
    rows
}

fn scrap_row(done: &[&MatchRecord]) -> TargetRow {
    let earned: u32 = done.iter().map(|m| m.scrap_earned).sum();
    let monster: u32 = done
        .iter()
        .map(|m| {
            m.scrap_earned
                .saturating_sub(m.scrap_player.saturating_add(m.scrap_container))
        })
        .sum();
    rate_row(
        33,
        "打怪废料占比",
        "soft",
        "45–65%",
        monster,
        earned,
        |v| (45.0..=65.0).contains(v),
    )
}

fn ttk_monsters(done: &[&MatchRecord]) -> TargetRow {
    let mut normal: Vec<f64> = done
        .iter()
        .flat_map(|m| {
            m.monster_ttk_ticks
                .iter()
                .map(|t| *t as f64 / TICKS_PER_SEC as f64)
        })
        .collect();
    let mut elite: Vec<f64> = done
        .iter()
        .flat_map(|m| {
            m.elite_ttk_ticks
                .iter()
                .map(|t| *t as f64 / TICKS_PER_SEC as f64)
        })
        .collect();
    let n = median(&mut normal);
    let e = median(&mut elite);
    match n {
        Some(n) => {
            let ok = (2.0..=5.0).contains(&n) && e.is_none_or(|e| (4.0..=10.0).contains(&e));
            measured(
                35,
                "普通怪击杀耗时",
                "soft",
                "2–5 秒（精英 ×2）",
                Some(serde_json::json!({"normal": n, "elite": e})),
                Some(ok),
                None,
            )
        }
        None => measured(
            35,
            "普通怪击杀耗时",
            "soft",
            "2–5 秒（精英 ×2）",
            None,
            None,
            None,
        ),
    }
}
