use evoclash_game::items::WeaponKind;

use super::super::KillRec;
use super::*;

pub(super) fn rows(done: &[&MatchRecord]) -> Vec<TargetRow> {
    let mut rows = Vec::with_capacity(8);
    rows.push(ttk_row());
    rows.push(med_row(
        14,
        "实际交战时长",
        "soft",
        "2–5 秒",
        &mut done
            .iter()
            .flat_map(|m| m.fight_ticks.iter().map(|t| secs(*t)))
            .collect::<Vec<_>>(),
        |v| (2.0..=5.0).contains(v),
        "秒",
    ));
    let (share, top) = weapon_share(done);
    rows.push(measured(
        15,
        "单把武器击杀占比",
        "hard",
        "≤ 30%（不含木棍）",
        share.map(json_num),
        share.map(|v| v <= 30.0),
        top,
    ));
    let stick = stick_share(done);
    rows.push(measured(
        16,
        "2:00 后木棍击杀占比",
        "soft",
        "≤ 15%",
        stick.map(json_num),
        stick.map(|v| v <= 15.0),
        None,
    ));
    rows.push(skip(
        17,
        "高一级 1v1 胜率",
        "hard",
        "65–80%",
        "需要场景对决（下一阶段）",
    ));
    rows.push(skip(
        18,
        "新手高配打高手低配",
        "hard",
        "40–60%",
        "需要场景对决（下一阶段）",
    ));
    rows
}

fn weapon_share(done: &[&MatchRecord]) -> (Option<f64>, Option<String>) {
    let mut counts: BTreeCount = BTreeCount::default();
    let mut total = 0u32;
    for k in done
        .iter()
        .flat_map(|m| m.kills.iter())
        .filter(|k| k.cause == "player")
    {
        let Some(name) = k.weapon.as_deref() else {
            continue;
        };
        if name == "木棍" {
            continue;
        }
        counts.add(name);
        total += 1;
    }
    if total == 0 {
        return (None, None);
    }
    let (name, n) = counts.max();
    (Some(100.0 * n as f64 / total as f64), Some(name))
}

fn stick_share(done: &[&MatchRecord]) -> Option<f64> {
    let late: Vec<&KillRec> = done
        .iter()
        .flat_map(|m| m.kills.iter())
        .filter(|k| k.cause == "player" && k.tick >= 2 * 60 * TICKS_PER_SEC)
        .collect();
    if late.is_empty() {
        return None;
    }
    let stick = late.iter().filter(|k| k.how == "木棍").count();
    Some(100.0 * stick as f64 / late.len() as f64)
}

fn ttk_row() -> TargetRow {
    let guns = [
        WeaponKind::Bow,
        WeaponKind::PipeShotgun,
        WeaponKind::Rifle,
        WeaponKind::Laser,
    ];
    let mut value = serde_json::Map::new();
    let mut ok = true;
    for kind in guns {
        let spec = kind.spec();
        let pellets = kind.ranged().map(|w| w.pellets()).unwrap_or(1).max(1) as f32;
        let dmg = (spec.damage() * pellets).max(1.0);
        let shots = (100.0 / dmg).ceil().max(1.0);
        let ttk = (shots - 1.0) * spec.cooldown() as f32 / TICKS_PER_SEC as f32;
        let (lo, hi) = match spec.tier() {
            1 => (1.0, 1.8),
            2 => (0.8, 1.4),
            _ => (0.6, 1.2),
        };
        if !(lo..=hi).contains(&ttk) {
            ok = false;
        }
        value.insert(
            format!("T{} {}", spec.tier(), spec.name()),
            json_num(ttk as f64),
        );
    }
    TargetRow {
        id: 13,
        name: "同层理论 TTK".into(),
        level: "hard",
        status: "measured",
        value: Some(serde_json::Value::Object(value)),
        target: "T1 1.0–1.8 秒，T2 0.8–1.4 秒，T3 0.6–1.2 秒".into(),
        ok: Some(ok),
        reason: None,
    }
}

#[derive(Default)]
struct BTreeCount(std::collections::BTreeMap<String, u32>);

impl BTreeCount {
    fn add(&mut self, name: &str) {
        *self.0.entry(name.to_string()).or_default() += 1;
    }
    fn max(&self) -> (String, u32) {
        self.0
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(k, v)| (k.clone(), *v))
            .unwrap_or_default()
    }
}
