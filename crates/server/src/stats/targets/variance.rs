use super::*;

pub(super) fn rows(done: &[&MatchRecord], repeat: Option<&[String]>) -> Vec<TargetRow> {
    let mut rows = Vec::with_capacity(8);
    rows.push(high_win(done));
    rows.push(high_rank(done));
    rows.push(repeat_row(repeat));
    rows.push(rate_row(
        30,
        "2:00 装备最高≠分数第一",
        "soft",
        "≥ 40%",
        done.iter().filter(|m| gear_not_leader(m)).count() as u32,
        done.iter().filter(|m| m.snap_2m.is_some()).count() as u32,
        |v| *v >= 40.0,
    ));
    rows.push(sourced_row(done));
    rows.push(skip(32, "真人同房三连冠", "soft", "≤ 10%", "需要真人"));
    rows
}

fn gear_not_leader(m: &MatchRecord) -> bool {
    let Some(rows) = m.snap_2m.as_deref() else {
        return false;
    };
    let Some(lead) = super::snowball::leader_id(rows) else {
        return false;
    };
    let lead_gear = rows
        .iter()
        .find(|r| r.client_id == lead)
        .map(|r| r.gear)
        .unwrap_or(0);
    let best = rows.iter().map(|r| r.gear).max().unwrap_or(0);
    lead_gear < best
}

fn one_high(m: &MatchRecord) -> bool {
    let highs = m.players.iter().filter(|p| p.skill == "high").count();
    let novice = m.players.iter().any(|p| p.skill == "novice");
    highs == 1 && !novice && m.players.len() == 8
}

fn high_win(done: &[&MatchRecord]) -> TargetRow {
    let rows: Vec<_> = done.iter().copied().filter(|m| one_high(m)).collect();
    if rows.is_empty() {
        return skip(
            27,
            "高手夺冠率",
            "hard",
            "25-40%",
            "需要 --skills high:1,default:7",
        );
    }
    let wins = rows
        .iter()
        .filter(|m| {
            m.winner_id.is_some_and(|id| {
                m.players
                    .iter()
                    .any(|p| p.client_id == id && p.skill == "high")
            })
        })
        .count() as u32;
    rate_row(
        27,
        "高手夺冠率",
        "hard",
        "25-40%",
        wins,
        rows.len() as u32,
        |v| (25.0..=40.0).contains(v),
    )
}

fn high_rank(done: &[&MatchRecord]) -> TargetRow {
    let ranks: Vec<f64> = done
        .iter()
        .filter(|m| one_high(m))
        .filter_map(|m| {
            m.players
                .iter()
                .find(|p| p.skill == "high")
                .map(|p| f64::from(p.rank))
        })
        .collect();
    if ranks.is_empty() {
        return skip(
            28,
            "高手平均名次",
            "soft",
            "2.0-3.0",
            "需要 --skills high:1,default:7",
        );
    }
    let v = ranks.iter().sum::<f64>() / ranks.len() as f64;
    measured(
        28,
        "高手平均名次",
        "soft",
        "2.0-3.0",
        Some(json_num(v)),
        Some((2.0..=3.0).contains(&v)),
        None,
    )
}

fn repeat_row(repeat: Option<&[String]>) -> TargetRow {
    let Some(names) = repeat else {
        return skip(
            29,
            "同房 10 局不同冠军",
            "soft",
            "≥ 5 人",
            "需要 8 个默认档同房连打 10 局",
        );
    };
    if names.len() < 10 {
        return skip(29, "同房 10 局不同冠军", "soft", "≥ 5 人", "连打不足 10 局");
    }
    let mut uniq: Vec<&str> = names.iter().map(String::as_str).collect();
    uniq.sort_unstable();
    uniq.dedup();
    let n = uniq.len() as f64;
    measured(
        29,
        "同房 10 局不同冠军",
        "soft",
        "≥ 5 人",
        Some(json_num(n)),
        Some(n >= 5.0),
        None,
    )
}

fn sourced_row(done: &[&MatchRecord]) -> TargetRow {
    let mut num = 0u32;
    let mut den = 0u32;
    for m in done {
        let Some(id) = m.winner_id else {
            continue;
        };
        for k in &m.kills {
            if k.killer_id == Some(id) && k.cause == "player" {
                den += 1;
                if k.sourced {
                    num += 1;
                }
            }
        }
    }
    rate_row(
        31,
        "冠军击杀来自地堡/巨兽/保险柜",
        "soft",
        "25-50%",
        num,
        den,
        |v| (25.0..=50.0).contains(v),
    )
}
