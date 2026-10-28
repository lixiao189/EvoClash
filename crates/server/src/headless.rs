//! Offline batch matches: no socket, no matchmaker, no heartbeat.
//!
//! Each seed is its own `App`. `FixedUpdate` is stepped by hand, so a match does not wait on
//! the wall clock. `--jobs` runs seeds on separate threads; a single match stays single-threaded.

use std::net::SocketAddr;
use std::thread;
use std::time::Instant;

use bevy::app::{TaskPoolOptions, TaskPoolPlugin};
use bevy::prelude::*;
use evoclash_game::protocol::{MatchState, Phase, Player, ROOM_COMBATANTS};

use crate::bots::{BotSkill, SkillMix};
use crate::config::{HeadlessOpts, ServerConfig};
use crate::gameplay::{MATCH_TICKS, PendingStart, ResetRequested};
use crate::journal::install_sim;
use crate::stats::{
    Fold, MatchRecord, StatsBus, StatsPlugin, autopsy_summary, print_table, targets,
};

pub fn run(opts: &HeadlessOpts) -> i32 {
    let skills = parse_skills(&opts.skills);
    let started = Instant::now();
    let records = run_seeds(&opts.seeds, opts.monsters, skills, opts.jobs, None);
    let batch_seconds = started.elapsed().as_secs_f64();
    let repeat_started = Instant::now();
    let repeat = if skills.iter().all(|s| *s == BotSkill::Default) {
        let seed = opts.seeds.first().copied().unwrap_or(0);
        Some(series_winners(seed, opts.monsters, skills, 10))
    } else {
        None
    };
    let repeat_seconds = repeat_started.elapsed().as_secs_f64();
    let rows = targets(&records, repeat.as_deref());
    let n = records.len();
    let per = if n == 0 {
        0.0
    } else {
        batch_seconds / n as f64
    };
    print_table(&rows);
    let report = serde_json::json!({
        "seeds": opts.seeds,
        "monsters": opts.monsters,
        "skills": skills.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "seconds": batch_seconds,
        "seconds_per_match": per,
        "repeat_seconds": repeat.as_ref().map(|_| repeat_seconds),
        "repeat_winners": repeat,
        "per_match": records,
        "targets": rows,
        "autopsy": autopsy_summary(&records),
    });
    if let Some(parent) = opts.report.parent()
        && !parent.as_os_str().is_empty()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        eprintln!("cannot create {}: {err}", parent.display());
        return 1;
    }
    if let Err(err) = std::fs::write(&opts.report, serde_json::to_string_pretty(&report).unwrap()) {
        eprintln!("cannot write {}: {err}", opts.report.display());
        return 1;
    }
    eprintln!(
        "{n} matches in {batch_seconds:.1}s ({per:.2}s each) -> {}",
        opts.report.display()
    );
    0
}

/// One seed, `playing_ticks` of Playing (`None` runs the full 8 minutes).
pub fn play_seed(
    seed: u64,
    monsters: bool,
    skills: [BotSkill; 8],
    playing_ticks: Option<u32>,
) -> MatchRecord {
    let mut app = boot(seed, monsters, skills);
    play_on(&mut app, playing_ticks)
}

fn run_seeds(
    seeds: &[u64],
    monsters: bool,
    skills: [BotSkill; 8],
    jobs: usize,
    playing_ticks: Option<u32>,
) -> Vec<MatchRecord> {
    if seeds.is_empty() {
        return Vec::new();
    }
    let jobs = jobs.max(1).min(seeds.len());
    if jobs == 1 {
        return seeds
            .iter()
            .map(|seed| play_seed(*seed, monsters, skills, playing_ticks))
            .collect();
    }
    let mut chunks: Vec<Vec<u64>> = (0..jobs).map(|_| Vec::new()).collect();
    for (i, seed) in seeds.iter().copied().enumerate() {
        chunks[i % jobs].push(seed);
    }
    let mut handles = Vec::new();
    for chunk in chunks {
        handles.push(thread::spawn(move || {
            chunk
                .into_iter()
                .map(|seed| play_seed(seed, monsters, skills, playing_ticks))
                .collect::<Vec<_>>()
        }));
    }
    let mut records = Vec::new();
    for handle in handles {
        records.extend(handle.join().expect("headless worker"));
    }
    records.sort_by_key(|r| (r.seed, r.match_index));
    records
}

fn series_winners(seed: u64, monsters: bool, skills: [BotSkill; 8], n: u32) -> Vec<String> {
    let mut app = boot(seed, monsters, skills);
    let mut names = Vec::new();
    for i in 0..n {
        if i > 0 {
            advance_room(&mut app);
        }
        let record = play_on(&mut app, None);
        let name = record
            .winner_id
            .and_then(|id| record.players.iter().find(|p| p.client_id == id))
            .map(|p| p.name.clone())
            .unwrap_or_default();
        names.push(name);
    }
    names
}

fn boot(seed: u64, monsters: bool, skills: [BotSkill; 8]) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(TaskPoolPlugin {
        task_pool_options: TaskPoolOptions::with_num_threads(1),
    }));
    app.insert_resource(match_config(seed, monsters));
    install_sim(&mut app);
    app.insert_resource(SkillMix {
        tiers: skills.to_vec(),
    });
    app.add_plugins(StatsPlugin);
    app.finish();
    app.world_mut().run_schedule(Startup);
    app
}

fn match_config(seed: u64, monsters: bool) -> ServerConfig {
    ServerConfig {
        port: 0,
        public_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        match_id: format!("headless-{seed}"),
        max_players: ROOM_COMBATANTS as usize,
        min_combatants: ROOM_COMBATANTS as usize,
        matchmakers: Vec::new(),
        single_match: false,
        monsters,
        threads: 1,
        agones: false,
        room_seed: seed,
        private: false,
        room_code: String::new(),
        log_dir: std::env::temp_dir(),
        replay: None,
        recover: false,
        logging: false,
    }
}

fn play_on(app: &mut App, playing_ticks: Option<u32>) -> MatchRecord {
    ensure_roster(app);
    let _ = std::mem::take(&mut *app.world_mut().resource_mut::<StatsBus>());
    let mut fold = Fold::new();
    let mut played = 0u32;
    let mut guard = 0u32;
    let cap = playing_ticks.unwrap_or(MATCH_TICKS) + 64;
    let finished = loop {
        guard += 1;
        if guard > cap + 8_000 {
            break false;
        }
        let before = phase_of(app.world_mut());
        app.world_mut().run_schedule(FixedUpdate);
        let after = phase_of(app.world_mut());
        let bus = std::mem::take(&mut *app.world_mut().resource_mut::<StatsBus>());
        let stamp = stamp_of(before, after);
        fold.tick(app.world_mut(), stamp, &bus);
        if stamp.is_some_and(|t| t > 0) {
            played += 1;
            if playing_ticks.is_some_and(|n| played >= n) {
                break false;
            }
        }
        if after.0 == Phase::Results {
            break true;
        }
    };
    fold.finish(app.world_mut(), finished)
}

fn ensure_roster(app: &mut App) {
    for _ in 0..32 {
        let n = count_players(app.world_mut());
        let phase = phase_of(app.world_mut()).0;
        if n >= usize::from(ROOM_COMBATANTS) && phase != Phase::Lobby {
            return;
        }
        if n >= usize::from(ROOM_COMBATANTS) && phase == Phase::Lobby {
            arm(app.world_mut());
            return;
        }
        app.world_mut().run_schedule(FixedUpdate);
    }
}

fn advance_room(app: &mut App) {
    {
        let world = app.world_mut();
        let mut states = world.query::<&mut MatchState>();
        for mut state in states.iter_mut(world) {
            if state.phase == Phase::Results {
                state.ticks_left = 1;
            }
        }
    }
    app.world_mut().run_schedule(FixedUpdate);
    arm(app.world_mut());
}

fn arm(world: &mut World) {
    let mut states = world.query::<&mut MatchState>();
    for mut state in states.iter_mut(world) {
        if state.phase == Phase::Lobby {
            state.phase = Phase::Starting;
            state.ticks_left = 2;
            state.countdown_active = false;
        }
    }
    world.resource_mut::<ResetRequested>().0 = true;
    world.resource_mut::<PendingStart>().0 = true;
}

fn phase_of(world: &mut World) -> (Phase, u32) {
    world
        .query::<&MatchState>()
        .iter(world)
        .next()
        .map(|s| (s.phase, s.ticks_left))
        .unwrap_or((Phase::Lobby, 0))
}

fn stamp_of(before: (Phase, u32), after: (Phase, u32)) -> Option<u32> {
    match (before.0, after.0) {
        (_, Phase::Playing) => Some(MATCH_TICKS.saturating_sub(after.1)),
        (Phase::Playing, Phase::Results) => Some(MATCH_TICKS),
        _ => None,
    }
}

fn count_players(world: &mut World) -> usize {
    world.query::<&Player>().iter(world).count()
}

pub fn parse_skills(spec: &str) -> [BotSkill; 8] {
    let mut out = [BotSkill::Default; 8];
    if spec.trim().is_empty() {
        return out;
    }
    let mut i = 0;
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (name, n) = part.split_once(':').unwrap_or((part, "1"));
        let tier = match name.trim() {
            "high" => BotSkill::High,
            "default" => BotSkill::Default,
            "novice" => BotSkill::Novice,
            other => panic!("unknown skill tier {other} (high, default, novice)"),
        };
        let n: usize = n.trim().parse().expect("skill count");
        for _ in 0..n {
            if i >= 8 {
                panic!("--skills lists more than 8 bots");
            }
            out[i] = tier;
            i += 1;
        }
    }
    out
}

pub fn parse_seeds(spec: &str) -> Vec<u64> {
    let spec = spec.trim();
    if let Some((a, b)) = spec.split_once("..=") {
        let a: u64 = a.trim().parse().expect("seed");
        let b: u64 = b.trim().parse().expect("seed");
        assert!(b >= a, "empty seed range");
        return (a..=b).collect();
    }
    if let Some((a, b)) = spec.split_once("..") {
        let a: u64 = a.trim().parse().expect("seed");
        let b: u64 = b.trim().parse().expect("seed");
        assert!(b > a, "empty seed range");
        return (a..b).collect();
    }
    spec.split(',')
        .map(|p| p.trim().parse().expect("seed"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_matches_twice() {
        let skills = [BotSkill::Default; 8];
        let a = play_seed(7, true, skills, Some(96));
        let b = play_seed(7, true, skills, Some(96));
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        assert_eq!(a.players.len(), 8);
    }

    #[test]
    fn two_short_seeds_write_json() {
        let skills = [BotSkill::Default; 8];
        let records = run_seeds(&[3, 4], false, skills, 1, Some(48));
        let rows = targets(&records, None);
        let report = serde_json::json!({
            "per_match": records,
            "targets": rows,
        });
        let text = serde_json::to_string(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["per_match"].as_array().unwrap().len(), 2);
        assert!(parsed["targets"].as_array().unwrap().len() >= 40);
        assert!(
            parsed["targets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["id"] == 6 && row["status"] == "not_measured")
        );
    }
}
