use super::*;

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
    paused: bool,
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
            "room_seed={} code={code} private={} bots={bots} monsters={} host={} match_index={} phase={} paused={}",
            self.room_seed,
            u8::from(self.private),
            u8::from(self.monsters),
            self.host,
            self.match_index,
            phase_name(self.phase),
            u8::from(self.paused),
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

    fn open_match(
        &mut self,
        index: u32,
        settings: &RoomSettings,
        roster: &[RosterEntry],
        spectators: &[SpectatorEntry],
    ) {
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
        let mut spectators = spectators.to_vec();
        spectators.sort_by_key(|entry| (entry.order, entry.client_id));
        for entry in &spectators {
            let name = entry.name.replace(['\n', '\r'], " ");
            log.line(&format!(
                "spectator {} {} {} {name}",
                entry.client_id, entry.order, entry.following
            ));
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
        self.mark(tick, line);
    }

    /// Append a tick line even at tick 0. Used for freeze, thaw and departures during a wait.
    pub(crate) fn mark(&mut self, tick: u32, line: &str) {
        if self.sealed {
            return;
        }
        if let Some(log) = self.match_log.as_mut() {
            log.line(&format!("t {tick} {line}"));
        }
    }

    pub(crate) fn record_plain(&mut self, line: &str) {
        if let Some(log) = self.match_log.as_mut() {
            log.line(line);
        }
    }

    /// Keep writing the match log that catch-up just replayed, instead of truncating it.
    pub(crate) fn adopt_match(&mut self, index: u32, path: PathBuf) {
        self.open_index = Some(index);
        self.match_path = Some(path.clone());
        self.sealed = false;
        if self.logging {
            self.match_log = LogBuf::append(&path);
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
                    "{}:{}:{}:{}:{}:{}",
                    score.client_id,
                    score.score,
                    score.kills,
                    score.deaths,
                    score.xp,
                    score.last_score_tick
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
            last_score_tick: stats.last_score_tick,
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

pub(super) fn open_logs(
    mut commands: Commands,
    config: Res<ServerConfig>,
    clock: Option<Res<LogClock>>,
) {
    let now = clock.map(|clock| clock.0).unwrap_or_else(SystemTime::now);
    if config.logging {
        let _ = fs::create_dir_all(&config.log_dir);
        purge_logs(&config.log_dir, now);
    }
    commands.insert_resource(Journal::create(&config));
}

pub(super) fn apply_presence(
    mut commands: Commands,
    mut injected: ResMut<Injected>,
    state: Query<&MatchState>,
    mut departed: Query<&mut Departed>,
    mut humans: Query<(Entity, &mut Human, &Player, &Stats, &Evolution)>,
) {
    let connects = std::mem::take(&mut injected.connects);
    let disconnects = std::mem::take(&mut injected.disconnects);
    let leaves = std::mem::take(&mut injected.leaves);
    let playing = state.iter().any(|s| s.phase == Phase::Playing);
    for id in connects {
        if let Some((_, mut human, _, _, _)) = humans
            .iter_mut()
            .find(|(_, _, player, _, _)| player.client_id == id)
        {
            human.disconnected_ticks = 0;
            human.left = false;
        }
    }
    for id in disconnects {
        if let Some((_, mut human, _, _, _)) = humans
            .iter_mut()
            .find(|(_, _, player, _, _)| player.client_id == id)
            && human.connected()
        {
            human.disconnected_ticks = 1;
        }
    }
    let mut gone = Vec::new();
    for id in leaves {
        if let Some((entity, _, player, stats, evo)) = humans
            .iter()
            .find(|(_, _, player, _, _)| player.client_id == id)
        {
            if playing {
                gone.push(Departure::from_combatant(player, stats, evo));
            }
            commands.entity(entity).despawn();
        }
    }
    if !gone.is_empty()
        && let Ok(mut departed) = departed.single_mut()
    {
        departed.0.extend(gone);
    }
}

pub(super) fn log_inputs(
    state: Query<&MatchState>,
    humans: Query<(&Player, &Human, &CurrentInput)>,
    catching: Option<Res<CatchingUp>>,
    mut trace: ResMut<Trace>,
) {
    if catching.is_some() {
        return;
    }
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

pub(super) fn journal_flow(
    mut journal: ResMut<Journal>,
    mut trace: ResMut<Trace>,
    tick: Res<SimTick>,
    state: Query<&MatchState>,
    settings: Query<&RoomSettings>,
    catching: Option<Res<CatchingUp>>,
    players: Query<(&Player, &Pos, &Vitals, &Stats, &Evolution)>,
    watchers: Query<(&crate::spectate::SpectatorSession, &Spectator)>,
    outbox: Res<crate::gameplay::Outbox>,
    clock: Option<Res<LogClock>>,
) {
    if catching.is_some() {
        return;
    }
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
            paused: settings.auto_paused,
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
            let mut spectators: Vec<SpectatorEntry> = watchers
                .iter()
                .map(|(session, spec)| SpectatorEntry {
                    client_id: session.client_id,
                    order: session.order,
                    following: spec.following,
                    name: session.name.clone(),
                })
                .collect();
            spectators.sort_by_key(|entry| (entry.order, entry.client_id));
            journal.open_match(state.match_index, settings, &roster, &spectators);
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
                last_score_tick: stats.last_score_tick,
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
