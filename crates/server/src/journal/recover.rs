//! `--recover`: rebuild a crashed room from its logs and wait for humans.
//!
//! Playing catch-up uses [`replay_world`]. A bad log, a digest mismatch or a catch-up that
//! runs longer than 30s exits 2 so the matchmaker abandons the room instead of retrying.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::prelude::World;
use evoclash_game::protocol::{BoardRow, MatchState, Phase, RecoverWait, RoomBoard, RoomSettings};
use evoclash_game::rng::mix;

use super::parse::parse_log;
use super::replay::{ReplayOptions, prepare_playing, replay_world};
use super::*;
use crate::config::ServerConfig;
use crate::gameplay::{self, FIRST_VAULT_TICKS, LOBBY_COUNTDOWN_TICKS, SimTick};
use crate::ops::OpsState;

const FAST_FORWARD_LIMIT: Duration = Duration::from_secs(30);

/// Exit code: the log cannot be trusted. The matchmaker must not launch another attempt.
pub const ABANDON_EXIT: i32 = 2;

pub(super) fn recover_on_startup(world: &mut World) {
    let config = world.resource::<ServerConfig>().clone();
    if !config.recover {
        return;
    }
    if let Err(err) = catch_up(world, &config) {
        eprintln!("recover abandoned: {err}");
        std::process::exit(ABANDON_EXIT);
    }
}

fn catch_up(world: &mut World, config: &ServerConfig) -> Result<(), String> {
    let snap = load_room(&config.log_dir, &config.match_id)?;
    match snap.phase.as_str() {
        "lobby" | "starting" => apply_lobby(world, &snap, false),
        "results" => apply_lobby(world, &snap, true),
        "playing" | "recovering" => catch_up_playing(world, config, &snap),
        other => Err(format!("room log phase {other} cannot be recovered")),
    }
}

fn apply_lobby(world: &mut World, snap: &RoomFile, next_index: bool) -> Result<(), String> {
    let match_index = if next_index {
        snap.match_index.saturating_add(1)
    } else {
        snap.match_index
    };
    let map_seed = mix(snap.room_seed, match_index);
    world.resource_mut::<Arena>().0 = evoclash_game::map::ArenaMap::generate(map_seed);
    world.resource::<crate::rng::SimRng>().reset_all(map_seed);
    world.resource_mut::<SimTick>().0 = 0;
    let settings = snap.settings(match_index);
    let public_room = !settings.private_room;
    let mut states = world.query::<(Entity, &MatchState)>();
    let entity = states
        .iter(world)
        .next()
        .map(|(entity, _)| entity)
        .ok_or("match state missing")?;
    if let Some(mut state) = world.get_mut::<MatchState>(entity) {
        state.phase = Phase::Lobby;
        state.match_index = match_index;
        state.map_seed = map_seed;
        state.winner = None;
        state.boss_eta = 0;
        state.vault_in = FIRST_VAULT_TICKS;
        state.recover = RecoverWait::default();
        // A public room restarts the 15s countdown (12s here, then 3s Starting).
        // A private room waits for the host again.
        if public_room {
            state.countdown_active = true;
            state.ticks_left = LOBBY_COUNTDOWN_TICKS;
        } else {
            state.countdown_active = false;
            state.ticks_left = 0;
        }
    }
    if let Some(mut room) = world.get_mut::<RoomSettings>(entity) {
        *room = settings;
    }
    install_board(world, snap.rows.clone());
    publish(
        world,
        "lobby",
        public_room.then_some(LOBBY_COUNTDOWN_TICKS / 64),
        "lobby",
    );
    Ok(())
}

fn catch_up_playing(
    world: &mut World,
    config: &ServerConfig,
    snap: &RoomFile,
) -> Result<(), String> {
    let path = match_log_path(&config.log_dir, &config.match_id, snap.match_index);
    let text = std::fs::read_to_string(&path)
        .map_err(|_| format!("match log missing: {}", path.display()))?;
    let parsed = parse_log(&text)?;
    if parsed.header.protocol != evoclash_game::net::PROTOCOL_ID
        || parsed.header.build != build_id()
    {
        return Err(format!(
            "replay rejected: protocol {} build {} (this binary is protocol {} build {})",
            parsed.header.protocol,
            parsed.header.build,
            evoclash_game::net::PROTOCOL_ID,
            build_id()
        ));
    }
    if parsed.header.match_index != snap.match_index {
        return Err(format!(
            "match log index {} does not match room {}",
            parsed.header.match_index, snap.match_index
        ));
    }
    world.insert_resource(CatchingUp);
    publish(world, "recovering", None, "none");
    prepare_playing(world, &parsed.header);
    let report = replay_world(
        world,
        &parsed,
        &ReplayOptions {
            deadline: Some(Instant::now() + FAST_FORWARD_LIMIT),
            ..ReplayOptions::default()
        },
    );
    world.remove_resource::<CatchingUp>();
    world.remove_resource::<Replaying>();
    if !report.ok {
        return Err(if report.error.is_empty() {
            match report.mismatch_tick {
                Some(tick) => format!("digest mismatch at tick {tick}"),
                None => "recover failed".into(),
            }
        } else {
            report.error
        });
    }
    gameplay::begin_wait(world);
    let tick = world.resource::<SimTick>().0;
    // The seats are held from the first tick the recovered room simulates. A replay of this
    // log learns the same holds from these lines, one tick after the freeze.
    let mut away: Vec<u64> = {
        let mut humans = world.query::<&evoclash_game::protocol::Player>();
        humans
            .iter(world)
            .filter(|player| !player.bot)
            .map(|player| player.client_id)
            .collect()
    };
    away.sort_unstable();
    if let Some(mut journal) = world.get_resource_mut::<Journal>() {
        journal.adopt_match(snap.match_index, path);
        journal.record_plain(&format!("recover {tick}"));
        journal.mark(tick, "freeze");
        for id in &away {
            journal.mark(tick.saturating_add(1), &format!("disconnect {id}"));
        }
        journal.flush();
    }
    let left = world
        .query::<&MatchState>()
        .iter(world)
        .next()
        .map(|state| state.ticks_left / 64);
    install_board(world, snap.rows.clone());
    // Replay already applied leaves. A row for someone no longer here is dropped.
    crate::gameplay::sync_present(world);
    publish(world, "recovering", left, "none");
    Ok(())
}

fn install_board(world: &mut World, rows: Vec<BoardRow>) {
    let mut boards = world.query::<&mut RoomBoard>();
    if let Some(mut board) = boards.iter_mut(world).next() {
        board.rows = rows;
    }
}

fn publish(world: &mut World, phase: &str, left: Option<u32>, join_mode: &str) {
    if let Some(ops) = world.get_resource::<OpsState>() {
        ops.publish_phase(phase, left, join_mode);
    }
}

fn match_log_path(dir: &std::path::Path, match_id: &str, index: u32) -> PathBuf {
    dir.join(format!("match-{}-{index}.log", sanitize(match_id)))
}

fn load_room(dir: &std::path::Path, match_id: &str) -> Result<RoomFile, String> {
    let path = dir.join(format!("room-{}.log", sanitize(match_id)));
    let text = std::fs::read_to_string(&path)
        .map_err(|_| format!("room log missing: {}", path.display()))?;
    let mut last = None;
    for line in text.lines() {
        if let Some(snap) = parse_room_line(line) {
            last = Some(snap);
        }
    }
    last.ok_or_else(|| format!("room log has no snapshot: {}", path.display()))
}

struct RoomFile {
    room_seed: u64,
    code: String,
    private: bool,
    bots: Option<u8>,
    monsters: bool,
    host: u64,
    match_index: u32,
    phase: String,
    paused: bool,
    rows: Vec<BoardRow>,
}

impl RoomFile {
    fn settings(&self, _match_index: u32) -> RoomSettings {
        RoomSettings {
            bots: self.bots,
            monsters: self.monsters,
            host: self.host,
            private_room: self.private,
            code: if self.code == "-" {
                String::new()
            } else {
                self.code.clone()
            },
            auto_paused: self.paused,
        }
    }
}

fn parse_room_line(line: &str) -> Option<RoomFile> {
    if !line.contains("room_seed=") {
        return None;
    }
    // `board=` is last and may contain spaces. Names are `esc`'d (`\|` `\;` `\\`).
    let (head, board_raw) = match line.split_once(" board=") {
        Some((head, raw)) => (head, Some(raw)),
        None => (line, None),
    };
    let mut room_seed = None;
    let mut code = None;
    let mut private = None;
    let mut bots = None;
    let mut monsters = None;
    let mut host = None;
    let mut match_index = None;
    let mut phase = None;
    let mut paused = false;
    for part in head.split_whitespace() {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        match key {
            "room_seed" => room_seed = value.parse().ok(),
            "code" => code = Some(value.to_string()),
            "private" => private = Some(value == "1"),
            "bots" => {
                bots = Some(if value == "off" {
                    None
                } else {
                    Some(value.parse().ok()?)
                });
            }
            "monsters" => monsters = Some(value == "1"),
            "host" => host = value.parse().ok(),
            "match_index" => match_index = value.parse().ok(),
            "phase" => phase = Some(value.to_string()),
            "paused" => paused = value == "1",
            _ => {}
        }
    }
    Some(RoomFile {
        room_seed: room_seed?,
        code: code?,
        private: private?,
        bots: bots?,
        monsters: monsters?,
        host: host?,
        match_index: match_index?,
        phase: phase?,
        paused,
        rows: board_raw.map(decode_board).unwrap_or_default(),
    })
}

fn decode_board(raw: &str) -> Vec<BoardRow> {
    if raw.is_empty() || raw == "-" {
        return Vec::new();
    }
    split_kept(raw, ';')
        .into_iter()
        .filter_map(|part| {
            let fields = split_esc(&part);
            if fields.len() < 6 {
                return None;
            }
            Some(BoardRow {
                client_id: fields[0].parse().ok()?,
                name: fields[1].clone(),
                points: fields[2].parse().ok()?,
                played: fields[3].parse().ok()?,
                last_place: fields[4].parse().ok()?,
                last_round: fields[5].parse().ok()?,
            })
        })
        .collect()
}

/// Split on `sep`, keeping a backslash escape with the following character.
fn split_kept(text: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            cur.push('\\');
            if let Some(next) = chars.next() {
                cur.push(next);
            }
        } else if c == sep {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() || text.contains(sep) {
        out.push(cur);
    }
    out
}

/// Used by tests. Same path as `--recover`, without exiting the process.
#[cfg(test)]
pub fn recover_into(world: &mut World) -> Result<(), String> {
    let config = world.resource::<ServerConfig>().clone();
    catch_up(world, &config)
}
