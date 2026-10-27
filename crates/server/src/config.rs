use std::net::SocketAddr;
use std::path::PathBuf;

use bevy::prelude::*;
use evoclash_game::net::DEFAULT_GAME_PORT;
use evoclash_game::protocol::ROOM_COMBATANTS;

#[derive(Resource, Clone, Debug)]
pub struct ServerConfig {
    /// UDP port (WebTransport over QUIC) to bind on all interfaces.
    pub port: u16,
    /// Address clients use to reach this server (what connect tokens point at).
    pub public_addr: SocketAddr,
    pub match_id: String,
    /// Maximum connected humans. Production rooms are always [`ROOM_COMBATANTS`]; tests and the
    /// self-play harness construct this config directly.
    pub max_players: usize,
    /// Combatant target including bots: [`ROOM_COMBATANTS`] in production. Used only as the
    /// fallback fill target when no replicated room settings exist (tests, self-play).
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
    /// Fixed for the life of this process. Every round's map seed is `mix(room_seed, match_index)`.
    pub room_seed: u64,
    /// Friends-only room. The countdown waits for the host, and quick start will not offer it.
    pub private: bool,
    /// Uppercase room code. Empty unless `private`.
    pub room_code: String,
    /// Directory for room and match logs. Default `logs/`.
    pub log_dir: PathBuf,
    /// Offline `--replay` input. The process does not bind or heartbeat.
    pub replay: Option<PathBuf>,
    /// Write room and match logs. Replay turns this off.
    pub logging: bool,
}

/// The one OS random draw: the startup seed when `--room-seed` is omitted.
#[allow(clippy::disallowed_methods)]
fn startup_seed() -> u64 {
    rand::random()
}

impl ServerConfig {
    pub fn from_args() -> Self {
        let mut port = DEFAULT_GAME_PORT;
        let mut public_addr: Option<SocketAddr> = None;
        let mut match_id = "local".to_string();
        let max_players = ROOM_COMBATANTS as usize;
        let min_combatants = ROOM_COMBATANTS as usize;
        let mut matchmakers = Vec::new();
        let mut single_match = false;
        let mut monsters = true;
        let mut threads = 1;
        let mut agones = false;
        let mut room_seed = None;
        let mut private = false;
        let mut room_code = String::new();
        let mut log_dir = PathBuf::from("logs");
        let mut replay = None;

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
                // Every production room is fixed at ROOM_COMBATANTS. Both flags are accepted so
                // existing scripts/Helm values keep working, but their values are ignored.
                "--max-players" => {
                    value("--max-players")
                        .parse::<usize>()
                        .expect("invalid --max-players");
                }
                "--bots" => {
                    value("--bots").parse::<usize>().expect("invalid --bots");
                }
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
                "--room-seed" => {
                    room_seed = Some(value("--room-seed").parse().expect("invalid --room-seed"))
                }
                "--private" => private = true,
                "--room-code" => room_code = value("--room-code").trim().to_uppercase(),
                "--log-dir" => log_dir = PathBuf::from(value("--log-dir")),
                "--replay" => replay = Some(PathBuf::from(value("--replay"))),
                "-h" | "--help" => {
                    println!(
                        "evoclash-server [--port 7000] [--public-addr 127.0.0.1:7000] [--match-id ID]\n\
                         \x20               [--max-players 8] [--bots 6] [--matchmaker URL[,URL...]] [--single-match]\n\
                         \x20               [--no-monsters] [--threads 1] [--agones] [--room-seed U64]\n\
                         \x20               [--private] [--room-code CODE] [--log-dir logs] [--replay FILE]\n\n\
                         --bots N          keep at least N combatants in the arena by adding bots (0 disables)\n\
                         --room-seed       fixed seed for this room; omitted once, from the OS\n\
                         --private         friends-only room; the host starts the match\n\
                         --room-code       4-character code for a private room\n\
                         --log-dir         room and match logs (default logs/)\n\
                         --replay FILE     offline match log: no port, no matchmaker; exit 1 on digest mismatch\n\
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
            room_seed: room_seed.unwrap_or_else(startup_seed),
            private,
            room_code,
            log_dir,
            replay,
            logging: true,
        }
    }
}
