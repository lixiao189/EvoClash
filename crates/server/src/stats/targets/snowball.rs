use super::super::SnapRow;
use super::*;

#[allow(clippy::vec_init_then_push)]
pub(super) fn rows(done: &[&MatchRecord]) -> Vec<TargetRow> {
    let mut rows = Vec::with_capacity(8);
    rows.push(rate_row(
        19,
        "2:00 第一名最终夺冠",
        "hard",
        "20–35%",
        done.iter()
            .filter(|m| leader_won(m, m.snap_2m.as_deref()))
            .count() as u32,
        done.iter().filter(|m| m.snap_2m.is_some()).count() as u32,
        |v| (20.0..=35.0).contains(v),
    ));
    rows.push(rate_row(
        20,
        "5:00 第一名最终夺冠",
        "soft",
        "35–55%",
        done.iter()
            .filter(|m| leader_won(m, m.snap_5m.as_deref()))
            .count() as u32,
        done.iter().filter(|m| m.snap_5m.is_some()).count() as u32,
        |v| (35.0..=55.0).contains(v),
    ));
    rows.push(rate_row(
        21,
        "6:00 落后 ≥3 分仍夺冠",
        "soft",
        "≥ 20%",
        done.iter().filter(|m| behind_won(m)).count() as u32,
        done.iter().filter(|m| m.snap_6m.is_some()).count() as u32,
        |v| *v >= 20.0,
    ));
    rows.push(med_row(
        22,
        "冠军分÷第4名",
        "soft",
        "≤ 2.0",
        &mut done.iter().filter_map(|m| spread(m)).collect::<Vec<_>>(),
        |v| *v <= 2.0,
        "倍",
    ));
    rows.push(p90_row(
        23,
        "单局长连杀 p90",
        "hard",
        "≤ 6",
        &mut done.iter().map(|m| m.max_streak as f64).collect::<Vec<_>>(),
        |v| *v <= 6.0,
        "杀",
    ));
    rows.push(rate_row(
        24,
        "悬赏后 60 秒内被杀",
        "soft",
        "55–80%",
        done.iter().map(|m| m.bounties_killed_60s).sum(),
        done.iter().map(|m| m.bounties).sum(),
        |v| (55.0..=80.0).contains(v),
    ));
    rows.push(rate_row(
        25,
        "以弱胜强击杀",
        "soft",
        "15–30%",
        done.iter()
            .flat_map(|m| m.kills.iter())
            .filter(|k| k.cause == "player" && k.underdog)
            .count() as u32,
        done.iter().map(|m| player_kills(m)).sum(),
        |v| (15.0..=30.0).contains(v),
    ));
    rows.push(med_row(
        26,
        "终局等级差",
        "soft",
        "≤ 4 级",
        &mut done
            .iter()
            .filter_map(|m| level_gap(m.snap_6m.as_deref().unwrap_or(&[])))
            .collect::<Vec<_>>(),
        |v| *v <= 4.0,
        "级",
    ));
    rows
}

pub(super) fn leader_id(rows: &[SnapRow]) -> Option<u64> {
    rows.iter()
        .max_by(|a, b| {
            super::super::rank_key(a.score, a.kills, a.deaths, a.last_score_tick, a.client_id).cmp(
                &super::super::rank_key(b.score, b.kills, b.deaths, b.last_score_tick, b.client_id),
            )
        })
        .map(|r| r.client_id)
}

fn leader_won(m: &MatchRecord, snap: Option<&[SnapRow]>) -> bool {
    let Some(rows) = snap else {
        return false;
    };
    leader_id(rows).is_some_and(|id| m.winner_id == Some(id))
}

fn behind_won(m: &MatchRecord) -> bool {
    let Some(rows) = m.snap_6m.as_deref() else {
        return false;
    };
    let Some(winner) = m.winner_id else {
        return false;
    };
    let lead = rows.iter().map(|r| r.score).max().unwrap_or(0);
    rows.iter()
        .find(|r| r.client_id == winner)
        .is_some_and(|r| lead.saturating_sub(r.score) >= 3)
}

fn spread(m: &MatchRecord) -> Option<f64> {
    let mut scores: Vec<u32> = m.players.iter().map(|p| p.score).collect();
    scores.sort_unstable_by(|a, b| b.cmp(a));
    let fourth = *scores.get(3)?;
    if fourth == 0 {
        return Some(99.0);
    }
    Some(scores[0] as f64 / fourth as f64)
}

fn level_gap(rows: &[SnapRow]) -> Option<f64> {
    let hi = rows.iter().map(|r| r.level).max()?;
    let lo = rows.iter().map(|r| r.level).min()?;
    Some(f64::from(hi - lo))
}
