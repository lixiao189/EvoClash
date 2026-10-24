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
    /// Matchmaker base URLs for heartbeats, e.g. `http://127.0.0.1:8080`. Every replica gets
    /// its own heartbeat, so any of them can place players here and survive the others failing.
    pub matchmakers: Vec<String>,
    /// Exit after one match ends (orchestrated mode: the fleet starts a fresh process).
    pub single_match: bool,
    /// Spawn wandering monsters and spore pods.
    pub monsters: bool,
    /// Worker threads for the ECS task pools. A match ticks in well under a millisecond, so one
    /// thread is plenty and keeps many match processes per host from oversubscribing the cores.
    pub threads: usize,
    /// Run as an Agones GameServer: identity and public address come from the SDK sidecar.
    pub agones: bool,
}

impl ServerConfig {
    pub fn from_args() -> Self {
        let mut port = DEFAULT_GAME_PORT;
        let mut public_addr: Option<SocketAddr> = None;
        let mut match_id = "local".to_string();
        let mut max_players = 6;
        let mut min_combatants = 4;
        let mut matchmakers = Vec::new();
        let mut single_match = false;
        let mut monsters = true;
        let mut threads = 1;
        let mut agones = false;

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = |name: &str| {
                args.next()
                    .unwrap_or_else(|| panic!("missing value for {name}"))
            };
            match arg.as_str() {
                "--port" => port = value("--port").parse().expect("invalid --port"),
                "--public-addr" => {
                    public_addr = Some(
                        value("--public-addr")
                            .parse()
                            .expect("invalid --public-addr"),
                    )
                }
                "--match-id" => match_id = value("--match-id"),
                "--max-players" => {
                    max_players = value("--max-players")
                        .parse()
                        .expect("invalid --max-players")
                }
                "--bots" => min_combatants = value("--bots").parse().expect("invalid --bots"),
                "--matchmaker" => matchmakers.extend(
                    value("--matchmaker")
                        .split(',')
                        .map(str::trim)
                        .filter(|url| !url.is_empty())
                        .map(str::to_owned),
                ),
                "--single-match" => single_match = true,
                "--no-monsters" => monsters = false,
                "--agones" => agones = true,
                "--threads" => {
                    threads = value("--threads")
                        .parse::<usize>()
                        .expect("invalid --threads")
                        .max(1)
                }
                "-h" | "--help" => {
                    println!(
                        "evoclash-server [--port 7000] [--public-addr 127.0.0.1:7000] [--match-id ID]\n\
                         \x20               [--max-players 6] [--bots 4] [--matchmaker URL[,URL...]] [--single-match]\n\
                         \x20               [--no-monsters] [--threads 1] [--agones]\n\n\
                         --bots N          keep at least N combatants in the arena by adding bots (0 disables)\n\
                         --matchmaker      heartbeat to every listed matchmaker, and to every address\n\
                         \x20                 a hostname resolves to (a headless Service reaches all pods)\n\
                         --agones          run as an Agones GameServer (implies --single-match)\n\
                         SIGTERM / Ctrl-C  drain: finish the current match, then exit (twice: exit now)"
                    );
                    std::process::exit(0);
                }
                other => panic!("unknown argument {other}"),
            }
        }

        Self {
            port,
            public_addr: public_addr.unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], port))),
            match_id,
            max_players,
            min_combatants,
            matchmakers,
            single_match,
            monsters,
            threads,
            agones,
        }
    }
}
