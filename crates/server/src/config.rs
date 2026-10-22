use std::net::SocketAddr;

use bevy::prelude::*;
use evoclash_game::net::DEFAULT_GAME_PORT;

#[derive(Resource, Clone, Debug)]
pub struct ServerConfig {
    /// UDP port to bind on all interfaces.
    pub port: u16,
    /// Address clients use to reach this server (what connect tokens point at).
    pub public_addr: SocketAddr,
    pub match_id: String,
    pub max_players: usize,
    /// Bots fill the arena up to this many combatants and leave as humans join.
    pub min_combatants: usize,
    /// Matchmaker base URL for heartbeats, e.g. `http://127.0.0.1:8080`.
    pub matchmaker: Option<String>,
    /// Exit after one match ends (orchestrated mode: the fleet starts a fresh process).
    pub single_match: bool,
}

impl ServerConfig {
    pub fn from_args() -> Self {
        let mut port = DEFAULT_GAME_PORT;
        let mut public_addr: Option<SocketAddr> = None;
        let mut match_id = "local".to_string();
        let mut max_players = 6;
        let mut min_combatants = 4;
        let mut matchmaker = None;
        let mut single_match = false;

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = |name: &str| {
                args.next()
                    .unwrap_or_else(|| panic!("missing value for {name}"))
            };
            match arg.as_str() {
                "--port" => port = value("--port").parse().expect("invalid --port"),
                "--public-addr" => {
                    public_addr = Some(value("--public-addr").parse().expect("invalid --public-addr"))
                }
                "--match-id" => match_id = value("--match-id"),
                "--max-players" => {
                    max_players = value("--max-players").parse().expect("invalid --max-players")
                }
                "--bots" => {
                    min_combatants = value("--bots").parse().expect("invalid --bots")
                }
                "--matchmaker" => matchmaker = Some(value("--matchmaker")),
                "--single-match" => single_match = true,
                "-h" | "--help" => {
                    println!(
                        "evoclash-server [--port 7000] [--public-addr 127.0.0.1:7000] [--match-id ID]\n\
                         \x20               [--max-players 6] [--bots 4] [--matchmaker http://host:8080] [--single-match]\n\n\
                         --bots N   keep at least N combatants in the arena by adding bots (0 disables)"
                    );
                    std::process::exit(0);
                }
                other => panic!("unknown argument {other}"),
            }
        }

        Self {
            port,
            public_addr: public_addr
                .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], port))),
            match_id,
            max_players,
            min_combatants,
            matchmaker,
            single_match,
        }
    }
}
