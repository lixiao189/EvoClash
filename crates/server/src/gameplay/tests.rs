use evoclash_game::evolution::{EvoPath, Perk, xp_to_reach};
use evoclash_game::shop::{PlayerCommand, RESPAWN_MIN_TICKS, ShopItem, can_deploy};
use lightyear::prelude::PeerId;

use super::*;
use crate::connections::Human;
use crate::evolution::handle_command;

#[test]
fn evolution_resets_when_a_match_starts() {
    let mut app = App::new();
    app.init_resource::<LootState>()
        .init_resource::<PropSlots>()
        .init_resource::<Blasts>()
        .insert_resource(ResetRequested(true));
    app.add_systems(Update, reset_match);
    let mut evo = Evolution::default();
    evo.add_xp(500);
    evo.ranks[Perk::Claws.index()] = 2;
    let p = app
        .world_mut()
        .spawn((
            Player {
                client_id: 1,
                name: "p".into(),
                color: [1.0; 3],
                bot: false,
            },
            Vitals::default(),
            Loadout::default(),
            Stats {
                level: evo.level,
                ..default()
            },
            evo,
            SkillState {
                cd_a: 50,
                ..default()
            },
            Wallet {
                scrap: 80,
                ..default()
            },
            LifeLog {
                kills: 3,
                ..default()
            },
        ))
        .id();
    app.update();
    let world = app.world();
    assert_eq!(world.get::<Evolution>(p).unwrap(), &Evolution::default());
    assert_eq!(world.get::<SkillState>(p).unwrap(), &SkillState::default());
    assert_eq!(world.get::<Stats>(p).unwrap().level, 1);
    assert_eq!(world.get::<Wallet>(p).unwrap(), &Wallet::default());
    assert_eq!(world.get::<LifeLog>(p).unwrap().kills, 0);
}

fn respawn_app() -> App {
    respawn_app_in(Phase::Playing)
}

fn respawn_app_in(phase: Phase) -> App {
    let mut app = App::new();
    app.init_resource::<Arena>().init_resource::<SimTick>();
    app.world_mut().spawn(MatchState { phase, ..default() });
    app.add_systems(Update, respawn);
    app
}

fn spawn_dead(app: &mut App, evo: Evolution, wallet: Wallet, life: LifeLog) -> Entity {
    app.world_mut()
        .spawn((
            Pos(Vec2::ZERO),
            Vitals {
                alive: false,
                respawn_in: RESPAWN_MAX_TICKS,
                ..default()
            },
            Dash::default(),
            PosHistory::default(),
            Loadout::default(),
            evo,
            SkillState::default(),
            wallet,
            life,
        ))
        .id()
}

fn command(app: &mut App, e: Entity, cmd: PlayerCommand) {
    let world = app.world_mut();
    let mut q = world.query::<(&mut Evolution, &mut Wallet, &mut Vitals, &mut LifeLog)>();
    let (mut evo, mut wallet, mut vitals, mut life) = q.get_mut(world, e).unwrap();
    handle_command(&cmd, &mut evo, &mut wallet, &mut vitals, &mut life, true);
}

fn alive(app: &App, e: Entity) -> bool {
    app.world().get::<Vitals>(e).unwrap().alive
}

#[test]
fn deploy_is_allowed_after_4s_and_idle_humans_stay_dead() {
    let mut app = respawn_app();
    let mut evo = Evolution::default();
    evo.add_xp(xp_to_reach(2));
    assert!(evo.points > 0, "unspent points must not block deploy");
    let early = spawn_dead(&mut app, evo, default(), default());
    let idle = spawn_dead(&mut app, default(), default(), default());
    for _ in 0..RESPAWN_MIN_TICKS {
        command(&mut app, early, PlayerCommand::Deploy);
        assert!(!alive(&app, early), "deployed too early");
        app.update();
    }
    assert!(!alive(&app, early));
    assert!(app.world().get::<Evolution>(early).unwrap().points > 0);
    command(&mut app, early, PlayerCommand::Deploy);
    app.update();
    assert!(alive(&app, early));
    assert!(app.world().get::<Evolution>(early).unwrap().points > 0);
    assert!(!alive(&app, idle));
    for _ in 0..RESPAWN_MAX_TICKS - RESPAWN_MIN_TICKS {
        app.update();
    }
    let idle_vitals = app.world().get::<Vitals>(idle).unwrap();
    assert!(!idle_vitals.alive);
    assert_eq!(idle_vitals.respawn_in, 1, "stuck until Deploy");
}

#[test]
fn deploy_last_waits_for_4s_and_brings_the_last_cart() {
    let mut app = respawn_app();
    let mut evo = Evolution::default();
    evo.add_xp(xp_to_reach(2));
    let mut last_ranks = [0; 12];
    last_ranks[Perk::Claws.index()] = 1;
    let wallet = Wallet {
        scrap: 50,
        last_cart: vec![ShopItem::Bow],
        ..default()
    };
    let life = LifeLog {
        last_ranks,
        last_first_pick: [1, 0, 0],
        ..default()
    };
    let p = spawn_dead(&mut app, evo, wallet, life);
    command(&mut app, p, PlayerCommand::DeployLast);
    assert_eq!(
        app.world().get::<Evolution>(p).unwrap().rank(Perk::Claws),
        1
    );
    assert!(app.world().get::<LifeLog>(p).unwrap().deploy_asap);
    for _ in 0..RESPAWN_MIN_TICKS {
        app.update();
    }
    assert!(!alive(&app, p));
    app.update();
    assert!(alive(&app, p));
    let world = app.world();
    assert_eq!(world.get::<Loadout>(p).unwrap().weapon(), WeaponKind::Bow);
    let wallet = world.get::<Wallet>(p).unwrap();
    assert_eq!((wallet.scrap, wallet.cart.len()), (15, 0));
    assert!(!world.get::<LifeLog>(p).unwrap().deploy_asap);
}

#[test]
fn lobby_stands_up_when_the_timer_ends() {
    let mut app = respawn_app_in(Phase::Lobby);
    let p = spawn_dead(
        &mut app,
        default(),
        default(),
        LifeLog {
            deploy_asap: true,
            ..default()
        },
    );
    app.world_mut().get_mut::<Vitals>(p).unwrap().respawn_in = LOBBY_RESPAWN_TICKS;
    for _ in 0..LOBBY_RESPAWN_TICKS - 1 {
        app.update();
        assert!(!alive(&app, p), "lobby ignores the playing confirm hold");
    }
    app.update();
    assert!(alive(&app, p));
    assert!(!app.world().get::<LifeLog>(p).unwrap().deploy_asap);
}

#[test]
fn scrap_and_perks_survive_death_and_the_cart_is_equipped() {
    let mut app = App::new();
    app.init_resource::<DamageQueue>()
        .init_resource::<Outbox>()
        .init_resource::<XpQueue>()
        .init_resource::<Dealt>()
        .init_resource::<Arena>()
        .init_resource::<SimTick>();
    app.world_mut().spawn(MatchState {
        phase: Phase::Playing,
        ..default()
    });
    app.add_systems(Update, (resolve_damage, respawn).chain());
    let mut evo = Evolution::default();
    evo.add_xp(xp_to_reach(2));
    evo.spend(Perk::Overclock);
    let p = app
        .world_mut()
        .spawn((
            Player {
                client_id: 1,
                name: "p".into(),
                color: [1.0; 3],
                bot: false,
            },
            Pos(Vec2::ZERO),
            Vitals::default(),
            Loadout::default(),
            Stats::default(),
            evo,
            SkillState::default(),
            Recovery::default(),
            Wallet {
                scrap: 100,
                ..default()
            },
            LifeLog {
                scrap: 100,
                kills: 2,
                ..default()
            },
            (Dash::default(), PosHistory::default()),
        ))
        .id();
    app.world_mut()
        .resource_mut::<DamageQueue>()
        .0
        .push(Damage {
            target: p,
            attacker: None,
            amount: 500.0,
            armor_mult: 1.0,
            weapon: None,
            emp: false,
            cause: Some("辐射"),
        });
    app.update();
    let world = app.world();
    assert!(!alive(&app, p));
    let recap = world.get::<Wallet>(p).unwrap().recap.clone().unwrap();
    assert_eq!(
        (recap.scrap, recap.kills, recap.how.as_str()),
        (100, 2, "辐射")
    );
    let life = world.get::<LifeLog>(p).unwrap();
    assert_eq!((life.scrap, life.kills), (0, 0));
    assert_eq!(life.last_ranks[Perk::Overclock.index()], 1);

    command(&mut app, p, PlayerCommand::Buy(ShopItem::Rifle));
    command(&mut app, p, PlayerCommand::Buy(ShopItem::ScrapArmor));
    for _ in 0..RESPAWN_MIN_TICKS {
        app.update();
    }
    assert!(!alive(&app, p), "still dead until deploy");
    assert!(can_deploy(app.world().get::<Vitals>(p).unwrap().respawn_in));
    command(&mut app, p, PlayerCommand::Deploy);
    app.update();
    let world = app.world();
    assert!(alive(&app, p));
    let wallet = world.get::<Wallet>(p).unwrap();
    assert_eq!((wallet.scrap, wallet.cart.len()), (0, 0));
    assert_eq!(
        wallet.last_cart,
        vec![ShopItem::Rifle, ShopItem::ScrapArmor]
    );
    let loadout = world.get::<Loadout>(p).unwrap();
    assert_eq!(
        loadout.weapon(),
        WeaponKind::Rifle,
        "the bought gun comes with its ammo"
    );
    assert_eq!(
        loadout.rounds(AmmoKind::Bullet),
        AmmoKind::Bullet.box_rounds()
    );
    let vitals = world.get::<Vitals>(p).unwrap();
    assert_eq!(vitals.armor, ArmorKind::Scrap);
    assert_eq!(world.get::<Evolution>(p).unwrap().rank(Perk::Overclock), 1);
}

fn combatant(app: &mut App, id: u64, name: &str) -> Entity {
    app.world_mut()
        .spawn((
            Player {
                client_id: id,
                name: name.into(),
                color: [1.0; 3],
                bot: false,
            },
            Pos(Vec2::ZERO),
            Vitals::default(),
            Loadout::default(),
            Stats {
                score: 4,
                streak: 2,
                ..default()
            },
            Evolution::default(),
            SkillState::default(),
            Recovery::default(),
            Wallet {
                scrap: 10,
                ..default()
            },
            LifeLog::default(),
            (Dash::default(), PosHistory::default()),
        ))
        .id()
}

#[test]
fn public_join_stays_open_twelve_seconds_in_and_closes_at_seven() {
    // 20s clock = 17s Lobby + 3s Starting. Twelve seconds in, 8s remain: still offered.
    let mut state = MatchState {
        phase: Phase::Lobby,
        countdown_active: true,
        ticks_left: LOBBY_COUNTDOWN_TICKS - 12 * TICKS_PER_SEC,
        ..default()
    };
    assert!(lobby_open(&state), "8s until Playing still accepts");
    state.ticks_left = LOBBY_COUNTDOWN_TICKS - 13 * TICKS_PER_SEC;
    assert!(!lobby_open(&state), "7s until Playing is join_mode none");
    state.countdown_active = false;
    state.ticks_left = 0;
    assert!(lobby_open(&state));
    state.phase = Phase::Starting;
    state.ticks_left = STARTING_TICKS;
    assert!(!lobby_open(&state));
}

#[test]
fn lobby_kill_does_not_score_or_drop_a_bag() {
    let mut app = App::new();
    app.init_resource::<DamageQueue>()
        .init_resource::<Outbox>()
        .init_resource::<XpQueue>()
        .init_resource::<Dealt>()
        .init_resource::<SimTick>();
    app.world_mut().spawn(MatchState {
        phase: Phase::Lobby,
        ..default()
    });
    app.add_systems(Update, resolve_damage);
    let killer = combatant(&mut app, 1, "a");
    let victim = combatant(&mut app, 2, "b");
    app.world_mut()
        .resource_mut::<DamageQueue>()
        .0
        .push(Damage {
            target: victim,
            attacker: Some(killer),
            amount: 500.0,
            armor_mult: 1.0,
            weapon: Some(WeaponKind::Stick),
            emp: false,
            cause: None,
        });
    app.update();
    let world = app.world();
    assert_eq!(world.get::<Stats>(killer).unwrap().score, 4);
    assert_eq!(world.get::<Stats>(killer).unwrap().last_score_tick, 0);
    assert_eq!(world.get::<Stats>(killer).unwrap().streak, 2);
    assert_eq!(world.get::<Wallet>(killer).unwrap().scrap, 10);
    assert_eq!(world.get::<Evolution>(killer).unwrap().level, 1);
    assert_eq!(world.get::<Loadout>(victim).unwrap(), &Loadout::default());
    assert_eq!(
        world.get::<Vitals>(victim).unwrap().respawn_in,
        LOBBY_RESPAWN_TICKS
    );
    assert!(world.get::<Wallet>(victim).unwrap().recap.is_none());
    let bags = world
        .iter_entities()
        .filter(|e| e.contains::<Pickup>())
        .count();
    assert_eq!(bags, 0, "lobby deaths leave no bag");
}

fn private_settings(host: u64) -> RoomSettings {
    RoomSettings {
        bots: Some(ROOM_COMBATANTS),
        monsters: true,
        host,
        private_room: true,
        code: "ABCD".into(),
        auto_paused: false,
    }
}

fn server_config() -> ServerConfig {
    ServerConfig {
        port: 0,
        public_addr: "127.0.0.1:0".parse().unwrap(),
        match_id: "test".into(),
        max_players: ROOM_COMBATANTS as usize,
        min_combatants: ROOM_COMBATANTS as usize,
        matchmakers: Vec::new(),
        single_match: false,
        monsters: false,
        threads: 1,
        agones: false,
        room_seed: 1,
        private: false,
        room_code: String::new(),
        log_dir: std::env::temp_dir(),
        replay: None,
        logging: false,
    }
}

/// A `Playing` app at `ticks_left`, with
/// `(client_id, name, score, kills, deaths, last_score_tick)`.
fn flow_app(players: &[(u64, &str, u32, u32, u32, u32)], ticks_left: u32) -> App {
    let mut app = App::new();
    app.init_resource::<Arena>()
        .init_resource::<Outbox>()
        .init_resource::<ResetRequested>()
        .init_resource::<PendingStart>()
        .init_resource::<SimTick>()
        .insert_resource(server_config())
        .insert_resource(SimRng::from_match_seed(1));
    app.world_mut().spawn((
        MatchState {
            phase: Phase::Playing,
            ticks_left,
            ..default()
        },
        RoomSettings {
            bots: Some(ROOM_COMBATANTS),
            ..default()
        },
    ));
    for (client_id, name, score, kills, deaths, last_score_tick) in players {
        app.world_mut().spawn((
            Player {
                client_id: *client_id,
                name: (*name).into(),
                color: [1.0; 3],
                bot: false,
            },
            Stats {
                score: *score,
                kills: *kills,
                deaths: *deaths,
                last_score_tick: *last_score_tick,
                ..default()
            },
        ));
    }
    app.add_systems(Update, match_flow);
    app
}

/// Run one whistle tick and return the resulting phase, winner and ticks left.
fn whistle(players: &[(u64, &str, u32, u32, u32, u32)]) -> (Phase, Option<String>, u32) {
    let mut app = flow_app(players, 1);
    app.update();
    let world = app.world_mut();
    let mut query = world.query::<&MatchState>();
    let state = query.single(world).expect("match state");
    (state.phase, state.winner.clone(), state.ticks_left)
}

#[test]
fn non_host_room_command_does_not_change_settings() {
    let mut state = MatchState {
        phase: Phase::Lobby,
        ..default()
    };
    let mut settings = private_settings(1);
    let effect = room::apply_room_command(
        &RoomCommand::SetSettings {
            bots: None,
            monsters: false,
        },
        2,
        &mut state,
        &mut settings,
    );
    assert_eq!(effect, room::RoomEffect::Ignore);
    assert_eq!(settings.bots, Some(ROOM_COMBATANTS));
    assert!(settings.monsters);
}

#[test]
fn host_cannot_disable_bots_and_only_changes_monsters() {
    let mut state = MatchState {
        phase: Phase::Lobby,
        ..default()
    };
    let mut settings = private_settings(1);
    for bots in [None, Some(2), Some(4), Some(8)] {
        let effect = room::apply_room_command(
            &RoomCommand::SetSettings {
                bots,
                monsters: false,
            },
            1,
            &mut state,
            &mut settings,
        );
        assert_eq!(effect, room::RoomEffect::Updated);
        assert_eq!(settings.bots, Some(ROOM_COMBATANTS), "bots={bots:?}");
        assert!(!settings.monsters);
    }
}

#[test]
fn production_settings_always_field_eight_combatants() {
    assert_eq!(ROOM_COMBATANTS, 8);
    for bots in [0, 1, 2, 6, 8] {
        let mut config = server_config();
        config.min_combatants = bots;
        let settings = room::settings_from_config(&config);
        assert_eq!(
            settings.bots,
            Some(ROOM_COMBATANTS),
            "min_combatants={bots}"
        );
    }
}

#[test]
fn match_does_not_end_at_twenty_five_before_the_whistle() {
    let mut app = flow_app(&[(1, "a", 25, 5, 0, 0), (2, "b", 3, 0, 1, 0)], MATCH_TICKS);
    app.update();
    let world = app.world_mut();
    let mut query = world.query::<&MatchState>();
    let state = query.single(world).expect("match state");
    assert_eq!(
        state.phase,
        Phase::Playing,
        "no point target ends the match"
    );
    assert!(state.winner.is_none());
    assert_eq!(state.ticks_left, MATCH_TICKS - 1);
}

#[test]
fn whistle_ranks_by_score_kills_deaths_earlier_score_then_client_id() {
    // Highest score wins.
    assert_eq!(
        whistle(&[(1, "a", 10, 9, 0, 1), (2, "b", 11, 0, 5, 9)])
            .1
            .as_deref(),
        Some("b")
    );
    // Equal score: more kills wins.
    assert_eq!(
        whistle(&[(1, "a", 20, 3, 0, 1), (2, "b", 20, 5, 4, 9)])
            .1
            .as_deref(),
        Some("b")
    );
    // Equal score and kills: fewer deaths wins.
    assert_eq!(
        whistle(&[(1, "a", 20, 5, 3, 1), (2, "b", 20, 5, 1, 9)])
            .1
            .as_deref(),
        Some("b")
    );
    // Equal score, kills and deaths: whoever reached that score earlier wins.
    assert_eq!(
        whistle(&[(1, "a", 20, 5, 1, 400), (2, "b", 20, 5, 1, 100)])
            .1
            .as_deref(),
        Some("b")
    );
    // Still tied on the tick: lower client_id wins, whatever the spawn order.
    assert_eq!(
        whistle(&[(2, "b", 20, 5, 1, 100), (1, "a", 20, 5, 1, 100)])
            .1
            .as_deref(),
        Some("a")
    );
    assert_eq!(
        whistle(&[]),
        (Phase::Results, None, RESULTS_TICKS),
        "no players still ends the match without a winner"
    );
}

#[test]
fn set_settings_during_countdown_is_ignored() {
    let mut state = MatchState {
        phase: Phase::Lobby,
        countdown_active: true,
        ticks_left: PRIVATE_LOBBY_COUNTDOWN_TICKS,
        ..default()
    };
    let mut settings = private_settings(1);
    let effect = room::apply_room_command(
        &RoomCommand::SetSettings {
            bots: Some(2),
            monsters: false,
        },
        1,
        &mut state,
        &mut settings,
    );
    assert_eq!(effect, room::RoomEffect::Ignore);
    assert_eq!(settings.bots, Some(ROOM_COMBATANTS));
    assert!(settings.monsters);
    assert!(state.countdown_active);
}

#[test]
fn cancel_during_starting_is_ignored() {
    let mut state = MatchState {
        phase: Phase::Starting,
        countdown_active: false,
        ticks_left: STARTING_TICKS,
        ..default()
    };
    let mut settings = private_settings(1);
    let effect = room::apply_room_command(&RoomCommand::Cancel, 1, &mut state, &mut settings);
    assert_eq!(effect, room::RoomEffect::Ignore);
    assert_eq!(state.phase, Phase::Starting);
    assert_eq!(state.ticks_left, STARTING_TICKS);
}

#[test]
fn host_leave_transfers_to_the_longest_present_human() {
    let humans = [
        room::PresentHuman {
            client_id: 1,
            name: "host".into(),
            joined_at: 10,
            connected: true,
        },
        room::PresentHuman {
            client_id: 2,
            name: "early".into(),
            joined_at: 20,
            connected: true,
        },
        room::PresentHuman {
            client_id: 3,
            name: "late".into(),
            joined_at: 50,
            connected: true,
        },
        room::PresentHuman {
            client_id: 4,
            name: "ghost".into(),
            joined_at: 1,
            connected: false,
        },
    ];
    let (id, name) = room::next_host(&humans, Some(1)).expect("successor");
    assert_eq!(id, 2);
    assert_eq!(name, "early");
}

#[test]
fn add_score_stamps_the_latest_gain_and_ignores_zero() {
    let mut stats = Stats::default();
    stats.add_score(0, 10);
    assert_eq!((stats.score, stats.last_score_tick), (0, 0));
    stats.add_score(2, 40);
    stats.add_score(1, 90);
    assert_eq!((stats.score, stats.last_score_tick), (3, 90));
}

#[test]
fn playing_kill_stamps_the_score_tick() {
    let mut app = App::new();
    app.init_resource::<DamageQueue>()
        .init_resource::<Outbox>()
        .init_resource::<XpQueue>()
        .init_resource::<Dealt>()
        .insert_resource(SimTick(42));
    app.world_mut().spawn(MatchState {
        phase: Phase::Playing,
        ..default()
    });
    app.add_systems(Update, resolve_damage);
    let killer = combatant(&mut app, 1, "a");
    let victim = combatant(&mut app, 2, "b");
    app.world_mut()
        .resource_mut::<DamageQueue>()
        .0
        .push(Damage {
            target: victim,
            attacker: Some(killer),
            amount: 500.0,
            armor_mult: 1.0,
            weapon: Some(WeaponKind::Stick),
            emp: false,
            cause: None,
        });
    app.update();
    let world = app.world();
    let stats = world.get::<Stats>(killer).unwrap();
    assert_eq!(stats.score, 5);
    assert_eq!(stats.kills, 1);
    assert_eq!(stats.last_score_tick, 42);
    assert_eq!(world.get::<Stats>(victim).unwrap().last_score_tick, 0);
}

fn run_public_tick(phase: Phase, ticks_left: u32, match_index: u32) -> MatchState {
    let mut app = App::new();
    app.init_resource::<Arena>()
        .init_resource::<Outbox>()
        .init_resource::<ResetRequested>()
        .init_resource::<PendingStart>()
        .init_resource::<SimTick>()
        .insert_resource(server_config())
        .insert_resource(SimRng::from_match_seed(1));
    app.world_mut().spawn((
        MatchState {
            phase,
            ticks_left,
            countdown_active: false,
            match_index,
            ..default()
        },
        RoomSettings {
            bots: Some(ROOM_COMBATANTS),
            ..default()
        },
    ));
    for id in 1..=2 {
        app.world_mut().spawn((
            Player {
                client_id: id,
                name: format!("p{id}"),
                color: [1.0; 3],
                bot: false,
            },
            Stats::default(),
        ));
        app.world_mut().spawn(Human {
            peer: PeerId::Netcode(id),
            disconnected_ticks: 0,
            joined_at: 0,
            left: false,
        });
    }
    app.add_systems(Update, match_flow);
    app.update();
    let world = app.world_mut();
    let mut query = world.query::<&MatchState>();
    query.single(world).expect("match state").clone()
}

#[test]
fn results_last_fifteen_seconds_and_later_lobbies_are_fifteen() {
    assert_eq!(RESULTS_TICKS, 15 * TICKS_PER_SEC);

    let first = run_public_tick(Phase::Lobby, 0, 0);
    assert!(first.countdown_active);
    assert_eq!(first.match_index, 0);
    assert_eq!(first.ticks_left, LOBBY_COUNTDOWN_TICKS);
    assert_eq!(
        first.ticks_left + STARTING_TICKS,
        20 * TICKS_PER_SEC,
        "the first public countdown is still 20s"
    );

    let next = run_public_tick(Phase::Results, 1, 0);
    assert_eq!(next.phase, Phase::Lobby);
    assert_eq!(next.match_index, 1);
    assert!(next.countdown_active);
    assert_eq!(next.ticks_left, NEXT_LOBBY_COUNTDOWN_TICKS);
    assert_eq!(
        next.ticks_left + STARTING_TICKS,
        15 * TICKS_PER_SEC,
        "a round after results waits 15s"
    );

    let resumed = run_public_tick(Phase::Lobby, 0, 3);
    assert_eq!(resumed.ticks_left, NEXT_LOBBY_COUNTDOWN_TICKS);
}

#[test]
fn later_lobby_still_closes_inside_eight_seconds() {
    let mut state = MatchState {
        phase: Phase::Lobby,
        countdown_active: true,
        ticks_left: NEXT_LOBBY_COUNTDOWN_TICKS - 7 * TICKS_PER_SEC,
        match_index: 1,
        ..default()
    };
    assert!(lobby_open(&state), "8s until Playing still accepts");
    state.ticks_left = NEXT_LOBBY_COUNTDOWN_TICKS - 8 * TICKS_PER_SEC;
    assert!(!lobby_open(&state), "7s until Playing is closed");
}

fn departed_rows(app: &mut App) -> Vec<Departure> {
    let world = app.world_mut();
    let mut query = world.query::<&Departed>();
    query
        .single(world)
        .map(|departed| departed.0.clone())
        .unwrap_or_default()
}

#[test]
fn playing_leave_is_recorded_and_lobby_leave_is_not() {
    use crate::journal::Injected;

    let mut ranks = [0; 12];
    ranks[Perk::Claws.index()] = 2;
    let mut first_pick = [0; 3];
    first_pick[EvoPath::Mutant.index()] = 1;
    let evo = Evolution {
        level: 4,
        ranks,
        first_pick,
        ..Evolution::default()
    };
    let stats = Stats {
        score: 9,
        kills: 4,
        deaths: 2,
        level: 4,
        ..default()
    };

    let mut playing = App::new();
    playing.insert_resource(Injected {
        rooms: vec![(1, RoomCommand::Leave)],
        ..default()
    });
    playing.init_resource::<Outbox>();
    playing.world_mut().spawn((
        MatchState {
            phase: Phase::Playing,
            ..default()
        },
        private_settings(1),
        Departed::default(),
    ));
    playing.world_mut().spawn((
        Player {
            client_id: 1,
            name: "lea".into(),
            color: [1.0; 3],
            bot: false,
        },
        Human {
            peer: PeerId::Netcode(1),
            disconnected_ticks: 0,
            joined_at: 0,
            left: false,
        },
        stats,
        evo.clone(),
    ));
    playing.add_systems(Update, room::read_room_commands);
    playing.update();
    let rows = departed_rows(&mut playing);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "lea");
    assert_eq!((rows[0].score, rows[0].kills, rows[0].deaths), (9, 4, 2));
    assert_eq!(rows[0].level, 4);
    assert_eq!(rows[0].path, Some(EvoPath::Mutant));
    {
        let world = playing.world_mut();
        let mut players = world.query::<&Player>();
        assert_eq!(players.iter(world).count(), 0);
    }

    let mut lobby = App::new();
    lobby.insert_resource(Injected {
        rooms: vec![(1, RoomCommand::Leave)],
        ..default()
    });
    lobby.init_resource::<Outbox>();
    lobby.world_mut().spawn((
        MatchState {
            phase: Phase::Lobby,
            ..default()
        },
        private_settings(1),
        Departed::default(),
    ));
    lobby.world_mut().spawn((
        Player {
            client_id: 1,
            name: "lea".into(),
            color: [1.0; 3],
            bot: false,
        },
        Human {
            peer: PeerId::Netcode(1),
            disconnected_ticks: 0,
            joined_at: 0,
            left: false,
        },
        stats,
        evo,
    ));
    lobby.add_systems(Update, room::read_room_commands);
    lobby.update();
    assert!(departed_rows(&mut lobby).is_empty());
}

#[test]
fn a_departed_high_score_does_not_take_the_win() {
    let mut app = flow_app(&[(2, "stay", 3, 1, 0, 10)], 1);
    app.world_mut().spawn(Departed(vec![Departure {
        name: "gone".into(),
        score: 100,
        kills: 20,
        deaths: 0,
        level: 8,
        path: Some(EvoPath::Cyborg),
    }]));
    app.update();
    let winner = {
        let world = app.world_mut();
        let mut states = world.query::<&MatchState>();
        let state = states.single(world).unwrap();
        assert_eq!(state.phase, Phase::Results);
        state.winner.clone()
    };
    assert_eq!(winner.as_deref(), Some("stay"));
    let world = app.world_mut();
    let mut departed = world.query::<&Departed>();
    let row = &departed.single(world).unwrap().0[0];
    assert_eq!(row.score, 100, "frozen");
    assert_ne!(winner.as_deref(), Some(row.name.as_str()));
}

fn idle_app(alive: bool) -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<Outbox>();
    app.world_mut().spawn((
        MatchState {
            phase: Phase::Playing,
            ..default()
        },
        Departed::default(),
    ));
    let body = app
        .world_mut()
        .spawn((
            Player {
                client_id: 3,
                name: "idle".into(),
                color: [1.0; 3],
                bot: false,
            },
            Human {
                peer: PeerId::Netcode(3),
                disconnected_ticks: 0,
                joined_at: 0,
                left: false,
            },
            Idle::default(),
            CurrentInput::default(),
            Vitals { alive, ..default() },
            Stats {
                score: 2,
                kills: 1,
                ..default()
            },
            Evolution::default(),
        ))
        .id();
    app.add_systems(Update, idle::watch_idle);
    (app, body)
}

#[test]
fn idle_human_is_warned_at_60s_and_removed_at_90s() {
    let (mut app, body) = idle_app(true);
    for _ in 0..idle::IDLE_WARN_TICKS - 1 {
        app.update();
    }
    assert!(app.world().get::<Player>(body).is_some());
    assert!(app.world().resource::<Outbox>().direct.is_empty());
    app.update();
    let note = app.world().resource::<Outbox>().direct.clone();
    assert_eq!(note.len(), 1);
    assert!(
        matches!(&note[0].1, GameEvent::Announce(text) if text == "60 秒内无操作，30 秒后将被移出房间")
    );
    app.world_mut().resource_mut::<Outbox>().direct.clear();
    for _ in 0..(idle::IDLE_KICK_TICKS - idle::IDLE_WARN_TICKS - 1) {
        app.update();
    }
    assert!(
        app.world().get::<Player>(body).is_some(),
        "still here at 89s"
    );
    app.update();
    assert!(app.world().get::<Player>(body).is_none());
    let rows = departed_rows(&mut app);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].name.as_str(), rows[0].score, rows[0].kills),
        ("idle", 2, 1)
    );
    let announced = app
        .world()
        .resource::<Outbox>()
        .reliable
        .iter()
        .any(|event| matches!(event, GameEvent::Announce(text) if text.contains("挂机")));
    assert!(announced);
}

#[test]
fn dead_time_and_a_changed_input_do_not_count_as_idle() {
    let (mut app, body) = idle_app(false);
    for _ in 0..idle::IDLE_KICK_TICKS {
        app.update();
    }
    assert!(
        app.world().get::<Player>(body).is_some(),
        "a corpse is not idle"
    );
    assert!(departed_rows(&mut app).is_empty());
    app.world_mut().get_mut::<Vitals>(body).unwrap().alive = true;
    app.world_mut()
        .get_mut::<CurrentInput>(body)
        .unwrap()
        .0
        .move_x = 1;
    for _ in 0..idle::IDLE_WARN_TICKS {
        app.update();
    }
    assert!(
        app.world().resource::<Outbox>().direct.is_empty(),
        "the move resets the clock"
    );
    assert!(app.world().get::<Player>(body).is_some());
}

#[test]
fn a_bot_is_not_idle_kicked() {
    let (mut app, _) = idle_app(true);
    let bot = app
        .world_mut()
        .spawn((
            Player {
                client_id: 9,
                name: "bot".into(),
                color: [0.5; 3],
                bot: true,
            },
            CurrentInput::default(),
            Vitals::default(),
            Stats::default(),
            Evolution::default(),
        ))
        .id();
    app.update();
    assert!(app.world().get::<Player>(bot).is_some());
}

#[test]
fn a_human_auto_deploys_last_loadout_after_45s_and_a_bot_does_not() {
    let mut app = respawn_app();
    let mut evo = Evolution::default();
    evo.add_xp(xp_to_reach(2));
    let mut last_ranks = [0; 12];
    last_ranks[Perk::Claws.index()] = 1;
    let life = LifeLog {
        last_ranks,
        last_first_pick: [1, 0, 0],
        ..default()
    };
    let human = spawn_dead(&mut app, evo, Wallet::default(), life);
    app.world_mut().entity_mut(human).insert((Player {
        client_id: 1,
        name: "h".into(),
        color: [1.0; 3],
        bot: false,
    },));
    app.world_mut().get_mut::<Vitals>(human).unwrap().deploy_in = AUTO_DEPLOY_TICKS;

    let bot = spawn_dead(
        &mut app,
        Evolution::default(),
        Wallet::default(),
        LifeLog::default(),
    );
    app.world_mut().entity_mut(bot).insert(Player {
        client_id: 2,
        name: "b".into(),
        color: [1.0; 3],
        bot: true,
    });

    for _ in 0..AUTO_DEPLOY_TICKS - 1 {
        app.update();
    }
    assert!(!alive(&app, human), "still on the death screen");
    assert_eq!(app.world().get::<Vitals>(human).unwrap().deploy_in, 1);
    app.update();
    assert!(alive(&app, human));
    assert_eq!(
        app.world()
            .get::<Evolution>(human)
            .unwrap()
            .rank(Perk::Claws),
        1
    );
    assert_eq!(app.world().get::<Vitals>(human).unwrap().deploy_in, 0);
    assert!(!alive(&app, bot), "bots are not on the 45s clock");
    assert_eq!(app.world().get::<Vitals>(bot).unwrap().respawn_in, 1);
}

fn run_private_tick(
    phase: Phase,
    ticks_left: u32,
    match_index: u32,
    auto_paused: bool,
) -> (MatchState, bool) {
    let mut app = App::new();
    app.init_resource::<Arena>()
        .init_resource::<Outbox>()
        .init_resource::<ResetRequested>()
        .init_resource::<PendingStart>()
        .init_resource::<SimTick>()
        .insert_resource(server_config())
        .insert_resource(SimRng::from_match_seed(1));
    let mut settings = private_settings(1);
    settings.auto_paused = auto_paused;
    app.world_mut().spawn((
        MatchState {
            phase,
            ticks_left,
            countdown_active: false,
            match_index,
            ..default()
        },
        settings,
    ));
    for id in 1..=2 {
        app.world_mut().spawn((
            Player {
                client_id: id,
                name: format!("p{id}"),
                color: [1.0; 3],
                bot: false,
            },
            Stats::default(),
            Human {
                peer: PeerId::Netcode(id),
                disconnected_ticks: 0,
                joined_at: 0,
                left: false,
            },
        ));
    }
    app.add_systems(Update, match_flow);
    app.update();
    let world = app.world_mut();
    let mut states = world.query::<&MatchState>();
    let mut rooms = world.query::<&RoomSettings>();
    (
        states.single(world).unwrap().clone(),
        rooms.single(world).unwrap().auto_paused,
    )
}

#[test]
fn private_room_auto_continues_after_the_first_match_unless_paused() {
    let (first, _) = run_private_tick(Phase::Lobby, 0, 0, false);
    assert_eq!(first.phase, Phase::Lobby);
    assert!(
        !first.countdown_active,
        "the first round still waits for the host"
    );

    let (next, paused) = run_private_tick(Phase::Results, 1, 0, false);
    assert_eq!(next.phase, Phase::Lobby);
    assert_eq!(next.match_index, 1);
    assert!(next.countdown_active);
    assert_eq!(next.ticks_left, NEXT_LOBBY_COUNTDOWN_TICKS);
    assert_eq!(next.ticks_left + STARTING_TICKS, 15 * TICKS_PER_SEC);
    assert!(!paused);

    let (held, paused) = run_private_tick(Phase::Results, 1, 0, true);
    assert_eq!(held.phase, Phase::Lobby);
    assert_eq!(held.match_index, 1);
    assert!(!held.countdown_active);
    assert_eq!(held.ticks_left, 0);
    assert!(paused);
}

#[test]
fn pause_auto_start_is_host_only_and_ignored_once_the_countdown_runs() {
    let mut state = MatchState {
        phase: Phase::Results,
        match_index: 1,
        ..default()
    };
    let mut settings = private_settings(1);
    assert_eq!(
        room::apply_room_command(&RoomCommand::PauseAutoStart, 2, &mut state, &mut settings),
        room::RoomEffect::Ignore
    );
    assert!(!settings.auto_paused);
    assert_eq!(
        room::apply_room_command(&RoomCommand::PauseAutoStart, 1, &mut state, &mut settings),
        room::RoomEffect::Updated
    );
    assert!(settings.auto_paused);

    state.phase = Phase::Lobby;
    state.countdown_active = false;
    state.ticks_left = 0;
    state.match_index = 1;
    assert_eq!(
        room::apply_room_command(&RoomCommand::ResumeAutoStart, 1, &mut state, &mut settings),
        room::RoomEffect::Updated
    );
    assert!(!settings.auto_paused);
    assert!(!state.countdown_active, "resume does not start the clock");
    assert_eq!(
        room::apply_room_command(&RoomCommand::Start, 1, &mut state, &mut settings),
        room::RoomEffect::Started
    );
    assert_eq!(state.ticks_left, PRIVATE_LOBBY_COUNTDOWN_TICKS);
    // Put the 15s clock up, then show Pause no longer applies.
    state.countdown_active = true;
    state.ticks_left = NEXT_LOBBY_COUNTDOWN_TICKS;

    assert_eq!(
        room::apply_room_command(&RoomCommand::PauseAutoStart, 1, &mut state, &mut settings),
        room::RoomEffect::Ignore,
        "a running countdown is cancelled, not paused"
    );
    assert!(!settings.auto_paused);
    assert!(state.countdown_active);

    assert_eq!(
        room::apply_room_command(&RoomCommand::Cancel, 1, &mut state, &mut settings),
        room::RoomEffect::Cancelled
    );
    assert!(!state.countdown_active);
}

fn announces(app: &App) -> Vec<String> {
    app.world()
        .resource::<Outbox>()
        .reliable
        .iter()
        .filter_map(|event| match event {
            GameEvent::Announce(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn flash_of(app: &mut App) -> Option<LeaderFlash> {
    let world = app.world_mut();
    let mut query = world.query::<&LeaderPing>();
    query.single(world).expect("leader ping").latest
}

struct Spot {
    id: u64,
    name: &'static str,
    score: u32,
    kills: u32,
    deaths: u32,
    scored_at: u32,
    pos: Vec2,
    alive: bool,
    bot: bool,
}

fn spot(id: u64, name: &'static str, score: u32, pos: Vec2) -> Spot {
    Spot {
        id,
        name,
        score,
        kills: 0,
        deaths: 0,
        scored_at: 0,
        pos,
        alive: true,
        bot: false,
    }
}

/// Playing, one tick before `ticks_left` becomes the value passed in.
fn finale_app(spots: &[Spot], ticks_left: u32) -> (App, Vec<Entity>) {
    let mut app = App::new();
    app.init_resource::<Arena>()
        .init_resource::<Outbox>()
        .init_resource::<ResetRequested>()
        .init_resource::<PendingStart>()
        .insert_resource(SimTick(100))
        .insert_resource(server_config())
        .insert_resource(SimRng::from_match_seed(1));
    app.world_mut().spawn((
        MatchState {
            phase: Phase::Playing,
            ticks_left,
            vault_in: 1_000_000,
            ..default()
        },
        RoomSettings {
            bots: Some(ROOM_COMBATANTS),
            ..default()
        },
        LeaderPing::default(),
    ));
    let mut entities = Vec::new();
    for spot in spots {
        entities.push(
            app.world_mut()
                .spawn((
                    Player {
                        client_id: spot.id,
                        name: spot.name.into(),
                        color: [1.0; 3],
                        bot: spot.bot,
                    },
                    Stats {
                        score: spot.score,
                        kills: spot.kills,
                        deaths: spot.deaths,
                        last_score_tick: spot.scored_at,
                        ..default()
                    },
                    Pos(spot.pos),
                    Vitals {
                        alive: spot.alive,
                        ..default()
                    },
                ))
                .id(),
        );
    }
    app.add_systems(Update, (match_flow, super::finale::step_finale).chain());
    (app, entities)
}

fn run_flow(app: &mut App, n: u32) {
    for _ in 0..n {
        app.update();
    }
}

#[test]
fn finale_clock_is_the_last_two_minutes_of_playing() {
    assert_eq!(FINALE_TICKS, 2 * 60 * TICKS_PER_SEC);
    assert_eq!(super::finale::FINALE_PING_TICKS, 10 * TICKS_PER_SEC);
    let mut state = MatchState {
        phase: Phase::Playing,
        ticks_left: FINALE_TICKS + 1,
        ..default()
    };
    assert!(!state.finale());
    // Resolve still sees this pre-decrement value, so the entry tick already pays.
    assert!(super::finale::finale_for_kills(&state));
    state.ticks_left = FINALE_TICKS;
    assert!(state.finale());
    state.ticks_left = 1;
    assert!(state.finale());
    assert!(super::finale::finale_for_kills(&state));
    state.ticks_left = 0;
    assert!(!state.finale());
    assert!(!super::finale::finale_for_kills(&state));
    state.phase = Phase::Lobby;
    state.ticks_left = FINALE_TICKS;
    assert!(!state.finale());
    assert!(!super::finale::finale_for_kills(&state));
    // Resolve sees the clock before this tick's decrement, so the entry tick counts.
    state.phase = Phase::Playing;
    state.ticks_left = FINALE_TICKS + 1;
    assert!(super::finale::finale_for_kills(&state));
    state.ticks_left = FINALE_TICKS + 2;
    assert!(!super::finale::finale_for_kills(&state));
}

#[test]
fn finale_broadcast_fires_once_at_two_minutes_left() {
    let mut lead = spot(2, "bee", 8, Vec2::new(9.0, 3.0));
    lead.bot = true;
    lead.kills = 4;
    let mut other = spot(1, "ann", 8, Vec2::new(1.0, 0.0));
    other.kills = 1;
    let (mut app, _) = finale_app(&[other, lead], FINALE_TICKS + 2);
    app.update();
    assert!(
        !announces(&app).iter().any(|text| text.contains("终局")),
        "one second early"
    );
    assert!(flash_of(&mut app).is_none());
    app.update();
    let notes = announces(&app);
    assert_eq!(notes.iter().filter(|text| text.contains("终局")).count(), 1);
    assert_eq!(notes[0], super::finale::FINALE_ANNOUNCE);
    let flash = flash_of(&mut app).expect("flash at second 0");
    assert_eq!(flash.client_id, 2, "more kills wins the tie, bots included");
    assert_eq!(flash.pos, Vec2::new(9.0, 3.0));
    assert_eq!(flash.tick, 100);
    let world = app.world_mut();
    let mut query = world.query::<&MatchState>();
    assert!(query.single(world).unwrap().finale());
}

#[test]
fn leader_ping_repeats_every_ten_seconds() {
    let (mut app, ids) = finale_app(&[spot(1, "ann", 3, Vec2::new(1.0, 2.0))], FINALE_TICKS + 1);
    app.update();
    assert_eq!(flash_of(&mut app).unwrap().tick, 100);
    app.world_mut().resource_mut::<SimTick>().0 = 101;
    run_flow(&mut app, super::finale::FINALE_PING_TICKS - 1);
    assert_eq!(flash_of(&mut app).unwrap().tick, 100, "not between flashes");
    assert_eq!(
        announces(&app)
            .iter()
            .filter(|text| text.contains("终局"))
            .count(),
        1,
        "the banner is once"
    );
    app.world_mut().get_mut::<Pos>(ids[0]).unwrap().0 = Vec2::new(20.0, -5.0);
    app.world_mut().resource_mut::<SimTick>().0 = 250;
    app.update();
    let flash = flash_of(&mut app).unwrap();
    assert_eq!(flash.tick, 250);
    assert_eq!(flash.pos, Vec2::new(20.0, -5.0));
    assert_eq!(flash.client_id, 1);
}

#[test]
fn dead_leader_skips_the_flash_and_keeps_the_last_one() {
    let mut leader = spot(2, "bee", 8, Vec2::new(4.0, 4.0));
    leader.alive = false;
    leader.bot = true;
    let (mut app, ids) = finale_app(
        &[spot(1, "ann", 1, Vec2::new(1.0, 1.0)), leader],
        FINALE_TICKS + 1,
    );
    app.update();
    assert!(
        flash_of(&mut app).is_none(),
        "do not fall through to second"
    );
    assert!(announces(&app).iter().any(|text| text.contains("终局")));
    // A later boundary still skips, and does not wipe a flash from earlier.
    let world = app.world_mut();
    let mut query = world.query::<&mut LeaderPing>();
    query.single_mut(world).unwrap().latest = Some(LeaderFlash {
        client_id: 2,
        pos: Vec2::new(4.0, 4.0),
        tick: 3,
    });
    app.world_mut().resource_mut::<SimTick>().0 = 9;
    run_flow(&mut app, super::finale::FINALE_PING_TICKS);
    assert_eq!(
        flash_of(&mut app),
        Some(LeaderFlash {
            client_id: 2,
            pos: Vec2::new(4.0, 4.0),
            tick: 3,
        })
    );
    app.world_mut().get_mut::<Vitals>(ids[1]).unwrap().alive = true;
    app.world_mut().get_mut::<Pos>(ids[1]).unwrap().0 = Vec2::new(7.0, 8.0);
    app.world_mut().resource_mut::<SimTick>().0 = 40;
    run_flow(&mut app, super::finale::FINALE_PING_TICKS);
    let flash = flash_of(&mut app).unwrap();
    assert_eq!(flash.client_id, 2);
    assert_eq!(flash.pos, Vec2::new(7.0, 8.0));
    assert_eq!(flash.tick, 40);
}

#[test]
fn zero_scores_have_no_leader_to_flash() {
    let mut killer = spot(1, "ann", 0, Vec2::new(1.0, 0.0));
    killer.kills = 9;
    let (mut app, _) = finale_app(&[killer, spot(2, "bee", 0, Vec2::ZERO)], FINALE_TICKS + 1);
    app.update();
    assert!(flash_of(&mut app).is_none());
    assert!(announces(&app).iter().any(|text| text.contains("终局")));
}

#[test]
fn match_start_clears_the_leader_ping() {
    let mut app = App::new();
    app.init_resource::<Arena>()
        .init_resource::<Outbox>()
        .init_resource::<ResetRequested>()
        .init_resource::<PendingStart>()
        .init_resource::<SimTick>()
        .insert_resource(server_config())
        .insert_resource(SimRng::from_match_seed(1));
    app.world_mut().spawn((
        MatchState {
            phase: Phase::Starting,
            ticks_left: 1,
            ..default()
        },
        RoomSettings {
            bots: Some(ROOM_COMBATANTS),
            ..default()
        },
        LeaderPing {
            latest: Some(LeaderFlash {
                client_id: 9,
                pos: Vec2::new(3.0, 4.0),
                tick: 8,
            }),
        },
    ));
    for id in 1..=2 {
        app.world_mut().spawn((
            Player {
                client_id: id,
                name: format!("p{id}"),
                color: [1.0; 3],
                bot: false,
            },
            Stats::default(),
        ));
    }
    app.add_systems(Update, (match_flow, super::finale::step_finale).chain());
    app.update();
    assert!(flash_of(&mut app).is_none());
    let world = app.world_mut();
    let mut query = world.query::<&MatchState>();
    let state = query.single(world).unwrap();
    assert_eq!(state.phase, Phase::Playing);
    assert!(!state.finale());
}

fn duel(phase: Phase, ticks_left: u32) -> (App, Entity, Entity) {
    let mut app = App::new();
    app.init_resource::<DamageQueue>()
        .init_resource::<Outbox>()
        .init_resource::<XpQueue>()
        .init_resource::<Dealt>()
        .insert_resource(SimTick(42));
    app.world_mut().spawn(MatchState {
        phase,
        ticks_left,
        ..default()
    });
    app.add_systems(Update, resolve_damage);
    let killer = combatant(&mut app, 1, "ann");
    let victim = combatant(&mut app, 2, "bee");
    {
        let world = app.world_mut();
        let mut killer_stats = world.get_mut::<Stats>(killer).unwrap();
        killer_stats.score = 1;
        killer_stats.streak = 0;
        let mut victim_stats = world.get_mut::<Stats>(victim).unwrap();
        victim_stats.score = 9;
        victim_stats.streak = 0;
        world.get_mut::<Player>(victim).unwrap().bot = true;
    }
    (app, killer, victim)
}

fn strike(app: &mut App, attacker: Option<Entity>, victim: Entity) {
    app.world_mut()
        .resource_mut::<DamageQueue>()
        .0
        .push(Damage {
            target: victim,
            attacker,
            amount: 500.0,
            armor_mult: 1.0,
            weapon: Some(WeaponKind::Stick),
            emp: false,
            cause: None,
        });
    app.update();
}

#[test]
fn killing_the_leader_in_finale_stacks_two_on_the_bounty() {
    let (mut app, killer, victim) = duel(Phase::Playing, FINALE_TICKS + 1);
    app.world_mut().get_mut::<Stats>(victim).unwrap().streak = sim::BOUNTY_STREAK;
    strike(&mut app, Some(killer), victim);
    let stats = app.world().get::<Stats>(killer).unwrap();
    assert_eq!(
        stats.score, 6,
        "bounty 3 plus finale 2, on top of the 1 already held"
    );
    assert_eq!(stats.kills, 1);
    assert_eq!(stats.last_score_tick, 42);
    assert!(
        announces(&app)
            .iter()
            .any(|text| text == "ann 击杀了第一名 bee，+2")
    );
    // The last tick of the match is still finale for a kill.
    let (mut app, killer, victim) = duel(Phase::Playing, 1);
    strike(&mut app, Some(killer), victim);
    assert_eq!(app.world().get::<Stats>(killer).unwrap().score, 4);
}

#[test]
fn killing_the_leader_outside_finale_adds_no_bonus() {
    let (mut app, killer, victim) = duel(Phase::Playing, FINALE_TICKS + 2);
    strike(&mut app, Some(killer), victim);
    assert_eq!(app.world().get::<Stats>(killer).unwrap().score, 2);
    assert!(!announces(&app).iter().any(|text| text.contains("第一名")));

    let (mut app, killer, victim) = duel(Phase::Playing, MATCH_TICKS);
    app.world_mut().get_mut::<Stats>(victim).unwrap().streak = sim::BOUNTY_STREAK;
    strike(&mut app, Some(killer), victim);
    assert_eq!(
        app.world().get::<Stats>(killer).unwrap().score,
        4,
        "bounty stays 3, with no finale 2"
    );
    assert!(!announces(&app).iter().any(|text| text.contains("第一名")));
}

#[test]
fn lobby_and_suicide_do_not_pay_the_finale_bonus() {
    let (mut app, killer, victim) = duel(Phase::Lobby, FINALE_TICKS);
    strike(&mut app, Some(killer), victim);
    assert_eq!(app.world().get::<Stats>(killer).unwrap().score, 1);
    assert_eq!(app.world().get::<Stats>(victim).unwrap().score, 9);
    assert!(announces(&app).is_empty());

    let (mut app, killer, victim) = duel(Phase::Playing, FINALE_TICKS + 1);
    strike(&mut app, Some(victim), victim);
    assert_eq!(app.world().get::<Stats>(killer).unwrap().score, 1);
    assert_eq!(app.world().get::<Stats>(victim).unwrap().score, 9);
    assert!(!announces(&app).iter().any(|text| text.contains("第一名")));
}

#[test]
fn monster_and_wasteland_kills_do_not_pay_the_finale_bonus() {
    let (mut app, killer, victim) = duel(Phase::Playing, FINALE_TICKS + 1);
    let roach = app
        .world_mut()
        .spawn(Monster {
            kind: MonsterKind::Roach,
            elite: false,
        })
        .id();
    strike(&mut app, Some(roach), victim);
    assert_eq!(app.world().get::<Stats>(killer).unwrap().score, 1);
    assert!(!announces(&app).iter().any(|text| text.contains("第一名")));

    let (mut app, killer, victim) = duel(Phase::Playing, 1);
    app.world_mut()
        .resource_mut::<DamageQueue>()
        .0
        .push(Damage {
            target: victim,
            attacker: None,
            amount: 500.0,
            armor_mult: 1.0,
            weapon: None,
            emp: false,
            cause: Some("辐射"),
        });
    app.update();
    assert_eq!(app.world().get::<Stats>(killer).unwrap().score, 1);
    assert!(!announces(&app).iter().any(|text| text.contains("第一名")));
}

fn room_closes_after(phase: Phase, watcher: bool) -> bool {
    use crate::ops::OpsState;
    use crate::spectate::SpectatorSession;

    let mut app = App::new();
    app.insert_resource(server_config());
    app.insert_resource(OpsState::for_test());
    app.world_mut().spawn(MatchState { phase, ..default() });
    if watcher {
        app.world_mut().spawn((
            Spectator {
                following: 1,
                queue_pos: 1,
            },
            SpectatorSession {
                client_id: 9,
                name: "eye".into(),
                peer: PeerId::Netcode(9),
                link: Entity::PLACEHOLDER,
                order: 1,
                watched: false,
                disconnected_ticks: 0,
            },
        ));
    }
    app.add_systems(Update, empty_close);
    for _ in 0..EMPTY_MATCH_TICKS {
        app.update();
    }
    let mut messages = app.world_mut().resource_mut::<Messages<AppExit>>();
    messages.drain().next().is_some()
}

#[test]
fn only_watchers_keep_a_playing_room_open() {
    assert!(
        room_closes_after(Phase::Playing, false),
        "no humans and no watchers still closes"
    );
    assert!(
        !room_closes_after(Phase::Playing, true),
        "a watcher counts as someone in the room"
    );
    assert!(
        !room_closes_after(Phase::Results, true),
        "results with only a watcher does not close"
    );
}
