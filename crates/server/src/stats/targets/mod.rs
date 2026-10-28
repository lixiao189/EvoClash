//! Turn per-match records into the numbered rows of `doc/design-targets.md`.

mod combat;
mod fauna;
mod growth;
mod pace;
mod scrape;
mod snowball;
mod variance;

use super::MatchRecord;
use crate::gameplay::TICKS_PER_SEC;

#[derive(Clone, Debug, serde::Serialize)]
pub struct TargetRow {
    pub id: u8,
    pub name: String,
    pub level: &'static str,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub(super) fn secs(ticks: u32) -> f64 {
    ticks as f64 / TICKS_PER_SEC as f64
}

pub fn targets(matches: &[MatchRecord], repeat: Option<&[String]>) -> Vec<TargetRow> {
    let done: Vec<&MatchRecord> = matches.iter().filter(|m| m.finished).collect();
    let mut rows = Vec::with_capacity(41);
    rows.extend(pace::rows(&done));
    rows.extend(scrape::rows(&done));
    rows.extend(combat::rows(&done));
    rows.extend(snowball::rows(&done));
    rows.extend(variance::rows(&done, repeat));
    rows.extend(fauna::rows(&done));
    rows.extend(growth::rows(&done));
    rows
}

pub fn print_table(rows: &[TargetRow]) {
    println!("#   指标                      实测        目标              级别  结果");
    for row in rows {
        if row.status != "measured" {
            continue;
        }
        let value = row.value.as_ref().map(brief).unwrap_or_else(|| "-".into());
        let mark = match row.ok {
            Some(true) => "达标",
            Some(false) => "出界",
            None => "-",
        };
        println!(
            "{:<3} {:<24} {:<12} {:<16} {:<4} {mark}",
            row.id, row.name, value, row.target, row.level
        );
    }
    println!("未测:");
    for row in rows.iter().filter(|r| r.status != "measured") {
        println!(
            "  #{} {} — {}",
            row.id,
            row.name,
            row.reason.as_deref().unwrap_or("样本不足")
        );
    }
}

pub(super) fn brief(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            if (f - f.round()).abs() < 1e-6 && f.abs() >= 10.0 {
                format!("{f:.0}")
            } else {
                format!("{f:.2}")
            }
        }
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub(super) fn json_num(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64(v)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

pub(super) fn measured(
    id: u8,
    name: &str,
    level: &'static str,
    target: &str,
    value: Option<serde_json::Value>,
    ok: Option<bool>,
    reason: Option<String>,
) -> TargetRow {
    let status = if value.is_some() {
        "measured"
    } else {
        "not_measured"
    };
    TargetRow {
        id,
        name: name.into(),
        level,
        status,
        value,
        target: target.into(),
        ok: if status == "measured" { ok } else { None },
        reason: if status == "measured" {
            reason.filter(|_| false)
        } else {
            Some(reason.unwrap_or_else(|| "样本不足".into()))
        },
    }
}

pub(super) fn skip(id: u8, name: &str, level: &'static str, target: &str, why: &str) -> TargetRow {
    TargetRow {
        id,
        name: name.into(),
        level,
        status: "not_measured",
        value: None,
        target: target.into(),
        ok: None,
        reason: Some(why.into()),
    }
}

pub(super) fn med_row(
    id: u8,
    name: &str,
    level: &'static str,
    target: &str,
    xs: &mut [f64],
    ok: impl Fn(&f64) -> bool,
    _unit: &str,
) -> TargetRow {
    match median(xs) {
        Some(v) => measured(
            id,
            name,
            level,
            target,
            Some(json_num(v)),
            Some(ok(&v)),
            None,
        ),
        None => measured(id, name, level, target, None, None, None),
    }
}

pub(super) fn p90_row(
    id: u8,
    name: &str,
    level: &'static str,
    target: &str,
    xs: &mut [f64],
    ok: impl Fn(&f64) -> bool,
    _unit: &str,
) -> TargetRow {
    match percentile(xs, 0.9) {
        Some(v) => measured(
            id,
            name,
            level,
            target,
            Some(json_num(v)),
            Some(ok(&v)),
            None,
        ),
        None => measured(id, name, level, target, None, None, None),
    }
}

pub(super) fn med_of_med(
    id: u8,
    name: &str,
    level: &'static str,
    target: &str,
    done: &[&MatchRecord],
    per: impl Fn(&MatchRecord) -> Vec<f64>,
    ok: impl Fn(&f64) -> bool,
    unit: &str,
) -> TargetRow {
    let mut xs: Vec<f64> = done
        .iter()
        .filter_map(|m| {
            let mut v = per(m);
            median(&mut v)
        })
        .collect();
    med_row(id, name, level, target, &mut xs, ok, unit)
}

pub(super) fn rate_row(
    id: u8,
    name: &str,
    level: &'static str,
    target: &str,
    num: u32,
    den: u32,
    ok: impl Fn(&f64) -> bool,
) -> TargetRow {
    if den == 0 {
        return measured(id, name, level, target, None, None, None);
    }
    let v = 100.0 * num as f64 / den as f64;
    measured(
        id,
        name,
        level,
        target,
        Some(json_num(v)),
        Some(ok(&v)),
        None,
    )
}

pub(super) fn pair_row(
    id: u8,
    name: &str,
    level: &'static str,
    target: &str,
    t1: Option<f64>,
    t2: Option<f64>,
    ok: impl Fn(f64, f64) -> bool,
) -> TargetRow {
    match (t1, t2) {
        (Some(a), Some(b)) => measured(
            id,
            name,
            level,
            target,
            Some(serde_json::json!({"t1": a, "t2": b})),
            Some(ok(a, b)),
            None,
        ),
        _ => measured(id, name, level, target, None, None, None),
    }
}

pub(super) fn player_kills(m: &MatchRecord) -> u32 {
    m.kills.iter().filter(|k| k.cause == "player").count() as u32
}

pub(super) fn median(xs: &mut [f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(f64::total_cmp);
    let n = xs.len();
    if n.is_multiple_of(2) {
        Some((xs[n / 2 - 1] + xs[n / 2]) / 2.0)
    } else {
        Some(xs[n / 2])
    }
}

pub(super) fn percentile(xs: &mut [f64], p: f64) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(f64::total_cmp);
    let i = (xs.len() as f64 * p).ceil() as usize - 1;
    Some(xs[i.min(xs.len() - 1)])
}
