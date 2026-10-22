//! EvoClash dedicated match server.
//!
//! One process hosts exactly one match. Scaling out means running more processes
//! (one per core is a good default); a crash or deploy only affects the matches on
//! that process, and the matchmaker routes new players around it.

mod bots;
mod combat;
mod config;
mod connections;
mod gameplay;
mod ops;

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use evoclash_game::net::{REPLICATION_INTERVAL, tick_duration};
use lightyear::prelude::server::ServerPlugins;
use lightyear::prelude::*;

use crate::config::ServerConfig;

fn main() {
    let config = ServerConfig::from_args();

    let mut app = App::new();
    app.add_plugins((
        // Wake up often enough that a fixed tick never waits on the loop, without spinning a core.
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(1))),
        LogPlugin::default(),
        StatesPlugin,
    ));
    app.add_plugins(ServerPlugins {
        tick_duration: tick_duration(),
    });
    app.insert_resource(ReplicationMetadata::new(REPLICATION_INTERVAL));
    app.add_plugins(evoclash_game::SharedPlugin);
    app.insert_resource(config);
    app.add_plugins((
        connections::ConnectionsPlugin,
        gameplay::GameplayPlugin,
        combat::CombatPlugin,
        bots::BotsPlugin,
        ops::OpsPlugin,
    ));
    app.run();
}
