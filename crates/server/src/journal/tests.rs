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
        commands.entity(entity).insert((
            Human {
                peer: PeerId::Netcode(*id),
                disconnected_ticks: 0,
                joined_at: 0,
                left: false,
            },
            crate::gameplay::Idle::default(),
        ));
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
    let app = recording_app(test_config(&dir, 0xB07, true), ROOM_COMBATANTS as usize);
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
    let app = recording_app(test_config(&dir, 0xB07, true), ROOM_COMBATANTS as usize);
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
fn score_line_keeps_the_last_score_tick() {
    let line = ScoreLine {
        client_id: 7,
        score: 4,
        kills: 3,
        deaths: 1,
        xp: 9,
        last_score_tick: 128,
    };
    let text = format!(
        "{}:{}:{}:{}:{}:{}",
        line.client_id, line.score, line.kills, line.deaths, line.xp, line.last_score_tick
    );
    assert_eq!(parse_scores(&text), vec![line]);
}

#[test]
#[ignore]
fn replay_eight_minutes() {
    let dir = temp("eight");
    let app = recording_app(test_config(&dir, 0xB07, true), ROOM_COMBATANTS as usize);
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

#[test]
fn replay_keeps_a_watcher_and_ignores_their_input() {
    let dir = temp("watch");
    let mut app = recording_app(test_config(&dir, 0x5A7, false), 2);
    spawn_humans(&mut app, &[11, 22]);
    let mut playing_ticks = 0;
    let mut joined = false;
    let mut switched = false;
    for _ in 0..8_000 {
        let tick = tick_of(&app);
        let playing = phase_of(&mut app) == Phase::Playing;
        if playing && tick == 10 && !joined {
            let mut seq = crate::spectate::SpectateSeq(
                app.world().resource::<crate::spectate::SpectateSeq>().0,
            );
            {
                let mut commands = app.world_mut().commands();
                crate::spectate::spawn_spectator(
                    &mut commands,
                    &mut seq,
                    None,
                    crate::spectate::NewSpectator {
                        client_id: 90,
                        name: "eye".into(),
                        peer: PeerId::Netcode(90),
                        link: Entity::PLACEHOLDER,
                        order: 1,
                        following: 0,
                        queue_pos: 1,
                        watched: false,
                        replicate: false,
                        log: false,
                    },
                );
            }
            app.world_mut().flush();
            app.world_mut()
                .resource_mut::<crate::spectate::SpectateSeq>()
                .0 = seq.0;
            app.world_mut()
                .resource_mut::<Trace>()
                .push("spectate 90 1 0 eye".to_string());
            joined = true;
        }
        {
            let mut injected = app.world_mut().resource_mut::<Injected>();
            injected.inputs.push((
                90,
                PlayerInput {
                    move_x: 1,
                    fire: true,
                    ..default()
                },
            ));
            injected.inputs.push((11, scripted(1, tick)));
            injected.inputs.push((22, scripted(-1, tick)));
            if playing && tick == 39 && !switched {
                injected.rooms.push((90, RoomCommand::SpectateNext));
                switched = true;
            }
        }
        app.world_mut().run_schedule(FixedUpdate);
        if phase_of(&mut app) == Phase::Playing {
            playing_ticks = playing_ticks.max(tick_of(&app));
        }
        if playing_ticks >= 700 {
            break;
        }
    }
    assert!(joined && switched, "never reached the follow switch");
    app.world_mut().resource_mut::<Journal>().flush();
    let rows = crate::spectate::spectator_rows(app.world_mut());
    assert_eq!(rows.len(), 1);
    assert_ne!(rows[0].1, 0, "a watcher is following someone");
    let path = app
        .world()
        .resource::<Journal>()
        .match_path
        .clone()
        .unwrap();
    drop(app);
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("spectate 90 1 0 eye"),
        "join was not logged: {text}"
    );
    assert!(
        text.contains(" spectate_next"),
        "switch was not logged: {text}"
    );
    let report = replay_log(&path, ReplayOptions::default());
    assert!(
        report.ok,
        "replay failed at {:?} {}",
        report.mismatch_tick, report.error
    );
    assert!(report.digests_checked >= 1);
    assert_eq!(report.spectators, rows);
}
