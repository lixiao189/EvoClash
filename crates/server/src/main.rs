//! EvoClash dedicated match server.
//!
//! One process hosts exactly one match. Scaling out means running more processes
//! (one per core is a good default, and each process sticks to `--threads` worker threads
//! so they don't fight over cores); a crash or deploy only affects the matches on
//! that process, and the matchmaker routes new players around it.

mod agones;
mod boss;
mod bots;
mod combat;
mod config;
mod connections;
mod environment;
mod evolution;
mod gameplay;
mod inventory;
mod journal;
mod monsters;
mod ops;
mod rng;
mod skills;

use std::time::Duration;

use bevy::app::{ScheduleRunnerPlugin, TaskPoolOptions, TaskPoolPlugin};
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use evoclash_game::net::{REPLICATION_INTERVAL, tick_duration};
use lightyear::prelude::server::ServerPlugins;
use lightyear::prelude::*;

use crate::config::ServerConfig;

fn main() {
    let mut config = ServerConfig::from_args();
    if let Some(path) = config.replay.clone() {
        // Offline: do not bind a port and do not talk to the matchmaker.
        let code = journal::replay_cli(&path);
        std::process::exit(code);
    }
    let agones = config.agones.then(|| {
        let sdk = agones::AgonesSdk::from_env();
        let (name, public_addr) = sdk
            .discover()
            .unwrap_or_else(|e| panic!("cannot reach the Agones SDK sidecar: {e}"));
        config.match_id = name;
        config.public_addr = public_addr;
        // One pod per match: Agones replaces it with a fresh Ready server afterwards.
        config.single_match = true;
        sdk
    });

    let mut app = App::new();
    app.add_plugins((
        // Wake up often enough that a fixed tick never waits on the loop, without spinning a core.
        MinimalPlugins
            .set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(1)))
            .set(TaskPoolPlugin {
                task_pool_options: TaskPoolOptions::with_num_threads(config.threads),
            }),
        LogPlugin::default(),
        StatesPlugin,
    ));
    app.add_plugins(ServerPlugins {
        tick_duration: tick_duration(),
    });
    app.insert_resource(ReplicationMetadata::new(REPLICATION_INTERVAL));
    app.add_plugins(evoclash_game::SharedPlugin);
    app.insert_resource(connections::TlsIdentity::self_signed(&config));
    info!("room seed {}", config.room_seed);
    app.insert_resource(config);
    app.add_plugins((
        connections::ConnectionsPlugin,
        gameplay::GameplayPlugin,
        combat::CombatPlugin,
        inventory::InventoryPlugin,
        environment::EnvironmentPlugin,
        bots::BotsPlugin,
        monsters::MonstersPlugin,
        boss::BossPlugin,
        ops::OpsPlugin,
        evolution::EvolutionPlugin,
        skills::SkillsPlugin,
        journal::JournalPlugin,
    ));
    if let Some(sdk) = &agones {
        app.add_plugins(agones::AgonesPlugin(sdk.clone()));
    }
    let exit = app.run();
    if let Some(sdk) = agones
        && let Err(e) = sdk.shutdown()
    {
        eprintln!("{e}");
    }
    std::process::exit(if exit.is_success() { 0 } else { 1 });
}
