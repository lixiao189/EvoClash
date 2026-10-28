//! Read-only match statistics for the headless runner.
//!
//! Systems here only append to [`StatsBus`]. They are not installed on the live server, so
//! online matches and replays keep the schedule they had before.

mod fold;
mod targets;

use bevy::prelude::*;
use evoclash_game::protocol::GameEvent;

use crate::gameplay::{DamageQueue, Outbox, SimSet, flush_outbox, reset_match, resolve_damage};

pub use fold::{Autopsy, Fold, TraceSnap};
pub use targets::{print_table, targets};

/// Stance / weapon / nearby-threat picture of bot deaths, grouped by how they died.
pub fn autopsy_summary(records: &[MatchRecord]) -> serde_json::Value {
    use std::collections::BTreeMap;
    let mut snaps: BTreeMap<&str, Vec<&TraceSnap>> = BTreeMap::new();
    for record in records {
        for case in &record.autopsies {
            snaps
                .entry(case.how.as_str())
                .or_default()
                .extend(&case.snaps);
        }
    }
    let mut out = serde_json::Map::new();
    for (how, rows) in snaps {
        let mut stance = BTreeMap::<u8, usize>::new();
        let mut weapon = BTreeMap::<&str, usize>::new();
        let mut threats: Vec<u8> = rows.iter().map(|s| s.threat_n).collect();
        let mut stick = 0usize;
        for snap in &rows {
            *stance.entry(snap.stance).or_default() += 1;
            *weapon.entry(snap.weapon.as_str()).or_default() += 1;
            if snap.weapon == "木棍" {
                stick += 1;
            }
        }
        threats.sort_unstable();
        let mid = threats.get(threats.len() / 2).copied().unwrap_or(0);
        out.insert(
            how.to_string(),
            serde_json::json!({
                "snaps": rows.len(),
                "stance": stance,
                "weapon": weapon,
                "threat_median": mid,
                "stick_pct": if rows.is_empty() { 0 } else { stick * 100 / rows.len() },
            }),
        );
    }
    serde_json::Value::Object(out)
}

/// Same ordering as `gameplay::final_rank`: higher is better.
pub(super) fn rank_key(
    score: u32,
    kills: u32,
    deaths: u32,
    last_score_tick: u32,
    client_id: u64,
) -> (
    u32,
    u32,
    std::cmp::Reverse<u32>,
    std::cmp::Reverse<u32>,
    std::cmp::Reverse<u64>,
) {
    (
        score,
        kills,
        std::cmp::Reverse(deaths),
        std::cmp::Reverse(last_score_tick),
        std::cmp::Reverse(client_id),
    )
}

pub struct StatsPlugin;

impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StatsBus>();
        app.add_systems(
            FixedUpdate,
            capture_hits.in_set(SimSet::Resolve).before(resolve_damage),
        );
        app.add_systems(
            FixedUpdate,
            capture_events
                .in_set(SimSet::Flow)
                .after(reset_match)
                .before(flush_outbox),
        );
    }
}

#[derive(Resource, Default)]
pub struct StatsBus {
    pub hits: Vec<PvpHit>,
    pub events: Vec<Note>,
}

pub struct PvpHit {
    pub attacker: u64,
    pub victim: u64,
}

pub enum Note {
    Kill {
        killer: String,
        victim: String,
        how: String,
        bounty: bool,
    },
    Announce(String),
    BossDown {
        killer: Option<String>,
    },
    Boom {
        pos: Vec2,
        radius: f32,
        fire: bool,
    },
}

fn capture_hits(
    mut bus: ResMut<StatsBus>,
    queue: Res<DamageQueue>,
    players: Query<&evoclash_game::protocol::Player>,
    state: Query<&evoclash_game::protocol::MatchState>,
) {
    if !state
        .iter()
        .any(|s| s.phase == evoclash_game::protocol::Phase::Playing)
    {
        return;
    }
    for dmg in &queue.0 {
        if dmg.amount <= 0.0 {
            continue;
        }
        let Some(attacker) = dmg.attacker else {
            continue;
        };
        let (Ok(victim), Ok(attacker)) = (players.get(dmg.target), players.get(attacker)) else {
            continue;
        };
        if victim.client_id == attacker.client_id {
            continue;
        }
        bus.hits.push(PvpHit {
            attacker: attacker.client_id,
            victim: victim.client_id,
        });
    }
}

fn capture_events(mut bus: ResMut<StatsBus>, outbox: Res<Outbox>) {
    for event in outbox.reliable.iter().chain(&outbox.fast) {
        match event {
            GameEvent::Kill {
                killer,
                victim,
                how,
                bounty,
            } => bus.events.push(Note::Kill {
                killer: killer.clone(),
                victim: victim.clone(),
                how: how.clone(),
                bounty: *bounty,
            }),
            GameEvent::Announce(text) => bus.events.push(Note::Announce(text.clone())),
            GameEvent::BossDown { killer } => bus.events.push(Note::BossDown {
                killer: killer.clone(),
            }),
            GameEvent::Boom { pos, radius, fire } => bus.events.push(Note::Boom {
                pos: *pos,
                radius: *radius,
                fire: *fire,
            }),
            _ => {}
        }
    }
}

/// One finished (or truncated) match. Integers only, so two runs compare equal.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MatchRecord {
    pub seed: u64,
    pub match_index: u32,
    pub monsters: bool,
    pub playing_ticks: u32,
    pub finished: bool,
    pub players: Vec<PlayerRec>,
    pub kills: Vec<KillRec>,
    pub lives: Vec<LifeRec>,
    pub scrap_earned: u32,
    pub scrap_player: u32,
    pub scrap_container: u32,
    /// Safes a combatant cracked. Not the vault-open announcement.
    pub safes_cracked: u32,
    /// Times a bot entered Retreat, once per continuous stretch.
    pub bot_retreats: u32,
    /// Times a bot entered Avoid, once per continuous stretch.
    pub bot_avoids: u32,
    pub snap_2m: Option<Vec<SnapRow>>,
    pub snap_5m: Option<Vec<SnapRow>>,
    pub snap_6m: Option<Vec<SnapRow>>,
    pub finale_inner: u32,
    pub finale_alive: u32,
    pub t4_hold_ticks: Vec<u32>,
    pub vault_opens: u32,
    pub vault_t4_hits: u32,
    pub bounties: u32,
    pub bounties_killed_60s: u32,
    pub boss_appeared: bool,
    pub boss_killed: bool,
    pub boss_left: bool,
    pub boss_appear_tick: Option<u32>,
    pub boss_end_tick: Option<u32>,
    pub boss_top_not_leader: Option<bool>,
    pub max_streak: u32,
    /// Bot deaths, in order, capped at 40. Headless only.
    pub autopsies: Vec<Autopsy>,
    /// Lives that started this match, and how many of those already held a gun.
    pub births: u32,
    pub births_armed: u32,
    pub monster_ttk_ticks: Vec<u32>,
    pub elite_ttk_ticks: Vec<u32>,
    pub fight_ticks: Vec<u32>,
    pub winner_id: Option<u64>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PlayerRec {
    pub client_id: u64,
    pub name: String,
    pub skill: String,
    pub path: String,
    pub rank: u8,
    pub score: u32,
    pub kills: u32,
    pub deaths: u32,
    pub level: u8,
    pub had_t4: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct KillRec {
    pub tick: u32,
    pub killer_id: Option<u64>,
    pub victim_id: u64,
    pub how: String,
    /// `player` / `monster` / `boss` / `environment`.
    pub cause: String,
    pub weapon: Option<String>,
    pub killer_tier: Option<u8>,
    pub victim_gear: u8,
    pub underdog: bool,
    pub bounty: bool,
    /// Killer's weapon came from the vault, a safe, or the behemoth.
    pub sourced: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct LifeRec {
    pub client_id: u64,
    pub start: u32,
    pub end: u32,
    pub opens: u32,
    pub first_t1: Option<u32>,
    pub first_t2: Option<u32>,
    pub quiet_ticks: u32,
    pub scrap: u32,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SnapRow {
    pub client_id: u64,
    pub score: u32,
    pub kills: u32,
    pub deaths: u32,
    pub last_score_tick: u32,
    pub gear: u8,
    pub level: u8,
}
