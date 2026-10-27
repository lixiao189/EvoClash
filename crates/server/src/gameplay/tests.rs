use evoclash_game::evolution::{Perk, xp_to_reach};
use evoclash_game::shop::{PlayerCommand, RESPAWN_MIN_TICKS, ShopItem, can_deploy};

use super::*;
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

/// A `Playing` app at `ticks_left`, with players `(client_id, name, score, kills, deaths)`.
fn flow_app(players: &[(u64, &str, u32, u32, u32)], ticks_left: u32) -> App {
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
    for (client_id, name, score, kills, deaths) in players {
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
                ..default()
            },
        ));
    }
    app.add_systems(Update, match_flow);
    app
}

/// Run one whistle tick and return the resulting phase, winner and ticks left.
fn whistle(players: &[(u64, &str, u32, u32, u32)]) -> (Phase, Option<String>, u32) {
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
fn production_settings_always_field_six_combatants() {
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
    let mut app = flow_app(&[(1, "a", 25, 5, 0), (2, "b", 3, 0, 1)], MATCH_TICKS);
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
fn whistle_ranks_by_score_then_kills_then_fewer_deaths_then_client_id() {
    // Highest score wins.
    assert_eq!(
        whistle(&[(1, "a", 10, 9, 0), (2, "b", 11, 0, 5)])
            .1
            .as_deref(),
        Some("b")
    );
    // Equal score: more kills wins.
    assert_eq!(
        whistle(&[(1, "a", 20, 3, 0), (2, "b", 20, 5, 4)])
            .1
            .as_deref(),
        Some("b")
    );
    // Equal score and kills: fewer deaths wins.
    assert_eq!(
        whistle(&[(1, "a", 20, 5, 3), (2, "b", 20, 5, 1)])
            .1
            .as_deref(),
        Some("b")
    );
    // Fully tied: lower client_id wins, whatever the spawn order.
    assert_eq!(
        whistle(&[(2, "b", 20, 5, 1), (1, "a", 20, 5, 1)])
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
