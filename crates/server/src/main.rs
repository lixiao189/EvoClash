//! EvoClash dedicated match server.
//!
//! One process hosts exactly one match. Scaling out means running more processes
//! (one per core is a good default, and each process sticks to `--threads` worker threads
//! so they don't fight over cores); a crash or deploy only affects the matches on
//! that process, and the matchmaker routes new players around it.

mod boss;
mod bots;
mod combat;
mod config;
mod connections;
mod environment;
mod evolution;
mod gameplay;
mod headless;
mod inbox;
mod inventory;
mod journal;
mod k8s;
mod monsters;
mod ops;
mod rng;
mod skills;
mod spectate;
mod stats;

use std::time::Duration;

use bevy::app::{ScheduleRunnerPlugin, TaskPoolOptions, TaskPoolPlugin};
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use evoclash_game::net::{REPLICATION_INTERVAL, tick_duration};
use lightyear::prelude::server::ServerPlugins;
use lightyear::prelude::*;

use crate::config::{Launch, ServerConfig};

fn main() {
    let launch = ServerConfig::parse_launch();
    let mut config = match launch {
        Launch::Replay(path) => {
            let code = journal::replay_cli(&path);
            std::process::exit(code);
        }
        Launch::Headless(opts) => {
            let code = headless::run(&opts);
            std::process::exit(code);
        }
        Launch::Healthcheck(path) => std::process::exit(k8s::healthcheck(&path)),
        Launch::Serve(config) => config,
    };
    // `--replay` is taken before this config exists; the field stays for the shared struct.
    debug_assert!(config.replay.is_none());
    if config.statefulset {
        let id = k8s::identity_from_env().unwrap_or_else(|e| panic!("--statefulset: {e}"));
        config.port = id.port;
        config.public_addr = id.public_addr;
        // The journal on this pod's volume decides: resume the room it holds (a crash restart)
        // or open a new one. Either way the pod keeps its address and port.
        match k8s::start_from_journal(&config.log_dir) {
            k8s::Start::Resume {
                match_id,
                room_seed,
            } => {
                config.match_id = match_id;
                config.room_seed = room_seed;
                config.recover = true;
                // println, not info: the logger is not installed yet at this point.
                println!(
                    "statefulset pod {} (ordinal {}) → resuming match {} on {} from {}",
                    id.pod,
                    id.ordinal,
                    config.match_id,
                    id.public_addr,
                    config.log_dir.display()
                );
            }
            k8s::Start::Fresh => {
                // The journal has the last word in this mode, even over an explicit `--recover`:
                // a fresh room has no log to resume.
                config.recover = false;
                config.match_id = id.match_id.clone();
                println!(
                    "statefulset pod {} (ordinal {}) → match {} on {}",
                    id.pod, id.ordinal, config.match_id, id.public_addr
                );
            }
        }
    }
    // A graceful exit closes the room log so the next container start does not resume a room
    // that was drained on purpose.
    let journal_dir = config.log_dir.clone();
    let journal_id = config.match_id.clone();
    if config.recover {
        info!("recovering match {}", config.match_id);
    }

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
    let statefulset = config.statefulset;
    app.insert_resource(config);
    app.add_plugins((
        connections::ConnectionsPlugin,
        inbox::InboxPlugin,
        gameplay::GameplayPlugin,
        spectate::SpectatePlugin,
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
    if statefulset {
        app.add_plugins(k8s::K8sPlugin::from_env());
    }
    let exit = app.run();
    drop(app);
    if exit.is_success() {
        journal::mark_finished(&journal_dir, &journal_id);
    }
    std::process::exit(if exit.is_success() { 0 } else { 1 });
}
