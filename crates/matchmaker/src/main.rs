//! EvoClash matchmaker.
//!
//! - `POST /v1/join`               → pick (or launch) a match, return a signed connect token
//! - `POST /v1/rejoin`             → fresh token for the same match after a disconnect
//! - `POST /v1/servers/heartbeat`  → match servers report load; reply may ask them to drain
//! - `GET  /v1/matches`            → fleet overview
//! - `POST /v1/matches/{id}/drain` → stop sending players to a match (deploys, maintenance)
//! - `GET  /healthz`               → liveness for the load balancer (clients also time it to pick a region)
//! - `GET  /metrics`               → Prometheus text format, for dashboards and autoscaling
//! - `GET  /play/`                 → the browser client (`--web-dir`), so it calls this API same-origin
//!
//! It holds no state that can't be rebuilt: the fleet view comes from heartbeats and
//! reconnect keys are HMACs, so a restart (or a replacement replica) loses nothing.
//! Run several replicas with `--peer` and every match heartbeats to all of them, so any
//! replica can place players and the others keep working when one goes down.

mod fleet;
mod registry;
mod rooms;
mod tokens;

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use evoclash_game::api::{CreateRoomRequest, Heartbeat, JoinRequest, JoinResponse, RejoinRequest};
use evoclash_game::net;
use evoclash_game::protocol::ROOM_COMBATANTS;
use serde_json::json;
use tower_http::services::ServeDir;

use crate::fleet::{Fleet, FleetConfig};
use crate::registry::{JoinPlan, MatchInfo, Registry};

struct AppState {
    registry: Mutex<Registry>,
    /// `None` when servers are provided by an external orchestrator (adopt-only mode).
    fleet: Option<Fleet>,
    key: [u8; 32],
    /// Empty spare matches kept running so joins never wait on a process start.
    warm_matches: usize,
    joins: AtomicU64,
    join_failures: AtomicU64,
    rejoins: AtomicU64,
}

type Shared = Arc<AppState>;

fn error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "error": code, "message": message.into() })),
    )
        .into_response()
}

fn error_retry(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
    retry_after: u32,
) -> Response {
    (
        status,
        Json(json!({
            "error": code,
            "message": message.into(),
            "retry_after": retry_after,
        })),
    )
        .into_response()
}

const BUSY: &str = "服务器繁忙，请稍后再试。";
const STARTED: &str = "这局已经开始，正在为你寻找新的房间…";
const STARTING: &str = "房间正在开局，请稍候。";

/// A freshly launched server has no certificate hash until its first heartbeat (well under a
/// second after start); browsers can't connect without it, so hold the reply until it arrives.
async fn cert_digest(state: &AppState, match_id: &str) -> Option<String> {
    for _ in 0..50 {
        let digest = state
            .registry
            .lock()
            .expect("registry lock")
            .matches
            .get(match_id)
            .and_then(|m| m.cert_digest.clone());
        if digest.is_some() {
            return digest;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

async fn issue(
    state: &AppState,
    match_id: String,
    addr: SocketAddr,
    client_id: u64,
    name: &str,
    room_code: Option<String>,
    spectate: bool,
) -> Response {
    let Some(cert_digest) = cert_digest(state, &match_id).await else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "busy", BUSY);
    };
    match tokens::connect_token(&state.key, addr, client_id, name) {
        Ok(connect_token) => Json(JoinResponse {
            reconnect_key: tokens::reconnect_key(&state.key, &match_id, client_id),
            match_id,
            server_addr: addr.to_string(),
            cert_digest,
            client_id,
            connect_token,
            room_code,
            spectate,
        })
        .into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, "internal", e),
    }
}

/// Start a match process and register it so it counts before its first heartbeat.
fn launch(registry: &mut Registry, fleet: &Fleet) -> Result<(String, SocketAddr), String> {
    let (id, addr) = fleet.launch(&registry.used_addrs())?;
    println!("launched match {id} at {addr}");
    registry.matches.insert(
        id.clone(),
        MatchInfo::new(id.clone(), addr, fleet.config.max_players),
    );
    Ok((id, addr))
}

/// Socket address when the listener recorded one. Missing (tests, odd proxies) still rate-limits.
struct ClientIp(String);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        Ok(ClientIp(ip))
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(16).collect()
}

fn room_error(fail: rooms::RoomFailure) -> Response {
    let status = StatusCode::from_u16(fail.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    match fail.retry_after {
        Some(retry_after) => error_retry(status, fail.error, fail.message, retry_after),
        None => error(status, fail.error, fail.message),
    }
}

fn new_client_id() -> u64 {
    rand::random_range(1..u64::MAX - (1 << 32))
}

async fn join(
    State(state): State<Shared>,
    ip: ClientIp,
    Json(request): Json<JoinRequest>,
) -> Response {
    if request.room.is_some() {
        return join_private(state, ip.0, request).await;
    }
    let name = sanitize_name(&request.name);
    // Ids below the bot range, never zero.
    let client_id = new_client_id();

    enum Seat {
        Ready(String, SocketAddr, bool),
        Deny(Response),
        Busy,
    }
    let seat = {
        let mut registry = state.registry.lock().expect("registry lock");
        match registry.plan_join(request.match_id.as_deref()) {
            JoinPlan::Seat(id, addr) => Seat::Ready(id, addr, false),
            JoinPlan::Started => Seat::Deny(error(StatusCode::CONFLICT, "started", STARTED)),
            JoinPlan::Starting { retry_after } => Seat::Deny(error_retry(
                StatusCode::CONFLICT,
                "starting",
                STARTING,
                retry_after,
            )),
            JoinPlan::Quick => match registry.reserve_seat() {
                Some((id, addr, spectate)) => Seat::Ready(id, addr, spectate),
                None => match &state.fleet {
                    Some(fleet) => match launch(&mut registry, fleet) {
                        Ok((id, addr)) => {
                            registry.reserve_in(&id);
                            Seat::Ready(id, addr, false)
                        }
                        Err(_) => Seat::Busy,
                    },
                    None => Seat::Busy,
                },
            },
        }
    };
    match seat {
        Seat::Ready(match_id, addr, spectate) => {
            println!("{name} ({client_id}) → {match_id}");
            state.joins.fetch_add(1, Ordering::Relaxed);
            issue(&state, match_id, addr, client_id, &name, None, spectate).await
        }
        Seat::Deny(response) => {
            state.join_failures.fetch_add(1, Ordering::Relaxed);
            response
        }
        Seat::Busy => {
            state.join_failures.fetch_add(1, Ordering::Relaxed);
            error(StatusCode::SERVICE_UNAVAILABLE, "busy", BUSY)
        }
    }
}

async fn join_private(state: Shared, ip: String, request: JoinRequest) -> Response {
    let name = sanitize_name(&request.name);
    let raw = request.room.unwrap_or_default();
    let client_id = new_client_id();
    let seat = {
        let mut registry = state.registry.lock().expect("registry lock");
        registry.lookup_room(&raw, &ip, Instant::now())
    };
    match seat {
        Ok(seat) => {
            println!("{} ({client_id}) → {} room {}", name, seat.id, seat.code);
            state.joins.fetch_add(1, Ordering::Relaxed);
            issue(
                &state,
                seat.id,
                seat.addr,
                client_id,
                &name,
                Some(seat.code),
                seat.spectate,
            )
            .await
        }
        Err(fail) => {
            state.join_failures.fetch_add(1, Ordering::Relaxed);
            room_error(fail)
        }
    }
}

async fn create_room(
    State(state): State<Shared>,
    ip: ClientIp,
    Json(request): Json<CreateRoomRequest>,
) -> Response {
    let name = sanitize_name(&request.name);
    let client_id = new_client_id();
    let launched = {
        let mut registry = state.registry.lock().expect("registry lock");
        if !registry.allow_ip(&ip.0, Instant::now()) {
            return room_error(rooms::RoomFailure::rate());
        }
        let Some(fleet) = &state.fleet else {
            return error(StatusCode::SERVICE_UNAVAILABLE, "busy", BUSY);
        };
        let Some(code) = registry.mint_code() else {
            return error(StatusCode::SERVICE_UNAVAILABLE, "busy", BUSY);
        };
        let room_seed = rand::random::<u64>();
        match fleet.launch_room(&registry.used_addrs(), room_seed, Some(&code)) {
            Ok((id, addr)) => {
                println!("launched private room {code} as {id} at {addr}");
                let mut info = MatchInfo::new(id.clone(), addr, fleet.config.max_players);
                info.is_private = true;
                info.room_code = Some(code.clone());
                info.room_seed = room_seed;
                registry.matches.insert(id.clone(), info);
                registry.reserve_in(&id);
                Some((id, addr, code))
            }
            Err(_) => None,
        }
    };
    match launched {
        Some((id, addr, code)) => {
            println!("{name} ({client_id}) creates {id} room {code}");
            state.joins.fetch_add(1, Ordering::Relaxed);
            issue(&state, id, addr, client_id, &name, Some(code), false).await
        }
        None => {
            state.join_failures.fetch_add(1, Ordering::Relaxed);
            error(StatusCode::SERVICE_UNAVAILABLE, "busy", BUSY)
        }
    }
}

async fn rejoin(State(state): State<Shared>, Json(request): Json<RejoinRequest>) -> Response {
    if !tokens::verify_reconnect_key(
        &state.key,
        &request.match_id,
        request.client_id,
        &request.reconnect_key,
    ) {
        return error(StatusCode::FORBIDDEN, "forbidden", "bad reconnect key");
    }
    let found = {
        let mut registry = state.registry.lock().expect("registry lock");
        let healthy = registry
            .matches
            .get(&request.match_id)
            .is_some_and(|m| m.healthy(std::time::Instant::now()));
        if !healthy {
            return error(StatusCode::GONE, "gone", "match is over or unreachable");
        }
        let room_code = registry
            .matches
            .get(&request.match_id)
            .and_then(|m| m.room_code.clone());
        registry
            .reserve_in(&request.match_id)
            .map(|addr| (addr, room_code))
    };
    state.rejoins.fetch_add(1, Ordering::Relaxed);
    match found {
        // The server knows the name from the original token; the body is kept on its side.
        Some((addr, room_code)) => {
            issue(
                &state,
                request.match_id,
                addr,
                request.client_id,
                "",
                room_code,
                false,
            )
            .await
        }
        None => error(StatusCode::GONE, "gone", "match not found"),
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

async fn metrics(State(state): State<Shared>) -> Response {
    use std::fmt::Write;

    let views = state.registry.lock().expect("registry lock").views();
    let count = |f: fn(&registry::MatchView) -> bool| views.iter().filter(|v| f(v)).count();
    let sum = |f: fn(&registry::MatchView) -> u64| views.iter().map(f).sum::<u64>();
    let mut out = String::new();
    let mut gauge = |name: &str, help: &str, value: u64| {
        let _ = writeln!(
            out,
            "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {value}"
        );
    };
    gauge(
        "evoclash_matches",
        "Matches known from heartbeats or launches.",
        views.len() as u64,
    );
    gauge(
        "evoclash_matches_healthy",
        "Matches heartbeating on time.",
        count(|v| v.healthy) as u64,
    );
    gauge(
        "evoclash_matches_draining",
        "Matches finishing up before exit.",
        count(|v| v.draining) as u64,
    );
    gauge(
        "evoclash_matches_overloaded",
        "Matches skipped for slow ticks.",
        count(|v| v.overloaded) as u64,
    );
    gauge(
        "evoclash_players",
        "Players in all matches.",
        sum(|v| v.players.into()),
    );
    gauge(
        "evoclash_seats_reserved",
        "Seats handed out, player not yet connected.",
        sum(|v| v.reserved as u64),
    );
    gauge(
        "evoclash_seats_total",
        "Player capacity of all matches.",
        sum(|v| v.max_players.into()),
    );
    if let Some(fleet) = &state.fleet {
        gauge(
            "evoclash_processes",
            "Match processes launched by this replica.",
            fleet.running() as u64,
        );
    }
    for (name, help, counter) in [
        ("evoclash_joins_total", "Join tokens issued.", &state.joins),
        (
            "evoclash_join_failures_total",
            "Joins refused for lack of capacity.",
            &state.join_failures,
        ),
        (
            "evoclash_rejoins_total",
            "Reconnect tokens issued.",
            &state.rejoins,
        ),
    ] {
        let value = counter.load(Ordering::Relaxed);
        let _ = writeln!(
            out,
            "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}"
        );
    }
    let _ = writeln!(
        out,
        "# HELP evoclash_tick_us Average simulation tick time per match.\n# TYPE evoclash_tick_us gauge"
    );
    for v in &views {
        let _ = writeln!(out, "evoclash_tick_us{{match=\"{}\"}} {}", v.id, v.tick_us);
    }
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], out).into_response()
}

async fn drain(State(state): State<Shared>, Path(id): Path<String>) -> Response {
    let mut registry = state.registry.lock().expect("registry lock");
    match registry.matches.get_mut(&id) {
        Some(m) => {
            m.draining = true;
            Json(json!({ "draining": id })).into_response()
        }
        None => error(StatusCode::NOT_FOUND, "not_found", "no such match"),
    }
}

/// Drop dead servers from the registry, collect exited processes and keep the warm pool full.
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
                state.registry.lock().expect("registry lock").forget(&id);
            }
            let mut registry = state.registry.lock().expect("registry lock");
            for _ in registry.idle_matches()..state.warm_matches {
                if let Err(e) = launch(&mut registry, fleet) {
                    println!("cannot warm a spare match: {e}");
                    break;
                }
            }
        }
    }
}

/// Ctrl-C locally, SIGTERM from a container runtime or process manager.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
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
    warm_matches: usize,
    peers: Vec<String>,
    web_dir: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut args = Args {
        listen: net::DEFAULT_MATCHMAKER_ADDR.parse().unwrap(),
        public_host: IpAddr::from([127, 0, 0, 1]),
        ports: (7000, 7099),
        max_matches: 16,
        max_players: ROOM_COMBATANTS.into(),
        bots: ROOM_COMBATANTS.into(),
        server_bin: None,
        log_dir: PathBuf::from("logs"),
        launch: true,
        warm_matches: 1,
        peers: Vec::new(),
        web_dir: None,
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
            "--max-players" => {
                // Every room is fixed at ROOM_COMBATANTS; clamp older flags to it.
                args.max_players = value("--max-players")
                    .parse::<u32>()
                    .expect("number")
                    .min(ROOM_COMBATANTS.into());
            }
            "--bots" => args.bots = value("--bots").parse().expect("number"),
            "--server-bin" => args.server_bin = Some(value("--server-bin").into()),
            "--log-dir" => args.log_dir = value("--log-dir").into(),
            "--no-launch" => args.launch = false,
            "--warm-matches" => {
                args.warm_matches = value("--warm-matches").parse().expect("number")
            }
            "--peer" => args.peers.extend(
                value("--peer")
                    .split(',')
                    .map(|url| url.trim().trim_end_matches('/').to_owned())
                    .filter(|url| !url.is_empty()),
            ),
            "--web-dir" => args.web_dir = Some(value("--web-dir").into()),
            "-h" | "--help" => {
                println!(
                    "evoclash-matchmaker [--listen 127.0.0.1:8080] [--public-host 127.0.0.1] [--ports 7000-7099]\n\
                     \x20                   [--max-matches 16] [--max-players 8] [--bots 8]\n\
                     \x20                   [--server-bin PATH] [--log-dir logs] [--no-launch]\n\
                     \x20                   [--warm-matches 1] [--peer URL[,URL...]] [--web-dir DIR]\n\n\
                     --no-launch     only adopt servers that heartbeat in (external orchestrator)\n\
                     --warm-matches  empty matches kept running so joins never wait on a launch\n\
                     --peer          other matchmaker replicas; launched servers heartbeat to all\n\
                     --web-dir       browser client to serve at /play/ (default: `web` next to this binary)"
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
            matchmaker_urls: std::iter::once(format!("http://{}", args.listen))
                .chain(args.peers.iter().cloned())
                .collect(),
            log_dir: args.log_dir.clone(),
        })
    });
    let state = Arc::new(AppState {
        registry: Mutex::default(),
        fleet,
        key,
        warm_matches: args.warm_matches,
        joins: AtomicU64::new(0),
        join_failures: AtomicU64::new(0),
        rejoins: AtomicU64::new(0),
    });
    tokio::spawn(janitor(state.clone()));

    let web_dir = args.web_dir.clone().or_else(|| {
        let bundled = std::env::current_exe().ok()?.with_file_name("web");
        bundled.is_dir().then_some(bundled)
    });
    let mut app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/join", post(join))
        .route("/v1/rooms", post(create_room))
        .route("/v1/rejoin", post(rejoin))
        .route("/v1/servers/heartbeat", post(heartbeat))
        .route("/metrics", get(metrics))
        .route("/v1/matches", get(list_matches))
        .route("/v1/matches/{id}/drain", post(drain))
        .with_state(state);
    match &web_dir {
        Some(dir) => {
            println!("serving the browser client from {}", dir.display());
            app = app
                .route("/", get(|| async { Redirect::to("/play/") }))
                .nest_service("/play", ServeDir::new(dir));
        }
        None => println!("no --web-dir: API only, the browser client is not served"),
    }

    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .unwrap_or_else(|e| panic!("bind {}: {e}", args.listen));
    println!("matchmaker listening on http://{}", args.listen);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        shutdown_signal().await;
        println!("shutting down; running matches keep going and will be re-adopted");
    })
    .await
    .expect("server error");
}
