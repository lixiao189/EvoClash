//! Room log, match log, retention, and offline `--replay`.
//!
//! Playing is a pure function of `match_seed`, the opening roster, room settings, and the
//! external inputs after that. The match log records those inputs and a digest every 640
//! ticks. Replay rebuilds Playing from the header and checks the digests.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use evoclash_game::evolution::{Evolution, Perk};
use evoclash_game::items::InventoryMove;
use evoclash_game::net::PROTOCOL_ID;
use evoclash_game::protocol::*;
use evoclash_game::rng::mix;
use evoclash_game::shop::{PlayerCommand, ShopItem};
use evoclash_game::sim::Arena;
use lightyear::prelude::PeerId;

use crate::bots::Bot;
use crate::config::ServerConfig;
use crate::connections::Human;
use crate::gameplay::{
    CurrentInput, FIRST_VAULT_TICKS, MATCH_TICKS, SPAWN_SHIELD_TICKS, SimSet, SimTick, match_flow,
    read_human_inputs, reset_match, spawn_combatant,
};
use crate::rng::SimRng;

/// Sealed logs older than a match length times two are deleted.
pub const RETAIN_SECS: u64 = 16 * 60;
const DIGEST_EVERY: u32 = 640;
const FLUSH_EVERY: u32 = 16;

pub fn build_id() -> &'static str {
    option_env!("EVOCLASH_BUILD")
        .filter(|id| !id.is_empty())
        .unwrap_or("dev")
}

/// External inputs a test or a replay injects for the upcoming tick.
#[derive(Resource, Default)]
pub struct Injected {
    pub inputs: Vec<(u64, PlayerInput)>,
    pub commands: Vec<(u64, PlayerCommand)>,
    pub moves: Vec<(u64, InventoryMove)>,
    pub rooms: Vec<(u64, RoomCommand)>,
    pub connects: Vec<u64>,
    pub disconnects: Vec<u64>,
    pub leaves: Vec<u64>,
}

/// Lines stamped with the current sim tick when the match log flushes them.
#[derive(Resource, Default)]
pub struct Trace {
    lines: Vec<String>,
}

impl Trace {
    pub fn push(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }
}

/// Override `SystemTime::now` in tests. Production leaves it unset.
#[derive(Resource, Clone, Copy)]
pub struct LogClock(pub SystemTime);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KillLine {
    pub tick: u32,
    pub killer: String,
    pub victim: String,
    pub how: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreLine {
    pub client_id: u64,
    pub score: u32,
    pub kills: u32,
    pub deaths: u32,
    pub xp: u32,
}

struct LogBuf {
    file: File,
    buf: Vec<u8>,
}

impl LogBuf {
    fn create(path: &Path) -> Option<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok()?;
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .ok()?;
        Some(Self {
            file,
            buf: Vec::new(),
        })
    }

    fn append(path: &Path) -> Option<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok()?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        Some(Self {
            file,
            buf: Vec::new(),
        })
    }

    fn line(&mut self, text: &str) {
        self.buf.extend_from_slice(text.as_bytes());
        self.buf.push(b'\n');
    }

    fn flush(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let _ = self.file.write_all(&self.buf);
        self.buf.clear();
        let _ = self.file.sync_all();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RoomSnap {
    room_seed: u64,
    code: String,
    private: bool,
    bots: Option<u8>,
    monsters: bool,
    host: u64,
    match_index: u32,
    phase: Phase,
}

impl RoomSnap {
    fn line(&self) -> String {
        let code = if self.code.is_empty() {
            "-".to_string()
        } else {
            self.code.clone()
        };
        let bots = self
            .bots
            .map(|n| n.to_string())
            .unwrap_or_else(|| "off".into());
        format!(
            "room_seed={} code={code} private={} bots={bots} monsters={} host={} match_index={} phase={}",
            self.room_seed,
            u8::from(self.private),
            u8::from(self.monsters),
            self.host,
            self.match_index,
            phase_name(self.phase),
        )
    }
}

#[derive(Resource)]
pub struct Journal {
    logging: bool,
    dir: PathBuf,
    match_id: String,
    room_seed: u64,
    room: Option<LogBuf>,
    match_log: Option<LogBuf>,
    open_index: Option<u32>,
    pub match_path: Option<PathBuf>,
    room_snap: Option<RoomSnap>,
    prev_phase: Option<Phase>,
    ticks: u32,
    sealed: bool,
    pub kills: Vec<KillLine>,
}

impl Journal {
    fn create(config: &ServerConfig) -> Self {
        let room = if config.logging {
            let path = config
                .log_dir
                .join(format!("room-{}.log", sanitize(&config.match_id)));
            let fresh = fs::metadata(&path).map(|m| m.len() == 0).unwrap_or(true);
            let mut room = LogBuf::append(&path);
            if fresh && let Some(room) = room.as_mut() {
                room.line("EVOCLASH room v1");
            }
            room
        } else {
            None
        };
        Self {
            logging: config.logging,
            dir: config.log_dir.clone(),
            match_id: config.match_id.clone(),
            room_seed: config.room_seed,
            room,
            match_log: None,
            open_index: None,
            match_path: None,
            room_snap: None,
            prev_phase: None,
            ticks: 0,
            sealed: false,
            kills: Vec::new(),
        }
    }

    fn note_room(&mut self, snap: RoomSnap) {
        if self.room_snap.as_ref() == Some(&snap) {
            return;
        }
        if let Some(room) = self.room.as_mut() {
            room.line(&snap.line());
        }
        self.room_snap = Some(snap);
    }

    fn open_match(&mut self, index: u32, settings: &RoomSettings, roster: &[RosterEntry]) {
        if !self.logging || self.match_log.is_some() {
            return;
        }
        let path = self
            .dir
            .join(format!("match-{}-{index}.log", sanitize(&self.match_id)));
        let Some(mut log) = LogBuf::create(&path) else {
            return;
        };
        log.line("EVOCLASH match v1");
        log.line(&format!("protocol {PROTOCOL_ID}"));
        log.line(&format!("build {}", build_id()));
        log.line(&format!("room_seed {}", self.room_seed));
        log.line(&format!("match_index {index}"));
        log.line(&settings_line(settings));
        let mut roster = roster.to_vec();
        roster.sort_by_key(|e| e.client_id);
        for entry in &roster {
            log.line(&entry.line());
        }
        log.line("");
        self.match_log = Some(log);
        self.match_path = Some(path);
        self.open_index = Some(index);
        self.sealed = false;
        self.kills.clear();
    }

    fn record(&mut self, tick: u32, line: &str) {
        if tick == 0 || self.sealed {
            return;
        }
        if let Some(log) = self.match_log.as_mut() {
            log.line(&format!("t {tick} {line}"));
        }
    }

    fn digest(&mut self, tick: u32, hash: u64) {
        self.record(tick, &format!("digest {hash:016x}"));
    }

    fn seal(&mut self, scores: &[ScoreLine], now: SystemTime) {
        if self.sealed || self.match_log.is_none() {
            return;
        }
        if let Some(log) = self.match_log.as_mut() {
            let mut parts = Vec::new();
            for score in scores {
                parts.push(format!(
                    "{}:{}:{}:{}:{}",
                    score.client_id, score.score, score.kills, score.deaths, score.xp
                ));
            }
            log.line(&format!("scores {}", parts.join(" ")));
            let kills: Vec<String> = self
                .kills
                .iter()
                .map(|kill| {
                    format!(
                        "{}|{}|{}|{}",
                        kill.tick,
                        esc(&kill.killer),
                        esc(&kill.victim),
                        esc(&kill.how)
                    )
                })
                .collect();
            log.line(&format!("kills {}", kills.join(";")));
            let secs = now
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            log.line(&format!("end {secs}"));
        }
        self.sealed = true;
        self.flush();
        if self.logging {
            purge_logs(&self.dir, now);
        }
    }

    pub fn flush(&mut self) {
        if let Some(room) = self.room.as_mut() {
            room.flush();
        }
        if let Some(log) = self.match_log.as_mut() {
            log.flush();
        }
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        self.flush();
    }
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '_',
            _ => c,
        })
        .collect()
}

fn esc(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(';', "\\;")
        .replace('\n', " ")
}

fn unesc(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn fmt_input(input: &PlayerInput) -> String {
    format!(
        "{} {} {} {} {} {} {} {} {} {} {}",
        input.move_x,
        input.move_y,
        input.aim.x.to_bits(),
        input.aim.y.to_bits(),
        u8::from(input.fire),
        u8::from(input.dash),
        u8::from(input.interact),
        u8::from(input.throw),
        u8::from(input.skill_a),
        u8::from(input.skill_b),
        input.select,
    )
}

pub fn fmt_command(cmd: &PlayerCommand) -> String {
    match cmd {
        PlayerCommand::Spend(perk) => format!("spend {perk:?}"),
        PlayerCommand::Refund(perk) => format!("refund {perk:?}"),
        PlayerCommand::ResetPerks => "reset".into(),
        PlayerCommand::Buy(item) => format!("buy {item:?}"),
        PlayerCommand::Unbuy(item) => format!("unbuy {item:?}"),
        PlayerCommand::Deploy => "deploy".into(),
        PlayerCommand::DeployLast => "deploy_last".into(),
    }
}

pub fn fmt_room(cmd: &RoomCommand) -> String {
    match cmd {
        RoomCommand::SetSettings { bots, monsters } => format!(
            "settings {} {}",
            bots.map(|n| n.to_string()).unwrap_or_else(|| "off".into()),
            u8::from(*monsters)
        ),
        RoomCommand::Start => "start".into(),
        RoomCommand::Cancel => "cancel".into(),
        RoomCommand::Leave => "leave".into(),
    }
}

fn settings_line(settings: &RoomSettings) -> String {
    let bots = settings
        .bots
        .map(|n| n.to_string())
        .unwrap_or_else(|| "off".into());
    let code = if settings.code.is_empty() {
        "-".to_string()
    } else {
        settings.code.clone()
    };
    format!(
        "settings {bots} {} {} {} {code}",
        u8::from(settings.monsters),
        settings.host,
        u8::from(settings.private_room),
    )
}

#[derive(Clone, Debug)]
struct RosterEntry {
    client_id: u64,
    name: String,
    bot: bool,
    color: [f32; 3],
}

impl RosterEntry {
    fn line(&self) -> String {
        let name = self.name.replace(['\n', '\r'], " ");
        format!(
            "player {} {} {} {} {} {name}",
            self.client_id,
            u8::from(self.bot),
            self.color[0].to_bits(),
            self.color[1].to_bits(),
            self.color[2].to_bits(),
        )
    }
}

pub fn phase_name(phase: Phase) -> &'static str {
    crate::gameplay::phase_name(phase)
}

/// FNV-1a 64. Stable across builds; `std` `DefaultHasher` is not.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

pub fn combatant_digest(world: &mut World) -> u64 {
    let mut query = world.query::<(&Player, &Pos, &Vitals, &Stats, &Evolution)>();
    let mut rows: Vec<(u64, u32, u32, u32, u32, u32)> = query
        .iter(world)
        .map(|(player, pos, vitals, stats, evo)| {
            (
                player.client_id,
                pos.0.x.to_bits(),
                pos.0.y.to_bits(),
                vitals.hp.to_bits(),
                stats.score,
                evo.xp,
            )
        })
        .collect();
    rows.sort_unstable_by_key(|row| row.0);
    let mut bytes = Vec::with_capacity(rows.len() * 28);
    for (id, x, y, hp, score, xp) in rows {
        bytes.extend(id.to_le_bytes());
        bytes.extend(x.to_le_bytes());
        bytes.extend(y.to_le_bytes());
        bytes.extend(hp.to_le_bytes());
        bytes.extend(score.to_le_bytes());
        bytes.extend(xp.to_le_bytes());
    }
    fnv1a64(&bytes)
}

pub fn scoreboard(world: &mut World) -> Vec<ScoreLine> {
    let mut query = world.query::<(&Player, &Stats, &Evolution)>();
    let mut rows: Vec<ScoreLine> = query
        .iter(world)
        .map(|(player, stats, evo)| ScoreLine {
            client_id: player.client_id,
            score: stats.score,
            kills: stats.kills,
            deaths: stats.deaths,
            xp: evo.xp,
        })
        .collect();
    rows.sort_by_key(|row| row.client_id);
    rows
}

/// Delete sealed logs whose `end` timestamp is older than 16 minutes. Unsealed logs stay.
pub fn purge_logs(dir: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now_secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("log") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let mut end = None;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("end ")
                && let Ok(ts) = rest.trim().parse::<u64>()
            {
                end = Some(ts);
            }
        }
        let Some(ts) = end else {
            continue;
        };
        if now_secs.saturating_sub(ts) > RETAIN_SECS {
            let _ = fs::remove_file(path);
        }
    }
}

pub struct JournalPlugin;

impl Plugin for JournalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Trace>().init_resource::<Injected>();
        // The multi-thread executor runs conflicting systems out of topological order, so two
        // passes of the same inputs (the live match and its replay) can diverge. One thread
        // keeps FixedUpdate on the schedule's fixed order.
        app.edit_schedule(FixedUpdate, |schedule| {
            schedule.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
        });
        app.add_systems(Startup, open_logs);
        app.add_systems(
            FixedUpdate,
            (
                apply_presence
                    .in_set(SimSet::Input)
                    .before(read_human_inputs),
                log_inputs.in_set(SimSet::Input).after(read_human_inputs),
                journal_flow
                    .in_set(SimSet::Flow)
                    .after(match_flow)
                    .before(reset_match),
            ),
        );
    }
}

fn open_logs(mut commands: Commands, config: Res<ServerConfig>, clock: Option<Res<LogClock>>) {
    let now = clock.map(|clock| clock.0).unwrap_or_else(SystemTime::now);
    if config.logging {
        let _ = fs::create_dir_all(&config.log_dir);
        purge_logs(&config.log_dir, now);
    }
    commands.insert_resource(Journal::create(&config));
}

fn apply_presence(
    mut commands: Commands,
    mut injected: ResMut<Injected>,
    mut humans: Query<(Entity, &mut Human, &Player)>,
) {
    let connects = std::mem::take(&mut injected.connects);
    let disconnects = std::mem::take(&mut injected.disconnects);
    let leaves = std::mem::take(&mut injected.leaves);
    for id in connects {
        if let Some((_, mut human, _)) = humans
            .iter_mut()
            .find(|(_, _, player)| player.client_id == id)
        {
            human.disconnected_ticks = 0;
            human.left = false;
        }
    }
    for id in disconnects {
        if let Some((_, mut human, _)) = humans
            .iter_mut()
            .find(|(_, _, player)| player.client_id == id)
            && human.connected()
        {
            human.disconnected_ticks = 1;
        }
    }
    for id in leaves {
        if let Some((entity, _, _)) = humans.iter().find(|(_, _, player)| player.client_id == id) {
            commands.entity(entity).despawn();
        }
    }
}

fn log_inputs(
    state: Query<&MatchState>,
    humans: Query<(&Player, &Human, &CurrentInput)>,
    mut trace: ResMut<Trace>,
) {
    let playing = state.iter().any(|s| s.phase == Phase::Playing);
    if !playing {
        return;
    }
    let mut rows: Vec<(u64, PlayerInput)> = humans
        .iter()
        .filter(|(_, human, _)| human.connected() && !human.left)
        .map(|(player, _, input)| (player.client_id, input.0.clone()))
        .collect();
    rows.sort_unstable_by_key(|(id, _)| *id);
    for (id, input) in rows {
        trace.push(format!("input {id} {}", fmt_input(&input)));
    }
}

fn journal_flow(
    mut journal: ResMut<Journal>,
    mut trace: ResMut<Trace>,
    tick: Res<SimTick>,
    state: Query<&MatchState>,
    settings: Query<&RoomSettings>,
    players: Query<(&Player, &Pos, &Vitals, &Stats, &Evolution)>,
    outbox: Res<crate::gameplay::Outbox>,
    clock: Option<Res<LogClock>>,
) {
    let Some(state) = state.iter().next() else {
        return;
    };
    let settings = settings.iter().next();
    let room_seed = journal.room_seed;
    if let Some(settings) = settings {
        journal.note_room(RoomSnap {
            room_seed,
            code: settings.code.clone(),
            private: settings.private_room,
            bots: settings.bots,
            monsters: settings.monsters,
            host: settings.host,
            match_index: state.match_index,
            phase: state.phase,
        });
    }
    if state.phase == Phase::Playing && journal.open_index != Some(state.match_index) {
        let roster: Vec<RosterEntry> = players
            .iter()
            .map(|(player, ..)| RosterEntry {
                client_id: player.client_id,
                name: player.name.clone(),
                bot: player.bot,
                color: player.color,
            })
            .collect();
        if let Some(settings) = settings {
            journal.flush();
            journal.match_log = None;
            journal.sealed = false;
            journal.open_match(state.match_index, settings, &roster);
        }
    }
    let lines = std::mem::take(&mut trace.lines);
    if state.phase == Phase::Playing || state.phase == Phase::Results {
        for line in &lines {
            journal.record(tick.0, line);
        }
    }
    for event in &outbox.reliable {
        if let GameEvent::Kill {
            killer,
            victim,
            how,
            ..
        } = event
        {
            journal.kills.push(KillLine {
                tick: tick.0,
                killer: killer.clone(),
                victim: victim.clone(),
                how: how.clone(),
            });
        }
    }
    if journal.match_log.is_some()
        && tick.0 > 0
        && tick.0.is_multiple_of(DIGEST_EVERY)
        && matches!(state.phase, Phase::Playing | Phase::Results)
    {
        let mut rows: Vec<(u64, u32, u32, u32, u32, u32)> = players
            .iter()
            .map(|(player, pos, vitals, stats, evo)| {
                (
                    player.client_id,
                    pos.0.x.to_bits(),
                    pos.0.y.to_bits(),
                    vitals.hp.to_bits(),
                    stats.score,
                    evo.xp,
                )
            })
            .collect();
        rows.sort_unstable_by_key(|row| row.0);
        let mut bytes = Vec::with_capacity(rows.len() * 28);
        for (id, x, y, hp, score, xp) in rows {
            bytes.extend(id.to_le_bytes());
            bytes.extend(x.to_le_bytes());
            bytes.extend(y.to_le_bytes());
            bytes.extend(hp.to_le_bytes());
            bytes.extend(score.to_le_bytes());
            bytes.extend(xp.to_le_bytes());
        }
        journal.digest(tick.0, fnv1a64(&bytes));
    }
    journal.ticks = journal.ticks.wrapping_add(1);
    let now = clock.map(|clock| clock.0).unwrap_or_else(SystemTime::now);
    let left_results = journal.prev_phase == Some(Phase::Results) && state.phase != Phase::Results;
    let finished_here = state.phase == Phase::Results && state.ticks_left == 0;
    if (left_results || finished_here) && !journal.sealed {
        let scores: Vec<ScoreLine> = players
            .iter()
            .map(|(player, _, _, stats, evo)| ScoreLine {
                client_id: player.client_id,
                score: stats.score,
                kills: stats.kills,
                deaths: stats.deaths,
                xp: evo.xp,
            })
            .collect();
        // scores sorted inside seal via the passed vec — sort here.
        let mut scores = scores;
        scores.sort_by_key(|row| row.client_id);
        journal.seal(&scores, now);
    }
    journal.prev_phase = Some(state.phase);
    if journal.ticks.is_multiple_of(FLUSH_EVERY) {
        journal.flush();
    }
}

#[derive(Clone, Debug)]
struct Header {
    protocol: u64,
    build: String,
    room_seed: u64,
    match_index: u32,
    settings: RoomSettings,
    roster: Vec<RosterEntry>,
}

enum Event {
    Input(u64, PlayerInput),
    Cmd(u64, PlayerCommand),
    Inv(u64, InventoryMove),
    Room(u64, RoomCommand),
    Connect(u64),
    Disconnect(u64),
    Leave(u64),
}

struct Parsed {
    header: Header,
    events: BTreeMap<u32, Vec<Event>>,
    digests: BTreeMap<u32, u64>,
    scores: Option<Vec<ScoreLine>>,
    kills: Option<Vec<KillLine>>,
    max_tick: u32,
}

#[derive(Default)]
pub struct ReplayOptions {
    /// After this tick's systems, nudge one combatant. Not written to the log.
    pub nudge_after_tick: Option<u32>,
}

pub struct ReplayReport {
    pub ok: bool,
    pub mismatch_tick: Option<u32>,
    pub scores: Vec<ScoreLine>,
    pub kills: Vec<KillLine>,
    pub digests_checked: usize,
    pub error: String,
}

/// In-process replay. Prints nothing; [`replay_cli`] does.
pub fn replay_log(path: &Path, opts: ReplayOptions) -> ReplayReport {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            return ReplayReport {
                ok: false,
                mismatch_tick: None,
                scores: Vec::new(),
                kills: Vec::new(),
                digests_checked: 0,
                error: format!("cannot read {}: {err}", path.display()),
            };
        }
    };
    let parsed = match parse_log(&text) {
        Ok(parsed) => parsed,
        Err(err) => {
            return ReplayReport {
                ok: false,
                mismatch_tick: None,
                scores: Vec::new(),
                kills: Vec::new(),
                digests_checked: 0,
                error: err,
            };
        }
    };
    if parsed.header.protocol != PROTOCOL_ID || parsed.header.build != build_id() {
        return ReplayReport {
            ok: false,
            mismatch_tick: None,
            scores: Vec::new(),
            kills: Vec::new(),
            digests_checked: 0,
            error: format!(
                "replay rejected: protocol {} build {} (this binary is protocol {PROTOCOL_ID} build {})",
                parsed.header.protocol,
                parsed.header.build,
                build_id()
            ),
        };
    }
    let mut app = boot_playing(&parsed.header);
    let mut checked = 0;
    let mut tick = 0;
    while tick < parsed.max_tick {
        let next = tick + 1;
        if let Some(events) = parsed.events.get(&next) {
            let mut injected = app.world_mut().resource_mut::<Injected>();
            for event in events {
                match event {
                    Event::Input(id, input) => injected.inputs.push((*id, input.clone())),
                    Event::Cmd(id, cmd) => injected.commands.push((*id, cmd.clone())),
                    Event::Inv(id, mov) => injected.moves.push((*id, *mov)),
                    Event::Room(id, cmd) => injected.rooms.push((*id, cmd.clone())),
                    Event::Connect(id) => injected.connects.push(*id),
                    Event::Disconnect(id) => injected.disconnects.push(*id),
                    Event::Leave(id) => injected.leaves.push(*id),
                }
            }
        }
        app.world_mut().run_schedule(FixedUpdate);
        tick = app.world().resource::<SimTick>().0;
        if let Some(expected) = parsed.digests.get(&tick) {
            let got = combatant_digest(app.world_mut());
            checked += 1;
            if got != *expected {
                return ReplayReport {
                    ok: false,
                    mismatch_tick: Some(tick),
                    scores: scoreboard(app.world_mut()),
                    kills: app.world().resource::<Journal>().kills.clone(),
                    digests_checked: checked,
                    error: String::new(),
                };
            }
        }
        if opts.nudge_after_tick == Some(tick) {
            nudge(app.world_mut());
        }
    }
    let scores = scoreboard(app.world_mut());
    let kills = app.world().resource::<Journal>().kills.clone();
    if let Some(logged) = &parsed.scores
        && logged != &scores
    {
        return ReplayReport {
            ok: false,
            mismatch_tick: None,
            scores,
            kills,
            digests_checked: checked,
            error: "scores differ from the log".into(),
        };
    }
    if let Some(logged) = &parsed.kills
        && logged != &kills
    {
        return ReplayReport {
            ok: false,
            mismatch_tick: None,
            scores,
            kills,
            digests_checked: checked,
            error: "kill sequence differs from the log".into(),
        };
    }
    ReplayReport {
        ok: true,
        mismatch_tick: None,
        scores,
        kills,
        digests_checked: checked,
        error: String::new(),
    }
}

pub fn replay_cli(path: &Path) -> i32 {
    let report = replay_log(path, ReplayOptions::default());
    if report.mismatch_tick.is_none() && !report.error.is_empty() {
        eprintln!("{}", report.error);
        return 1;
    }
    for line in &report.scores {
        println!(
            "score {} {} kills {} deaths {} xp {}",
            line.client_id, line.score, line.kills, line.deaths, line.xp
        );
    }
    for kill in &report.kills {
        println!(
            "kill {} {} {} {}",
            kill.tick, kill.killer, kill.victim, kill.how
        );
    }
    println!("digests checked {}", report.digests_checked);
    if let Some(tick) = report.mismatch_tick {
        println!("digest mismatch at tick {tick}");
        return 1;
    }
    if !report.ok {
        eprintln!("{}", report.error);
        return 1;
    }
    0
}

fn nudge(world: &mut World) {
    let entity = {
        let mut query = world.query::<(Entity, &Player)>();
        let mut ids: Vec<(u64, Entity)> = query
            .iter(world)
            .map(|(entity, player)| (player.client_id, entity))
            .collect();
        ids.sort_unstable_by_key(|(id, _)| *id);
        ids.first().map(|(_, entity)| *entity)
    };
    if let Some(entity) = entity
        && let Some(mut pos) = world.get_mut::<Pos>(entity)
    {
        pos.0.x += 80.0;
    }
}

fn boot_playing(header: &Header) -> App {
    let mut config = blank_config(header);
    config.logging = false;
    let mut app = sim_app(config);
    let match_seed = mix(header.room_seed, header.match_index);
    {
        let world = app.world_mut();
        world.resource_mut::<Arena>().0 = evoclash_game::map::ArenaMap::generate(match_seed);
        world.resource::<SimRng>().reinit_match(match_seed);
        world.resource_mut::<SimTick>().0 = 0;
        let settings = header.settings.clone();
        let index = header.match_index;
        let mut states = world.query::<(Entity, &MatchState)>();
        let state_entity = states.iter(world).next().map(|(e, _)| e);
        if let Some(entity) = state_entity {
            if let Some(mut state) = world.get_mut::<MatchState>(entity) {
                state.phase = Phase::Playing;
                state.ticks_left = MATCH_TICKS;
                state.countdown_active = false;
                state.match_index = index;
                state.vault_in = FIRST_VAULT_TICKS;
                state.winner = None;
                state.map_seed = match_seed;
                state.boss_eta = 0;
            }
            if let Some(mut room) = world.get_mut::<RoomSettings>(entity) {
                *room = settings;
            }
        }
    }
    spawn_roster(&mut app, &header.roster);
    stand_and_shield(app.world_mut());
    app
}

fn spawn_roster(app: &mut App, roster: &[RosterEntry]) {
    {
        let mut commands = app.world_mut().commands();
        for entry in roster {
            let player = Player {
                client_id: entry.client_id,
                name: entry.name.clone(),
                color: entry.color,
                bot: entry.bot,
            };
            let entity = spawn_combatant(&mut commands, player, None);
            if entry.bot {
                commands
                    .entity(entity)
                    .insert((Bot::fresh(), crate::gameplay::Aggressors::default()));
            } else {
                commands.entity(entity).insert(Human {
                    peer: PeerId::Netcode(entry.client_id),
                    disconnected_ticks: 0,
                    joined_at: 0,
                    left: false,
                });
            }
        }
    }
    app.world_mut().flush();
}

fn stand_and_shield(world: &mut World) {
    let spots = world.resource::<Arena>().spawn_points.clone();
    let mut ids: Vec<(u64, Entity)> = {
        let mut query = world.query::<(Entity, &Player)>();
        query
            .iter(world)
            .map(|(entity, player)| (player.client_id, entity))
            .collect()
    };
    ids.sort_unstable_by_key(|(id, _)| *id);
    for (i, (_, entity)) in ids.iter().enumerate() {
        if let Some(mut pos) = world.get_mut::<Pos>(*entity)
            && !spots.is_empty()
        {
            pos.0 = spots[i % spots.len()];
        }
        if let Some(mut vitals) = world.get_mut::<Vitals>(*entity) {
            *vitals = Vitals {
                shield: SPAWN_SHIELD_TICKS,
                ..Vitals::default()
            };
        }
        if let Some(mut dash) = world.get_mut::<Dash>(*entity) {
            *dash = Dash::default();
        }
    }
}

fn blank_config(header: &Header) -> ServerConfig {
    ServerConfig {
        port: 0,
        public_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        match_id: "replay".into(),
        max_players: ROOM_COMBATANTS as usize,
        min_combatants: ROOM_COMBATANTS as usize,
        matchmakers: Vec::new(),
        single_match: false,
        monsters: header.settings.monsters,
        threads: 1,
        agones: false,
        room_seed: header.room_seed,
        private: header.settings.private_room,
        room_code: header.settings.code.clone(),
        log_dir: std::env::temp_dir(),
        replay: None,
        logging: false,
    }
}

fn add_sim_plugins(app: &mut App) {
    app.add_plugins(bevy::state::app::StatesPlugin);
    app.add_plugins(lightyear::prelude::server::ServerPlugins {
        tick_duration: evoclash_game::net::tick_duration(),
    });
    app.insert_resource(lightyear::prelude::ReplicationMetadata::new(
        evoclash_game::net::REPLICATION_INTERVAL,
    ));
    app.add_plugins(evoclash_game::SharedPlugin);
}

fn sim_app(config: ServerConfig) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(config);
    add_sim_plugins(&mut app);
    app.add_plugins((
        crate::gameplay::GameplayPlugin,
        crate::combat::CombatPlugin,
        crate::inventory::InventoryPlugin,
        crate::environment::EnvironmentPlugin,
        crate::bots::BotsPlugin,
        crate::monsters::MonstersPlugin,
        crate::boss::BossPlugin,
        crate::evolution::EvolutionPlugin,
        crate::skills::SkillsPlugin,
        JournalPlugin,
    ));
    app.finish();
    app.world_mut().run_schedule(Startup);
    app
}

fn parse_log(text: &str) -> Result<Parsed, String> {
    let mut lines = text.lines().filter(|line| !line.is_empty() || true);
    let magic = lines.next().unwrap_or("");
    if magic != "EVOCLASH match v1" {
        return Err(format!("not a match log: {magic}"));
    }
    let mut protocol = None;
    let mut build = None;
    let mut room_seed = None;
    let mut match_index = None;
    let mut settings = None;
    let mut roster = Vec::new();
    let mut events: BTreeMap<u32, Vec<Event>> = BTreeMap::new();
    let mut digests = BTreeMap::new();
    let mut scores = None;
    let mut kills = None;
    let mut max_tick = 0;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("protocol ") {
            protocol = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("build ") {
            build = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("room_seed ") {
            room_seed = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("match_index ") {
            match_index = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("settings ") {
            settings = parse_settings(rest);
        } else if let Some(rest) = line.strip_prefix("player ") {
            if let Some(entry) = parse_player(rest) {
                roster.push(entry);
            }
        } else if let Some(rest) = line.strip_prefix("t ") {
            let mut parts = rest.splitn(3, ' ');
            let tick: u32 = parts
                .next()
                .unwrap_or("0")
                .parse()
                .map_err(|_| format!("bad tick in {line}"))?;
            let kind = parts.next().unwrap_or("");
            let body = parts.next().unwrap_or("");
            max_tick = max_tick.max(tick);
            if kind == "digest" {
                let hash = u64::from_str_radix(body.trim(), 16).map_err(|_| "bad digest")?;
                digests.insert(tick, hash);
            } else if let Some(event) = parse_event(kind, body) {
                events.entry(tick).or_default().push(event);
            }
        } else if let Some(rest) = line.strip_prefix("scores ") {
            scores = Some(parse_scores(rest));
        } else if let Some(rest) = line.strip_prefix("kills ") {
            kills = Some(parse_kills(rest));
        }
    }
    let protocol = protocol.ok_or("missing protocol")?;
    let build = build.ok_or("missing build")?;
    let room_seed = room_seed.ok_or("missing room_seed")?;
    let match_index = match_index.ok_or("missing match_index")?;
    let settings = settings.ok_or("missing settings")?;
    roster.sort_by_key(|e| e.client_id);
    Ok(Parsed {
        header: Header {
            protocol,
            build,
            room_seed,
            match_index,
            settings,
            roster,
        },
        events,
        digests,
        scores,
        kills,
        max_tick,
    })
}

fn parse_settings(rest: &str) -> Option<RoomSettings> {
    let mut parts = rest.split_whitespace();
    let bots = match parts.next()? {
        "off" => None,
        n => Some(n.parse().ok()?),
    };
    let monsters = parts.next()? == "1";
    let host = parts.next()?.parse().ok()?;
    let private_room = parts.next()? == "1";
    let code = parts.next().unwrap_or("-");
    Some(RoomSettings {
        bots,
        monsters,
        host,
        private_room,
        code: if code == "-" {
            String::new()
        } else {
            code.to_string()
        },
    })
}

fn parse_player(rest: &str) -> Option<RosterEntry> {
    let mut parts = rest.splitn(6, ' ');
    let client_id = parts.next()?.parse().ok()?;
    let bot = parts.next()? == "1";
    let r = f32::from_bits(parts.next()?.parse().ok()?);
    let g = f32::from_bits(parts.next()?.parse().ok()?);
    let b = f32::from_bits(parts.next()?.parse().ok()?);
    let name = parts.next().unwrap_or("").to_string();
    Some(RosterEntry {
        client_id,
        name,
        bot,
        color: [r, g, b],
    })
}

fn parse_event(kind: &str, body: &str) -> Option<Event> {
    match kind {
        "input" => parse_input(body).map(|(id, input)| Event::Input(id, input)),
        "cmd" => parse_cmd(body).map(|(id, cmd)| Event::Cmd(id, cmd)),
        "inv" => {
            let mut parts = body.split_whitespace();
            let id = parts.next()?.parse().ok()?;
            let from = parts.next()?.parse().ok()?;
            let to = parts.next()?.parse().ok()?;
            Some(Event::Inv(id, InventoryMove { from, to }))
        }
        "room" => parse_room(body).map(|(id, cmd)| Event::Room(id, cmd)),
        "connect" => Some(Event::Connect(body.trim().parse().ok()?)),
        "disconnect" => Some(Event::Disconnect(body.trim().parse().ok()?)),
        "leave" => Some(Event::Leave(body.trim().parse().ok()?)),
        _ => None,
    }
}

fn parse_input(body: &str) -> Option<(u64, PlayerInput)> {
    let mut p = body.split_whitespace();
    let id = p.next()?.parse().ok()?;
    let move_x = p.next()?.parse().ok()?;
    let move_y = p.next()?.parse().ok()?;
    let aim_x = f32::from_bits(p.next()?.parse().ok()?);
    let aim_y = f32::from_bits(p.next()?.parse().ok()?);
    let mut flag = || p.next().is_some_and(|s| s == "1");
    let fire = flag();
    let dash = flag();
    let interact = flag();
    let throw = flag();
    let skill_a = flag();
    let skill_b = flag();
    let select = p.next()?.parse().ok()?;
    Some((
        id,
        PlayerInput {
            move_x,
            move_y,
            aim: Vec2::new(aim_x, aim_y),
            fire,
            dash,
            interact,
            throw,
            skill_a,
            skill_b,
            select,
        },
    ))
}

fn by_name<T: Copy + std::fmt::Debug>(all: &[T], name: &str) -> Option<T> {
    all.iter().copied().find(|item| format!("{item:?}") == name)
}

fn parse_cmd(body: &str) -> Option<(u64, PlayerCommand)> {
    let mut parts = body.splitn(3, ' ');
    let id = parts.next()?.parse().ok()?;
    let verb = parts.next()?;
    let arg = parts.next().unwrap_or("");
    let cmd = match verb {
        "spend" => PlayerCommand::Spend(by_name(&Perk::ALL, arg)?),
        "refund" => PlayerCommand::Refund(by_name(&Perk::ALL, arg)?),
        "reset" => PlayerCommand::ResetPerks,
        "buy" => PlayerCommand::Buy(by_name(&ShopItem::STOCK, arg).or_else(|| {
            [ShopItem::Medkit, ShopItem::Ammo]
                .into_iter()
                .find(|item| format!("{item:?}") == arg)
        })?),
        "unbuy" => PlayerCommand::Unbuy(by_name(&ShopItem::STOCK, arg)?),
        "deploy" => PlayerCommand::Deploy,
        "deploy_last" => PlayerCommand::DeployLast,
        _ => return None,
    };
    Some((id, cmd))
}

fn parse_room(body: &str) -> Option<(u64, RoomCommand)> {
    let mut parts = body.split_whitespace();
    let id = parts.next()?.parse().ok()?;
    let verb = parts.next()?;
    let cmd = match verb {
        "start" => RoomCommand::Start,
        "cancel" => RoomCommand::Cancel,
        "leave" => RoomCommand::Leave,
        "settings" => {
            let bots = match parts.next()? {
                "off" => None,
                n => Some(n.parse().ok()?),
            };
            let monsters = parts.next()? == "1";
            RoomCommand::SetSettings { bots, monsters }
        }
        _ => return None,
    };
    Some((id, cmd))
}

fn parse_scores(rest: &str) -> Vec<ScoreLine> {
    rest.split_whitespace()
        .filter_map(|part| {
            let mut n = part.split(':');
            Some(ScoreLine {
                client_id: n.next()?.parse().ok()?,
                score: n.next()?.parse().ok()?,
                kills: n.next()?.parse().ok()?,
                deaths: n.next()?.parse().ok()?,
                xp: n.next()?.parse().ok()?,
            })
        })
        .collect()
}

fn parse_kills(rest: &str) -> Vec<KillLine> {
    if rest.trim().is_empty() {
        return Vec::new();
    }
    rest.split(';')
        .filter_map(|part| {
            let n = split_esc(part);
            let tick = n.first()?.parse().ok()?;
            Some(KillLine {
                tick,
                killer: unesc(n.get(1)?),
                victim: unesc(n.get(2)?),
                how: unesc(n.get(3).map(|s| s.as_str()).unwrap_or("")),
            })
        })
        .collect()
}

fn split_esc(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                cur.push(next);
            }
        } else if c == '|' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::color_for;
    use crate::gameplay::{PendingStart, ResetRequested};
    use std::time::Duration;

    #[derive(Resource)]
    struct Arm {
        need: usize,
    }

    fn arm_playing(
        mut done: Local<bool>,
        arm: Res<Arm>,
        players: Query<&Player>,
        mut state: Query<&mut MatchState>,
        mut reset: ResMut<ResetRequested>,
        mut place: ResMut<PendingStart>,
    ) {
        if *done || players.iter().count() < arm.need {
            return;
        }
        let Ok(mut state) = state.single_mut() else {
            return;
        };
        if state.phase != Phase::Lobby {
            return;
        }
        state.phase = Phase::Starting;
        state.ticks_left = 2;
        state.countdown_active = false;
        reset.0 = true;
        place.0 = true;
        *done = true;
    }

    fn test_config(dir: &Path, seed: u64, monsters: bool) -> ServerConfig {
        ServerConfig {
            port: 0,
            public_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            match_id: "match".into(),
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
            log_dir: dir.to_path_buf(),
            replay: None,
            logging: true,
        }
    }

    fn recording_app(config: ServerConfig, need: usize) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(config);
        add_sim_plugins(&mut app);
        app.insert_resource(Arm { need });
        app.add_plugins((
            crate::gameplay::GameplayPlugin,
            crate::combat::CombatPlugin,
            crate::inventory::InventoryPlugin,
            crate::environment::EnvironmentPlugin,
            crate::bots::BotsPlugin,
            crate::monsters::MonstersPlugin,
            crate::boss::BossPlugin,
            crate::evolution::EvolutionPlugin,
            crate::skills::SkillsPlugin,
            JournalPlugin,
        ));
        app.add_systems(
            FixedUpdate,
            arm_playing.before(match_flow).in_set(SimSet::Flow),
        );
        app.finish();
        app.world_mut().run_schedule(Startup);
        app
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "evoclash-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn phase_of(app: &mut App) -> Phase {
        let world = app.world_mut();
        let mut q = world.query::<&MatchState>();
        q.iter(world).next().unwrap().phase
    }

    fn tick_of(app: &App) -> u32 {
        app.world().resource::<SimTick>().0
    }

    struct Played {
        path: PathBuf,
        scores: Vec<ScoreLine>,
        kills: Vec<KillLine>,
        playing_ticks: u32,
    }

    fn play(mut app: App, playing_goal: u32, humans: Option<(u64, u64)>) -> Played {
        let mut moved = false;
        let mut playing_ticks = 0;
        for _ in 0..playing_goal + 8_000 {
            if let Some((a, b)) = humans {
                let tick = tick_of(&app);
                let playing = phase_of(&mut app) == Phase::Playing;
                let mut injected = app.world_mut().resource_mut::<Injected>();
                injected.inputs.push((a, scripted(1, tick)));
                injected.inputs.push((b, scripted(-1, tick)));
                if !moved && playing {
                    injected.moves.push((a, InventoryMove { from: 0, to: 1 }));
                    moved = true;
                }
            }
            app.world_mut().run_schedule(FixedUpdate);
            let phase = phase_of(&mut app);
            let tick = tick_of(&app);
            if phase == Phase::Playing {
                playing_ticks = playing_ticks.max(tick);
            }
            if playing_ticks >= playing_goal {
                break;
            }
            if phase == Phase::Results {
                break;
            }
        }
        app.world_mut().resource_mut::<Journal>().flush();
        let path = app
            .world()
            .resource::<Journal>()
            .match_path
            .clone()
            .expect("match log");
        let kills = app.world().resource::<Journal>().kills.clone();
        let scores = scoreboard(app.world_mut());
        drop(app);
        Played {
            path,
            scores,
            kills,
            playing_ticks,
        }
    }

    fn scripted(move_x: i8, tick: u32) -> PlayerInput {
        PlayerInput {
            move_x,
            move_y: if tick % 128 < 64 { 1 } else { -1 },
            aim: Vec2::X,
            fire: tick.is_multiple_of(5),
            ..default()
        }
    }

    fn spawn_humans(app: &mut App, ids: &[u64]) {
        let mut commands = app.world_mut().commands();
        for id in ids {
            let player = Player {
                client_id: *id,
                name: format!("p{id}"),
                color: color_for(*id),
                bot: false,
            };
            let entity = spawn_combatant(&mut commands, player, None);
            commands.entity(entity).insert(Human {
                peer: PeerId::Netcode(*id),
                disconnected_ticks: 0,
                joined_at: 0,
                left: false,
            });
        }
        app.world_mut().flush();
    }

    fn assert_replay(played: &Played) {
        assert!(
            played.playing_ticks >= 1280,
            "only {} playing ticks",
            played.playing_ticks
        );
        let report = replay_log(&played.path, ReplayOptions::default());
        assert!(
            report.ok,
            "replay failed at {:?} {}",
            report.mismatch_tick, report.error
        );
        assert!(
            report.digests_checked >= 2,
            "digests {}",
            report.digests_checked
        );
        assert_eq!(report.scores, played.scores);
        assert_eq!(report.kills, played.kills);
    }

    #[test]
    fn replay_bot_match_matches_scores_kills_and_digests() {
        let dir = temp("bots");
        let app = recording_app(test_config(&dir, 0xB07, true), 6);
        let played = play(app, 1280, None);
        assert_replay(&played);
    }

    #[test]
    fn replay_two_humans_matches_scores_kills_and_digests() {
        let dir = temp("humans");
        let mut app = recording_app(test_config(&dir, 0xA11, false), 2);
        spawn_humans(&mut app, &[11, 22]);
        let played = play(app, 1280, Some((11, 22)));
        let text = fs::read_to_string(&played.path).unwrap();
        assert!(
            text.contains(" inv "),
            "inventory move was not logged: {text}"
        );
        assert_replay(&played);
    }

    #[test]
    fn replay_names_the_first_mismatched_digest_tick() {
        let dir = temp("poison");
        let app = recording_app(test_config(&dir, 0xB07, true), 6);
        let played = play(app, 1280, None);
        let report = replay_log(
            &played.path,
            ReplayOptions {
                nudge_after_tick: Some(100),
            },
        );
        assert!(!report.ok);
        assert_eq!(report.mismatch_tick, Some(640));
    }

    #[test]
    fn purge_drops_old_sealed_logs_and_keeps_unsealed_ones() {
        let dir = temp("purge");
        let old = dir.join("old.log");
        let young = dir.join("young.log");
        let open = dir.join("open.log");
        fs::write(&old, "EVOCLASH match v1\nend 1000\n").unwrap();
        fs::write(&young, "EVOCLASH match v1\nend 5000\n").unwrap();
        fs::write(
            &open,
            "EVOCLASH match v1\nt 1 input 1 0 0 0 0 0 0 0 0 0 0 0\n",
        )
        .unwrap();
        let now = UNIX_EPOCH + Duration::from_secs(5000 + RETAIN_SECS);
        purge_logs(&dir, now);
        assert!(!old.exists(), "sealed log older than 16 min should go");
        assert!(young.exists(), "a log sealed exactly 16 min ago stays");
        assert!(open.exists(), "an unsealed log stays");
        purge_logs(&dir, now + Duration::from_secs(1));
        assert!(!young.exists());
        assert!(open.exists());
    }

    #[test]
    fn startup_purges_with_the_clock_resource() {
        let dir = temp("startup-purge");
        fs::write(dir.join("stale.log"), "end 1\n").unwrap();
        fs::write(dir.join("live.log"), "t 1 digest 0\n").unwrap();
        let mut config = test_config(&dir, 1, false);
        config.logging = true;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(config);
        app.insert_resource(LogClock(UNIX_EPOCH + Duration::from_secs(RETAIN_SECS + 30)));
        app.init_resource::<evoclash_game::sim::Obstacles>();
        app.add_plugins((crate::gameplay::GameplayPlugin, JournalPlugin));
        app.finish();
        app.world_mut().run_schedule(Startup);
        assert!(!dir.join("stale.log").exists());
        assert!(dir.join("live.log").exists());
    }

    #[test]
    #[ignore]
    fn replay_eight_minutes() {
        let dir = temp("eight");
        let app = recording_app(test_config(&dir, 0xB07, true), 6);
        let played = play(app, 8 * 60 * 64, None);
        let started = std::time::Instant::now();
        let report = replay_log(&played.path, ReplayOptions::default());
        let elapsed = started.elapsed();
        assert!(
            report.ok,
            "replay failed at {:?} {}",
            report.mismatch_tick, report.error
        );
        assert!(elapsed < Duration::from_secs(15), "replay took {elapsed:?}");
        eprintln!("replay elapsed {elapsed:?}");
    }
}
