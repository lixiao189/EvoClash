use super::parse::Event;
use super::*;

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
    /// `(client_id, following, queue_pos)` still watching at the last replayed tick.
    pub spectators: Vec<(u64, u64, u8)>,
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
                spectators: Vec::new(),
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
                spectators: Vec::new(),
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
            spectators: Vec::new(),
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
                    Event::Spectate(id, order, following, name) => {
                        injected
                            .spectates
                            .push((*id, *order, *following, name.clone()))
                    }
                    Event::Unspectate(id) => injected.unspectates.push(*id),
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
                    spectators: crate::spectate::spectator_rows(app.world_mut()),
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
            spectators: Vec::new(),
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
            spectators: Vec::new(),
            error: "kill sequence differs from the log".into(),
        };
    }
    ReplayReport {
        ok: true,
        mismatch_tick: None,
        scores,
        kills,
        digests_checked: checked,
        spectators: crate::spectate::spectator_rows(app.world_mut()),
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
            "score {} {} kills {} deaths {} xp {} tick {}",
            line.client_id, line.score, line.kills, line.deaths, line.xp, line.last_score_tick
        );
    }
    for kill in &report.kills {
        println!(
            "kill {} {} {} {}",
            kill.tick, kill.killer, kill.victim, kill.how
        );
    }
    for (id, following, queue_pos) in &report.spectators {
        println!("spectator {id} following {following} queue {queue_pos}");
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
    spawn_header_watchers(&mut app, &header.spectators);
    stand_and_shield(app.world_mut());
    app
}

fn spawn_header_watchers(app: &mut App, watchers: &[SpectatorEntry]) {
    if watchers.is_empty() {
        return;
    }
    let mut seq =
        crate::spectate::SpectateSeq(app.world().resource::<crate::spectate::SpectateSeq>().0);
    {
        let mut commands = app.world_mut().commands();
        for entry in watchers {
            crate::spectate::spawn_spectator(
                &mut commands,
                &mut seq,
                None,
                crate::spectate::NewSpectator {
                    client_id: entry.client_id,
                    name: entry.name.clone(),
                    peer: PeerId::Netcode(entry.client_id),
                    link: Entity::PLACEHOLDER,
                    order: entry.order,
                    following: entry.following,
                    queue_pos: 0,
                    watched: false,
                    replicate: false,
                    log: false,
                },
            );
        }
    }
    app.world_mut().flush();
    app.world_mut()
        .resource_mut::<crate::spectate::SpectateSeq>()
        .0 = seq.0;
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
                commands.entity(entity).insert((
                    Human {
                        peer: PeerId::Netcode(entry.client_id),
                        disconnected_ticks: 0,
                        joined_at: 0,
                        left: false,
                    },
                    crate::gameplay::Idle::default(),
                ));
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

pub(super) fn add_sim_plugins(app: &mut App) {
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
        crate::spectate::SpectatePlugin,
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
