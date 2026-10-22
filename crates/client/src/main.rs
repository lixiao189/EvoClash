//! EvoClash client.
//!
//! `evoclash --matchmaker http://127.0.0.1:8080 --name Alice`  (production path)
//! `evoclash --server 127.0.0.1:7000 --id 1`                    (dev: direct connect with the dev key)

mod effects;
mod hud;
mod input;
mod predict;
mod render;
mod session;

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::winit::WinitSettings;
use evoclash_game::net::tick_duration;
use lightyear::prelude::client::ClientPlugins;

use crate::session::Session;

#[derive(Resource, Clone, Debug)]
pub struct ClientArgs {
    pub name: String,
    pub matchmaker: Option<String>,
    pub server: Option<String>,
    pub client_id: u64,
    pub headless: bool,
    /// Drive inputs automatically (soak tests, headless load generation).
    pub autopilot: bool,
    /// Simulated one-way latency in ms for testing prediction (dev only).
    pub fake_latency_ms: u64,
}

fn parse_args() -> ClientArgs {
    let mut args = ClientArgs {
        name: std::env::var("USER").unwrap_or_else(|_| "幸存者".into()),
        matchmaker: None,
        server: None,
        client_id: rand::random::<u32>() as u64,
        headless: false,
        autopilot: false,
        fake_latency_ms: 0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .unwrap_or_else(|| panic!("missing value for {name}"))
        };
        match arg.as_str() {
            "--name" => args.name = value("--name"),
            "--matchmaker" => args.matchmaker = Some(value("--matchmaker")),
            "--server" => args.server = Some(value("--server")),
            "--id" => args.client_id = value("--id").parse().expect("invalid --id"),
            "--headless" => args.headless = true,
            "--autopilot" => args.autopilot = true,
            "--fake-latency" => {
                args.fake_latency_ms = value("--fake-latency")
                    .parse()
                    .expect("invalid --fake-latency")
            }
            "-h" | "--help" => {
                println!(
                    "evoclash [--matchmaker URL | --server HOST:PORT [--id N]] [--name NAME]\n\
                     \x20        [--headless] [--autopilot] [--fake-latency MS]"
                );
                std::process::exit(0);
            }
            other => panic!("unknown argument {other}"),
        }
    }
    if args.matchmaker.is_none() && args.server.is_none() {
        args.server = Some(format!(
            "127.0.0.1:{}",
            evoclash_game::net::DEFAULT_GAME_PORT
        ));
    }
    args
}

fn main() {
    let args = parse_args();
    let session = match Session::establish(&args) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("could not join a match: {e}");
            std::process::exit(1);
        }
    };

    let mut app = App::new();
    if args.headless {
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / 60.0,
            ))),
            LogPlugin::default(),
            StatesPlugin,
        ));
    } else {
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("EvoClash — {}", args.name),
                resolution: (1280, 800).into(),
                ..default()
            }),
            ..default()
        }));
        // Keep simulating at full rate when the window loses focus (multiple local clients).
        app.insert_resource(WinitSettings::continuous());
    }
    app.add_plugins(ClientPlugins {
        tick_duration: tick_duration(),
    });
    app.add_plugins(evoclash_game::SharedPlugin);
    app.insert_resource(args.clone());
    app.insert_resource(session);
    app.add_plugins((
        session::SessionPlugin,
        input::InputPlugin,
        predict::PredictPlugin,
    ));
    if args.headless {
        app.add_systems(Update, log_events_headless);
    } else {
        app.add_plugins((render::RenderPlugin, effects::EffectsPlugin, hud::HudPlugin));
    }
    app.run();
}

/// Without a HUD, still drain server events (and surface the kill feed in the log).
fn log_events_headless(
    mut receivers: Query<
        &mut lightyear::prelude::MessageReceiver<evoclash_game::protocol::GameEvent>,
    >,
) {
    use evoclash_game::protocol::GameEvent;
    for mut receiver in &mut receivers {
        for event in receiver.receive() {
            match event {
                GameEvent::Kill { killer, victim, .. } => info!("{killer} killed {victim}"),
                GameEvent::Announce(text) => info!("{text}"),
                _ => {}
            }
        }
    }
}
