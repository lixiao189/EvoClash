use evoclash_game::evolution::{MAX_RANK, SKILL_A_RANK, SKILL_B_RANK, xp_to_reach};
use evoclash_game::items::{AmmoKind, MELEE_SLOT};
use evoclash_game::map::PropKind;
use evoclash_game::nav::NavGrid;
use evoclash_game::shop::RESPAWN_MAX_TICKS;
use evoclash_game::sim::PLAYER_RADIUS;
use lightyear::prelude::PeerId;

use super::build::{arrange, build_order, prep_bot};
use super::engage::{self, Side, Stance, stance};
use super::think::{interest, think};
use super::*;
use crate::gameplay::{MATCH_TICKS, SimTick};

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
            let mut banked = false;
            assert!(prep_bot(
                path,
                &mut evo,
                &mut wallet,
                &mut vitals,
                &mut life,
                WeaponKind::Stick,
                &mut banked,
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
        statefulset: false,
        room_seed: 1,
        private: false,
        room_code: String::new(),
        log_dir: std::env::temp_dir(),
        replay: None,
        recover: false,
        logging: false,
    });
    app.insert_resource(SimRng::from_match_seed(1));
    app.init_resource::<SimTick>();
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
    // Outside the lobby the fill waits for the next whole second of sim time, then adds one bot.
    for tick in 1..64 {
        app.world_mut().resource_mut::<SimTick>().0 = tick;
        app.update();
        assert_eq!(roster(&mut app), (7, 1), "no fill before tick 64");
    }
    app.world_mut().resource_mut::<SimTick>().0 = 64;
    app.update();
    assert_eq!(roster(&mut app), (8, 2));
    let world = app.world_mut();
    let mut bots = world.query_filtered::<(&Player, &Stats), With<Bot>>();
    let rows: Vec<(u64, u32, u8)> = bots
        .iter(world)
        .map(|(player, stats)| (player.client_id, stats.score, stats.level))
        .collect();
    assert!(
        rows.iter()
            .all(|(_, score, level)| *score == 0 && *level == 1)
    );
    // The backfill continues the numbering of the bots on the field, so a replay names it the same.
    let mut ids: Vec<u64> = rows.iter().map(|(id, ..)| *id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![u64::MAX - 2, u64::MAX - 1]);
}

#[test]
fn bots_hold_scrap_until_a_gun_is_worth_buying() {
    let (none, banked) = super::build::shop_list(20, WeaponKind::Stick, false);
    assert!(none.is_empty() && !banked);
    let (wait, banked) = super::build::shop_list(40, WeaponKind::Stick, false);
    assert!(
        wait.is_empty() && banked,
        "first short life keeps the scrap"
    );
    let (gun, banked) = super::build::shop_list(40, WeaponKind::Stick, true);
    assert_eq!(gun, vec![ShopItem::PipeShotgun]);
    assert!(!banked);
    let (again, _) = super::build::shop_list(40, WeaponKind::Bow, false);
    assert_eq!(again, vec![ShopItem::PipeShotgun]);
    let (rich, banked) = super::build::shop_list(130, WeaponKind::Stick, false);
    assert_eq!(rich, vec![ShopItem::Rifle, ShopItem::Kevlar]);
    assert!(!banked);
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
    let mut banked = false;
    prep_bot(
        EvoPath::Cyborg,
        &mut evo,
        &mut wallet,
        &mut vitals,
        &mut life,
        WeaponKind::Stick,
        &mut banked,
    );
    assert!(wallet.cart.is_empty(), "waits until it may deploy");
    vitals.respawn_in = RESPAWN_MAX_TICKS / 2;
    prep_bot(
        EvoPath::Cyborg,
        &mut evo,
        &mut wallet,
        &mut vitals,
        &mut life,
        WeaponKind::Stick,
        &mut banked,
    );
    assert_eq!(
        wallet.cart,
        vec![ShopItem::Rifle, ShopItem::Kevlar, ShopItem::Emp]
    );
    assert_eq!((wallet.scrap, vitals.respawn_in), (20, 1));
    assert!(
        life.deploy_asap,
        "a bot that may deploy confirms, so the next respawn tick revives it"
    );
}

fn side(tier: u8, hp: f32, armor: f32) -> Side {
    Side { tier, hp, armor }
}

fn mind(me: Side, them: Side, dist: f32, aggressor: bool, hp_frac: f32) -> Stance {
    stance(&engage::StanceInput {
        me,
        them,
        player: true,
        dist,
        aggressor,
        dangerous: false,
        stick: me.tier == 0,
        armed: false,
        aggression: 1.0,
        scavenging: false,
        cornered: false,
        hp_frac,
        melee: false,
        ghoul: false,
        crawler: false,
    })
}

#[test]
fn a_worse_loadout_does_not_pick_the_fight() {
    let me = side(0, 100.0, 0.0);
    let them = side(1, 100.0, 0.0);
    assert!(engage::clearly_worse(me, them, 1.0));
    assert!(engage::power_ratio(me, them) < 1.0);
    assert_eq!(mind(me, them, 400.0, false, 1.0), Stance::Avoid);
}

#[test]
fn a_better_loadout_does_pick_the_fight() {
    let me = side(2, 100.0, 0.0);
    let them = side(0, 100.0, 0.0);
    assert!(engage::has_advantage(me, them));
    assert_eq!(mind(me, them, 400.0, false, 1.0), Stance::Fight);
}

#[test]
fn being_hit_is_reason_enough_to_hit_back() {
    let me = side(0, 100.0, 0.0);
    let them = side(1, 100.0, 40.0);
    assert_eq!(mind(me, them, 400.0, true, 1.0), Stance::Fight);
}

#[test]
fn low_health_without_an_edge_retreats() {
    let me = side(1, 30.0, 0.0);
    let them = side(1, 100.0, 0.0);
    assert!(!engage::has_advantage(me, them));
    assert_eq!(mind(me, them, 220.0, true, 0.30), Stance::Retreat);
}

#[test]
fn a_stance_sticks_for_two_seconds_and_avoid_needs_a_real_edge() {
    use super::engage::{
        AVOID_EXIT_RATIO, HOLD_TICKS, MIND_AVOID, MIND_FIGHT, MIND_RETREAT, commit,
    };
    assert_eq!(
        commit(MIND_AVOID, MIND_FIGHT, HOLD_TICKS, 10, false, 2.0),
        MIND_AVOID,
        "still inside the hold"
    );
    assert_eq!(
        commit(MIND_RETREAT, MIND_AVOID, 10, 10, true, 0.5),
        MIND_FIGHT,
        "cornered or shot up close cuts the hold"
    );
    assert_eq!(
        commit(
            MIND_AVOID,
            MIND_FIGHT,
            0,
            10,
            false,
            AVOID_EXIT_RATIO - 0.05
        ),
        MIND_AVOID
    );
    assert_eq!(
        commit(MIND_AVOID, MIND_FIGHT, 0, 10, false, AVOID_EXIT_RATIO),
        MIND_FIGHT
    );
}

#[test]
fn melee_fauna_is_fought_and_a_stick_steps_back_from_a_ghoul() {
    let mut s = engage::StanceInput {
        me: side(0, 30.0, 0.0),
        them: side(0, 45.0, 0.0),
        player: false,
        dist: 80.0,
        aggressor: true,
        dangerous: true,
        armed: false,
        stick: true,
        aggression: 1.0,
        scavenging: false,
        cornered: false,
        hp_frac: 0.3,
        melee: true,
        ghoul: false,
        crawler: false,
    };
    assert_eq!(
        stance(&s),
        Stance::Fight,
        "a roach on you is not a footrace"
    );
    s.ghoul = true;
    s.dist = 220.0;
    assert_eq!(stance(&s), Stance::Avoid, "stick keeps clear of the blast");
    s.stick = false;
    s.me.tier = 2;
    assert_eq!(stance(&s), Stance::Fight);
}

#[test]
fn a_crawler_is_fought_from_behind_its_shell() {
    let face = 0.0;
    assert!(super::fight::on_shell(face, Vec2::X));
    assert!(!super::fight::on_rear(face, Vec2::X));
    assert!(super::fight::on_rear(face, -Vec2::X));
    assert!(!super::fight::on_shell(face, -Vec2::X));
}

#[test]
fn a_stick_avoids_a_raider_and_a_weaker_gun_does_too() {
    assert!(engage::dangerous_monster(MonsterKind::Raider, false));
    let stick = side(0, 100.0, 0.0);
    let raider = side(engage::RAIDER_TIER, 100.0, 0.0);
    let mut shy = engage::StanceInput {
        me: stick,
        them: raider,
        player: false,
        dist: 400.0,
        aggressor: false,
        dangerous: true,
        armed: false,
        stick: true,
        aggression: 1.0,
        scavenging: false,
        cornered: false,
        hp_frac: 1.0,
        melee: false,
        ghoul: false,
        crawler: false,
    };
    assert_eq!(stance(&shy), Stance::Avoid);
    shy.me = side(1, 100.0, 0.0);
    shy.stick = false;
    shy.dangerous = false;
    shy.armed = true;
    assert_eq!(stance(&shy), Stance::Avoid, "a bow is a tier down");
    shy.me = side(2, 100.0, 0.0);
    assert_eq!(stance(&shy), Stance::Fight);
}

#[test]
fn low_health_walks_out_of_radiation() {
    let zone = Rect::from_center_size(Vec2::ZERO, Vec2::splat(200.0));
    let exit = super::scavenge::hazard_exit(&[zone], Vec2::new(10.0, 0.0)).unwrap();
    assert!(!zone.contains(exit));
    assert!(super::scavenge::near_barrel(
        Vec2::ZERO,
        &[Vec2::new(100.0, 0.0)]
    ));
    assert!(!super::scavenge::near_barrel(
        Vec2::ZERO,
        &[Vec2::new(200.0, 0.0)]
    ));
}

#[test]
fn a_bot_with_nobody_around_swings_at_a_nearby_crate() {
    let arena = Arena::default();
    let (bot_pos, crate_pos) = open_pair(&arena);
    let nav = Nav(NavGrid::new(&arena.0, PLAYER_RADIUS + 2.0));
    let mut app = App::new();
    app.insert_resource(arena);
    app.insert_resource(nav);
    app.insert_resource(Obstacles::default())
        .insert_resource(SimRng::from_match_seed(1))
        .insert_resource(SimTick(0));
    app.world_mut().spawn(MatchState {
        phase: Phase::Playing,
        ticks_left: MATCH_TICKS,
        ..default()
    });
    let bot = app
        .world_mut()
        .spawn((
            Player {
                client_id: 9,
                name: "b".into(),
                color: [0.7; 3],
                bot: true,
            },
            Pos(bot_pos),
            Vitals {
                alive: true,
                ..default()
            },
            Loadout::default(),
            Evolution::default(),
            SkillState::default(),
            CurrentInput::default(),
            LifeLog::default(),
            Aggressors::default(),
            Bot::fresh(),
        ))
        .id();
    app.world_mut().spawn((
        Prop {
            kind: PropKind::Crate,
            tier: 1,
            progress: 0,
            open: false,
        },
        Pos(crate_pos),
        Vitals {
            hp: PropKind::Crate.max_hp(),
            alive: true,
            ..default()
        },
    ));
    app.add_systems(Update, think);
    app.update();
    let input = app.world().get::<CurrentInput>(bot).unwrap().0.clone();
    assert!(input.fire, "stands on the crate and swings");
    let club = WeaponKind::Stick.melee().unwrap();
    assert!(
        club.strikes(
            crate_pos - bot_pos,
            PropKind::Crate.radius(),
            input.aim,
            0.0,
        ),
        "the swing reaches the crate"
    );
    assert!(
        PropKind::Crate.max_hp() <= club.damage() * 2.0,
        "two sticks break a crate"
    );
}

fn open_pair(arena: &Arena) -> (Vec2, Vec2) {
    for origin in &arena.0.spawn_points {
        for step in [50.0_f32, 40.0, -50.0, 36.0] {
            let crate_pos = *origin + Vec2::new(step, step * 0.2);
            if arena.0.line_of_sight(*origin, crate_pos)
                && !arena.0.in_hazard(*origin)
                && !arena.0.in_hazard(crate_pos)
            {
                return (*origin, crate_pos);
            }
        }
    }
    panic!("no clear ground beside a spawn");
}
