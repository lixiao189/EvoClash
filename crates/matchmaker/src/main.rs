//! EvoClash matchmaker.
//!
//! - `POST /v1/join`               → pick (or launch) a match, return a signed connect token
//! - `POST /v1/rejoin`             → fresh token for the same match after a disconnect
//! - `POST /v1/servers/heartbeat`  → match servers report load; reply may ask them to drain
//! - `GET  /v1/matches`            → fleet overview
//! - `POST /v1/matches/{id}/drain` → stop sending players to a match (deploys, maintenance)
//! - `GET  /healthz`               → liveness for the load balancer
//!
//! It holds no state that can't be rebuilt: the fleet view comes from heartbeats and
//! reconnect keys are HMACs, so a restart (or a replacement replica) loses nothing.

mod fleet;
mod registry;
mod tokens;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use evoclash_game::api::{Heartbeat, JoinRequest, JoinResponse, RejoinRequest};
use evoclash_game::net;
use serde_json::json;

use crate::fleet::{Fleet, FleetConfig};
use crate::registry::{MatchInfo, Registry};

struct AppState {
    registry: Mutex<Registry>,
    /// `None` when servers are provided by an external orchestrator (adopt-only mode).
    fleet: Option<Fleet>,
    key: [u8; 32],
}

type Shared = Arc<AppState>;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn issue(
    state: &AppState,
    match_id: String,
    addr: SocketAddr,
    client_id: u64,
    name: &str,
) -> Response {
    match tokens::connect_token(&state.key, addr, client_id, name) {
        Ok(connect_token) => Json(JoinResponse {
            reconnect_key: tokens::reconnect_key(&state.key, &match_id, client_id),
            match_id,
            server_addr: addr.to_string(),
            client_id,
            connect_token,
        })
        .into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn join(State(state): State<Shared>, Json(request): Json<JoinRequest>) -> Response {
    let name: String = request
        .name
        .chars()
        .filter(|c| !c.is_control())
        .take(16)
        .collect();
    // Ids below the bot range, never zero.
    let client_id = rand::random_range(1..u64::MAX - (1 << 32));

    let seat = {
        let mut registry = state.registry.lock().expect("registry lock");
        match registry.reserve_seat() {
            Some(seat) => Ok(seat),
            None => match &state.fleet {
                Some(fleet) => fleet.launch(&registry.used_addrs()).map(|(id, addr)| {
                    println!("launched match {id} at {addr}");
                    let max = fleet.config.max_players;
                    registry
                        .matches
                        .insert(id.clone(), MatchInfo::new(id.clone(), addr, max));
                    registry.reserve_in(&id);
                    (id, addr)
                }),
                None => Err("no match with free seats".to_string()),
            },
        }
    };
    match seat {
        Ok((match_id, addr)) => {
            println!("{name} ({client_id}) → {match_id}");
            issue(&state, match_id, addr, client_id, &name)
        }
        Err(e) => error(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn rejoin(State(state): State<Shared>, Json(request): Json<RejoinRequest>) -> Response {
    if !tokens::verify_reconnect_key(
        &state.key,
        &request.match_id,
        request.client_id,
        &request.reconnect_key,
    ) {
        return error(StatusCode::FORBIDDEN, "bad reconnect key");
    }
    let addr = {
        let mut registry = state.registry.lock().expect("registry lock");
        let healthy = registry
            .matches
            .get(&request.match_id)
            .is_some_and(|m| m.healthy(std::time::Instant::now()));
        if !healthy {
            return error(StatusCode::GONE, "match is over or unreachable");
        }
        registry.reserve_in(&request.match_id)
    };
    match addr {
        // The server knows the name from the original token; the body is kept on its side.
        Some(addr) => issue(&state, request.match_id, addr, request.client_id, ""),
        None => error(StatusCode::GONE, "match not found"),
    }
}

async fn heartbeat(State(state): State<Shared>, Json(beat): Json<Heartbeat>) -> Response {
    let drain = state
        .registry
        .lock()
        .expect("registry lock")
        .heartbeat(&beat);
    Json(json!({ "drain": drain })).into_response()
}

async fn list_matches(State(state): State<Shared>) -> Response {
    let mut registry = state.registry.lock().expect("registry lock");
    let running = state.fleet.as_ref().map(|f| f.running());
    Json(json!({ "matches": registry.views(), "launched_processes": running })).into_response()
}

async fn drain(State(state): State<Shared>, Path(id): Path<String>) -> Response {
    let mut registry = state.registry.lock().expect("registry lock");
    match registry.matches.get_mut(&id) {
        Some(m) => {
            m.draining = true;
            Json(json!({ "draining": id })).into_response()
        }
        None => error(StatusCode::NOT_FOUND, "no such match"),
    }
}

/// Drop dead servers from the registry and collect exited processes.
async fn janitor(state: Shared) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        interval.tick().await;
        let dead = state.registry.lock().expect("registry lock").reap();
        for id in dead {
            println!("match {id} stopped heartbeating; removed");
            if let Some(fleet) = &state.fleet {
                fleet.kill(&id);
            }
        }
        if let Some(fleet) = &state.fleet {
            for (id, code) in fleet.reap() {
                println!("match {id} exited ({code:?})");
                state
                    .registry
                    .lock()
                    .expect("registry lock")
                    .matches
                    .remove(&id);
            }
        }
    }
}

struct Args {
    listen: SocketAddr,
    public_host: IpAddr,
    ports: (u16, u16),
    max_matches: usize,
    max_players: u32,
    bots: u32,
    server_bin: Option<PathBuf>,
    log_dir: PathBuf,
    launch: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        listen: net::DEFAULT_MATCHMAKER_ADDR.parse().unwrap(),
        public_host: IpAddr::from([127, 0, 0, 1]),
        ports: (7000, 7099),
        max_matches: 16,
        max_players: 6,
        bots: 4,
        server_bin: None,
        log_dir: PathBuf::from("logs"),
        launch: true,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .unwrap_or_else(|| panic!("missing value for {name}"))
        };
        match arg.as_str() {
            "--listen" => args.listen = value("--listen").parse().expect("invalid --listen"),
            "--public-host" => {
                args.public_host = value("--public-host")
                    .parse()
                    .expect("invalid --public-host")
            }
            "--ports" => {
                let v = value("--ports");
                let (a, b) = v.split_once('-').expect("--ports like 7000-7099");
                args.ports = (a.parse().expect("port"), b.parse().expect("port"));
            }
            "--max-matches" => args.max_matches = value("--max-matches").parse().expect("number"),
            "--max-players" => args.max_players = value("--max-players").parse().expect("number"),
            "--bots" => args.bots = value("--bots").parse().expect("number"),
            "--server-bin" => args.server_bin = Some(value("--server-bin").into()),
            "--log-dir" => args.log_dir = value("--log-dir").into(),
            "--no-launch" => args.launch = false,
            "-h" | "--help" => {
                println!(
                    "evoclash-matchmaker [--listen 127.0.0.1:8080] [--public-host 127.0.0.1] [--ports 7000-7099]\n\
                     \x20                   [--max-matches 16] [--max-players 6] [--bots 4]\n\
                     \x20                   [--server-bin PATH] [--log-dir logs] [--no-launch]\n\n\
                     --no-launch  only adopt servers that heartbeat in (external orchestrator)"
                );
                std::process::exit(0);
            }
            other => panic!("unknown argument {other}"),
        }
    }
    args
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let (key, dev_key) = net::private_key_from_env();
    if dev_key {
        eprintln!(
            "warning: {} not set, using the insecure development key",
            net::KEY_ENV
        );
    }
    let fleet = args.launch.then(|| {
        let server_bin = args.server_bin.clone().unwrap_or_else(|| {
            let exe = std::env::current_exe().expect("current exe");
            exe.with_file_name(format!("evoclash-server{}", std::env::consts::EXE_SUFFIX))
        });
        Fleet::new(FleetConfig {
            server_bin,
            public_host: args.public_host,
            ports: args.ports.0..=args.ports.1,
            max_matches: args.max_matches,
            max_players: args.max_players,
            bots: args.bots,
            matchmaker_url: format!("http://{}", args.listen),
            log_dir: args.log_dir.clone(),
        })
    });
    let state = Arc::new(AppState {
        registry: Mutex::default(),
        fleet,
        key,
    });
    tokio::spawn(janitor(state.clone()));

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/join", post(join))
        .route("/v1/rejoin", post(rejoin))
        .route("/v1/servers/heartbeat", post(heartbeat))
        .route("/v1/matches", get(list_matches))
        .route("/v1/matches/{id}/drain", post(drain))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .unwrap_or_else(|e| panic!("bind {}: {e}", args.listen));
    println!("matchmaker listening on http://{}", args.listen);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            println!("shutting down; running matches keep going and will be re-adopted");
        })
        .await
        .expect("server error");
}
