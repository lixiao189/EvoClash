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
        recover: false,
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
            ..Default::default()
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

#[derive(Clone, PartialEq, Debug)]
struct Body {
    id: u64,
    x: u32,
    y: u32,
    score: u32,
    level: u8,
    loadout: Loadout,
}

fn bodies(world: &mut World) -> Vec<Body> {
    let mut query = world.query::<(&Player, &Pos, &Stats, &Evolution, &Loadout)>();
    let mut rows: Vec<Body> = query
        .iter(world)
        .map(|(player, pos, stats, evo, loadout)| Body {
            id: player.client_id,
            x: pos.0.x.to_bits(),
            y: pos.0.y.to_bits(),
            score: stats.score,
            level: evo.level,
            loadout: *loadout,
        })
        .collect();
    rows.sort_by_key(|row| row.id);
    rows
}

fn truncate_log(path: &Path, tick: u32) {
    let text = fs::read_to_string(path).unwrap();
    let mut out = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("t ") {
            let n: u32 = rest
                .split_whitespace()
                .next()
                .unwrap_or("0")
                .parse()
                .unwrap();
            if n > tick {
                continue;
            }
        }
        if line.starts_with("scores ") || line.starts_with("kills ") || line.starts_with("end ") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    fs::write(path, out).unwrap();
}

fn fresh_app(dir: &Path, seed: u64) -> App {
    recording_app(test_config(dir, seed, false), 99)
}

#[test]
fn recover_matches_the_truncated_tick_and_a_second_pass() {
    let dir = temp("recover");
    let mut app = fresh_app(&dir, 0xA11);
    spawn_humans(&mut app, &[11, 22]);
    let mut snap = None;
    for _ in 0..8_000 {
        let tick = tick_of(&app);
        if phase_of(&mut app) == Phase::Playing {
            let mut injected = app.world_mut().resource_mut::<Injected>();
            injected.inputs.push((11, scripted(1, tick)));
            injected.inputs.push((22, scripted(-1, tick)));
        }
        app.world_mut().run_schedule(FixedUpdate);
        let tick = tick_of(&app);
        if phase_of(&mut app) == Phase::Playing && tick == 640 {
            snap = Some(bodies(app.world_mut()));
        }
        if tick >= 700 && phase_of(&mut app) == Phase::Playing {
            break;
        }
    }
    let snap = snap.expect("tick 640");
    app.world_mut().resource_mut::<Journal>().flush();
    let path = app
        .world()
        .resource::<Journal>()
        .match_path
        .clone()
        .unwrap();
    drop(app);
    truncate_log(&path, 640);

    let mut recovered = fresh_app(&dir, 0xA11);
    super::recover::recover_into(recovered.world_mut()).expect("recover");
    assert_eq!(phase_of(&mut recovered), Phase::Recovering);
    assert_eq!(bodies(recovered.world_mut()), snap);
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("\nrecover 640\n") || text.contains("recover 640\n"),
        "{text}"
    );
    assert!(text.contains("t 640 freeze"), "{text}");

    // Grace is 20s from now, not whatever disconnect time the log had reached.
    {
        let world = recovered.world_mut();
        let mut humans = world.query::<&crate::connections::Human>();
        for human in humans.iter(world) {
            assert_eq!(human.disconnected_ticks, 1);
        }
        let (frozen_clock, frozen_vault, frozen_eta) = {
            let mut states = world.query::<&mut MatchState>();
            let mut state = states.iter_mut(world).next().unwrap();
            assert_eq!(
                state.recover.ticks_left,
                20 * crate::gameplay::TICKS_PER_SEC
            );
            assert!(!state.recover.resuming);
            let frozen_clock = state.ticks_left;
            let frozen_vault = state.vault_in;
            let frozen_eta = state.boss_eta;
            state.boss_eta = 1234;
            (frozen_clock, frozen_vault, frozen_eta)
        };
        let mut idles = world.query::<&mut crate::gameplay::Idle>();
        for mut idle in idles.iter_mut(world) {
            idle.set_count(40);
        }
        let saved = {
            let mut dead = world.query::<(&Player, &Vitals)>();
            dead.iter(world)
                .find(|(player, _)| player.client_id == 11)
                .map(|(_, vitals)| *vitals)
                .unwrap()
        };
        {
            let mut dead = world.query::<(&Player, &mut Vitals)>();
            let mut armed = false;
            for (player, mut vitals) in dead.iter_mut(world) {
                if player.client_id == 11 {
                    vitals.alive = false;
                    vitals.deploy_in = 80;
                    armed = true;
                }
            }
            assert!(armed);
        }
        for _ in 0..30 {
            world.run_schedule(FixedUpdate);
        }
        let state = world.query::<&MatchState>().iter(world).next().unwrap();
        assert_eq!(state.phase, Phase::Recovering);
        assert_eq!(state.ticks_left, frozen_clock);
        assert_eq!(state.vault_in, frozen_vault);
        assert_eq!(state.boss_eta, 1234);
        assert_eq!(
            state.recover.ticks_left,
            20 * crate::gameplay::TICKS_PER_SEC - 30
        );
        assert!(
            world
                .query::<&crate::gameplay::Idle>()
                .iter(world)
                .all(|idle| idle.ticks() == 40)
        );
        let vitals = world
            .query::<(&Player, &Vitals)>()
            .iter(world)
            .find(|(player, _)| player.client_id == 11)
            .unwrap()
            .1;
        assert!(!vitals.alive);
        assert_eq!(vitals.deploy_in, 80);
        // Those pokes are not in the log. Put the body back before play continues.
        let mut dead = world.query::<(&Player, &mut Vitals)>();
        for (player, mut vitals) in dead.iter_mut(world) {
            if player.client_id == 11 {
                *vitals = saved;
            }
        }
        let mut idles = world.query::<&mut crate::gameplay::Idle>();
        for mut idle in idles.iter_mut(world) {
            idle.set_count(0);
        }
        let mut states = world.query::<&mut MatchState>();
        if let Some(mut state) = states.iter_mut(world).next() {
            state.boss_eta = frozen_eta;
        }
    }
    {
        let world = recovered.world_mut();
        let mut humans = world.query::<&mut crate::connections::Human>();
        for mut human in humans.iter_mut(world) {
            human.disconnected_ticks = 0;
        }
    }
    let mut resumed_at = None;
    for _ in 0..20_000 {
        recovered.world_mut().run_schedule(FixedUpdate);
        let tick = tick_of(&recovered);
        if phase_of(&mut recovered) == Phase::Playing {
            resumed_at = Some(tick);
            break;
        }
    }
    let resumed_at = resumed_at.expect("resumed");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains(" thaw"), "freeze/thaw missing: {text}");
    for _ in 0..8_000 {
        let tick = tick_of(&recovered);
        if phase_of(&mut recovered) == Phase::Playing {
            let mut injected = recovered.world_mut().resource_mut::<Injected>();
            injected.inputs.push((11, scripted(1, tick + 1)));
            injected.inputs.push((22, scripted(-1, tick + 1)));
        }
        recovered.world_mut().run_schedule(FixedUpdate);
        let tick = tick_of(&recovered);
        if phase_of(&mut recovered) == Phase::Playing
            && tick > resumed_at
            && tick.is_multiple_of(640)
        {
            break;
        }
    }
    recovered.world_mut().resource_mut::<Journal>().flush();
    let cut = tick_of(&recovered);
    let snap2 = bodies(recovered.world_mut());
    drop(recovered);
    truncate_log(&path, cut);

    let mut again = fresh_app(&dir, 0xA11);
    super::recover::recover_into(again.world_mut()).expect("second recover");
    assert_eq!(phase_of(&mut again), Phase::Recovering);
    assert_eq!(bodies(again.world_mut()), snap2);
}

#[test]
fn a_changed_input_abandons_with_the_tick() {
    let dir = temp("poison-recover");
    let mut app = fresh_app(&dir, 0xB07);
    spawn_humans(&mut app, &[11, 22]);
    let played = play(app, 700, Some((11, 22)));
    truncate_log(&played.path, 640);
    let text = fs::read_to_string(&played.path).unwrap();
    let mut changed = 0;
    let poisoned: String = text
        .lines()
        .map(|line| {
            let mut parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 8 && parts[2] == "input" {
                changed += 1;
                parts[4] = "0";
                parts[5] = "-1";
                parts[8] = "0";
                return parts.join(" ");
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(changed > 10, "inputs {changed}");
    fs::write(&played.path, format!("{poisoned}\n")).unwrap();
    let mut recovered = fresh_app(&dir, 0xB07);
    let err = super::recover::recover_into(recovered.world_mut()).unwrap_err();
    assert!(err.contains("tick"), "{err}");
    assert!(err.contains("640"), "{err}");
}

#[test]
fn lobby_and_results_restore_the_room() {
    let dir = temp("lobby-recover");
    let mut app = fresh_app(&dir, 42);
    let room = dir.join("room-match.log");
    fs::write(
        &room,
        "EVOCLASH room v1\nroom_seed=42 code=ABCD private=1 bots=8 monsters=0 host=7 match_index=3 phase=lobby paused=1\n",
    )
    .unwrap();
    super::recover::recover_into(app.world_mut()).unwrap();
    let world = app.world_mut();
    let state = world.query::<&MatchState>().iter(world).next().unwrap();
    assert_eq!(state.phase, Phase::Lobby);
    assert_eq!(state.match_index, 3);
    assert!(!state.countdown_active);
    assert_eq!(state.ticks_left, 0);
    let settings = world.query::<&RoomSettings>().iter(world).next().unwrap();
    assert_eq!(settings.host, 7);
    assert_eq!(settings.code, "ABCD");
    assert!(settings.private_room);
    assert!(settings.auto_paused);
    assert!(!settings.monsters);
    assert_eq!(settings.bots, Some(8));
    drop(app);

    let mut app = fresh_app(&dir, 42);
    fs::write(
        &room,
        "EVOCLASH room v1\nroom_seed=42 code=- private=0 bots=8 monsters=1 host=9 match_index=4 phase=results paused=0\n",
    )
    .unwrap();
    super::recover::recover_into(app.world_mut()).unwrap();
    let world = app.world_mut();
    let state = world.query::<&MatchState>().iter(world).next().unwrap();
    assert_eq!(state.phase, Phase::Lobby);
    assert_eq!(state.match_index, 5);
    assert!(state.countdown_active);
    assert_eq!(state.ticks_left, crate::gameplay::LOBBY_COUNTDOWN_TICKS);
    assert_eq!(state.map_seed, evoclash_game::rng::mix(42, 5));
    let settings = world.query::<&RoomSettings>().iter(world).next().unwrap();
    assert_eq!(settings.host, 9);
    assert!(settings.code.is_empty());
    assert!(!settings.private_room);
}
