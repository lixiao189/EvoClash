use evoclash_game::evolution::{MAX_RANK, SKILL_A_RANK, SKILL_B_RANK, xp_to_reach};
use evoclash_game::items::{AmmoKind, MELEE_SLOT};
use evoclash_game::shop::RESPAWN_MAX_TICKS;
use lightyear::prelude::PeerId;

use super::build::{arrange, build_order, prep_bot};
use super::think::interest;
use super::*;

#[test]
fn bots_bring_their_best_gun_with_ammo_to_the_hotbar() {
    let evo = Evolution::default();
    let mut loadout = Loadout::default();
    for gun in [WeaponKind::Rifle, WeaponKind::Bow, WeaponKind::Laser] {
        loadout.add_weapon(gun);
    }
    loadout.add_ammo(AmmoKind::Cell, 10, &evo);
    loadout.add_ammo(AmmoKind::Arrow, 10, &evo);
    assert_eq!(loadout.bag[0], Some(WeaponKind::Laser));
    arrange(&mut loadout);
    assert!(loadout.guns.contains(&Some(WeaponKind::Laser)));
    assert!(loadout.guns.contains(&Some(WeaponKind::Bow)));
    assert_eq!(loadout.weapon(), WeaponKind::Laser);
    loadout.ammo = [0; 4];
    arrange(&mut loadout);
    assert_eq!(loadout.selected, MELEE_SLOT);
}

#[test]
fn bots_chase_ammo_for_guns_they_carry() {
    let evo = Evolution::default();
    let vitals = Vitals::default();
    let mut loadout = Loadout::default();
    loadout.add_weapon(WeaponKind::Rifle);
    let bullets = Item::ammo(AmmoKind::Bullet);
    let arrows = Item::ammo(AmmoKind::Arrow);
    let keen = interest(&bullets, None, &vitals, &loadout, &evo).unwrap();
    assert!(keen < 0.5, "dry rifle: bullets are urgent");
    assert_eq!(interest(&arrows, None, &vitals, &loadout, &evo), None);
    let bag = BagContents(vec![arrows, bullets]);
    assert!(interest(&Item::Bag, Some(&bag), &vitals, &loadout, &evo).unwrap() < keen);
}

#[test]
fn build_orders_reach_both_skills_on_time() {
    for path in EvoPath::ALL {
        assert!(build_order(path).iter().all(|p| p.evo_path() == path));
        let mut evo = Evolution::default();
        let (mut wallet, mut life) = (Wallet::default(), LifeLog::default());
        let mut vitals = Vitals::default();
        for level in 2..=10 {
            evo.add_xp(xp_to_reach(level) - evo.xp);
            assert!(prep_bot(
                path,
                &mut evo,
                &mut wallet,
                &mut vitals,
                &mut life
            ));
            let spent = level - 1;
            assert_eq!((evo.points, evo.path_rank(path)), (0, spent), "{path:?}");
            if spent == 2 {
                assert_eq!(evo.path_rank(path), SKILL_A_RANK);
            }
            if spent == 5 {
                assert_eq!(evo.path_rank(path), SKILL_B_RANK);
            }
        }
        assert!(Perk::ALL.iter().all(|p| evo.rank(*p) <= MAX_RANK));
        assert!(evo.skills().1.is_some());
    }
}

fn balance_app() -> App {
    let mut app = App::new();
    // Skip replication so a bot despawn does not need the netcode visibility registry.
    app.insert_resource(PlainBots);
    app.insert_resource(ServerConfig {
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
        recover: false,
        logging: false,
    });
    app.insert_resource(SimRng::from_match_seed(1));
    app.world_mut().spawn((
        MatchState {
            phase: Phase::Lobby,
            ..default()
        },
        RoomSettings {
            bots: Some(ROOM_COMBATANTS),
            ..default()
        },
    ));
    app.add_systems(Update, balance_bots);
    app
}

fn spawn_humans(app: &mut App, n: usize) -> Vec<Entity> {
    (0..n)
        .map(|i| {
            let client_id = i as u64 + 1;
            app.world_mut()
                .spawn((
                    Player {
                        client_id,
                        name: format!("h{client_id}"),
                        color: [1.0; 3],
                        bot: false,
                    },
                    Human {
                        peer: PeerId::Netcode(client_id),
                        disconnected_ticks: 0,
                        joined_at: 0,
                        left: false,
                    },
                ))
                .id()
        })
        .collect()
}

/// `(players, bots)` right now.
fn roster(app: &mut App) -> (usize, usize) {
    let world = app.world_mut();
    let mut players = world.query::<&Player>();
    let total = players.iter(world).count();
    let mut bots = world.query_filtered::<&Player, With<Bot>>();
    let bots = bots.iter(world).count();
    (total, bots)
}

#[test]
fn a_lobby_fills_to_eight_and_eight_humans_leave_no_bots() {
    assert_eq!(ROOM_COMBATANTS, 8);
    // Fewer humans: bots fill the remaining seats, total stays 8.
    let mut app = balance_app();
    spawn_humans(&mut app, 2);
    app.update();
    assert_eq!(roster(&mut app), (8, 6));

    // Eight humans: zero bots.
    let mut app = balance_app();
    spawn_humans(&mut app, 8);
    app.update();
    assert_eq!(roster(&mut app), (8, 0));
}

#[test]
fn bots_leave_and_refill_so_the_room_stays_eight() {
    let mut app = balance_app();
    app.update();
    assert_eq!(roster(&mut app), (8, 8), "an empty lobby fields eight bots");

    // Two humans join: the newest bots leave so the total stays 8.
    let humans = spawn_humans(&mut app, 2);
    app.update();
    assert_eq!(roster(&mut app), (8, 6));

    // A human leaves: a bot takes the free seat back.
    assert!(app.world_mut().despawn(humans[0]));
    app.update();
    assert_eq!(roster(&mut app), (8, 7));
}

#[test]
fn a_playing_seat_is_refilled_by_a_fresh_bot() {
    let mut app = balance_app();
    let humans = spawn_humans(&mut app, 7);
    {
        let world = app.world_mut();
        let mut states = world.query::<&mut MatchState>();
        states.single_mut(world).unwrap().phase = Phase::Playing;
    }
    app.update();
    assert_eq!(roster(&mut app), (8, 1));
    assert!(app.world_mut().despawn(humans[0]));
    // Outside the lobby the fill waits out the one-second cooldown, then adds one bot.
    for _ in 0..64 {
        app.update();
    }
    app.update();
    assert_eq!(roster(&mut app), (8, 2));
    let world = app.world_mut();
    let mut bots = world.query_filtered::<(&Player, &Stats), With<Bot>>();
    let rows: Vec<(u32, u8)> = bots
        .iter(world)
        .map(|(_, stats)| (stats.score, stats.level))
        .collect();
    assert!(rows.iter().all(|(score, level)| *score == 0 && *level == 1));
}

#[test]
fn dead_bots_shop_then_deploy() {
    let mut evo = Evolution::default();
    let mut wallet = Wallet {
        scrap: 160,
        ..default()
    };
    let mut life = LifeLog::default();
    let mut vitals = Vitals {
        alive: false,
        respawn_in: RESPAWN_MAX_TICKS,
        ..default()
    };
    prep_bot(
        EvoPath::Cyborg,
        &mut evo,
        &mut wallet,
        &mut vitals,
        &mut life,
    );
    assert!(wallet.cart.is_empty(), "waits until it may deploy");
    vitals.respawn_in = RESPAWN_MAX_TICKS / 2;
    prep_bot(
        EvoPath::Cyborg,
        &mut evo,
        &mut wallet,
        &mut vitals,
        &mut life,
    );
    assert_eq!(
        wallet.cart,
        vec![
            ShopItem::Rifle,
            ShopItem::Kevlar,
            ShopItem::Emp,
            ShopItem::Emp
        ]
    );
    assert_eq!((wallet.scrap, vitals.respawn_in), (5, 1));
    assert!(
        life.deploy_asap,
        "a bot that may deploy confirms, so the next respawn tick revives it"
    );
}
