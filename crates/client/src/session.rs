//! Getting into a match and staying there: matchmaker join, netcode connect, auto-reconnect.

use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use base64::Engine;
use bevy::prelude::*;
use evoclash_game::api::{JoinRequest, JoinResponse, RejoinRequest};
use evoclash_game::net::{self, CONNECTION_TIMEOUT_SECS, PROTOCOL_ID, TOKEN_EXPIRE_SECS};
use evoclash_game::protocol::{MatchState, Monster, Pickup, Player, Projectile};
use lightyear::connection::client::{Connected, Disconnected};
use lightyear::link::prelude::{LinkConditionerConfig, RecvLinkConditioner};
use lightyear::netcode::client_plugin::NetcodeConfig;
use lightyear::netcode::{ConnectToken, NetcodeClient};
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use serde::Serialize;

use crate::ClientArgs;

const MAX_RECONNECT_ATTEMPTS: u32 = 12;

#[derive(Clone)]
enum Credentials {
    /// Local development: build our own token from the shared dev key.
    Dev { private_key: [u8; 32] },
    /// Production: a token signed by the matchmaker.
    Token {
        /// Matchmaker replicas / regions, nearest first. Any of them can reissue tokens.
        matchmakers: Vec<String>,
        name: String,
        token: Vec<u8>,
        match_id: String,
        reconnect_key: String,
    },
}

#[derive(Resource, Clone)]
pub struct Session {
    pub server_addr: SocketAddr,
    pub client_id: u64,
    credentials: Credentials,
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into()
}

/// Order matchmakers by `/healthz` round trip, unreachable ones last. Matchmakers are deployed
/// next to their match servers, so the fastest one is also the region with the lowest ping.
fn rank_by_latency(matchmakers: &[String]) -> Vec<String> {
    if matchmakers.len() < 2 {
        return matchmakers.to_vec();
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(1)))
        .build()
        .into();
    let mut ranked: Vec<(Option<Duration>, String)> = std::thread::scope(|scope| {
        let probes: Vec<_> = matchmakers
            .iter()
            .map(|url| {
                let agent = agent.clone();
                scope.spawn(move || {
                    let started = Instant::now();
                    let healthz = format!("{}/healthz", url.trim_end_matches('/'));
                    let rtt = agent.get(&healthz).call().ok().map(|_| started.elapsed());
                    (rtt, url.clone())
                })
            })
            .collect();
        probes
            .into_iter()
            .map(|probe| probe.join().expect("probe thread"))
            .collect()
    });
    ranked.sort_by_key(|(rtt, _)| rtt.unwrap_or(Duration::MAX));
    for (rtt, url) in &ranked {
        info!("matchmaker {url}: {rtt:?}");
    }
    ranked.into_iter().map(|(_, url)| url).collect()
}

enum CallError {
    /// A matchmaker answered that the match is over or unknown: find a new one.
    Gone,
    Failed(String),
}

/// POST to each matchmaker in turn until one hands out a token.
fn post_any<T: Serialize>(
    matchmakers: &[String],
    path: &str,
    request: &T,
) -> Result<JoinResponse, CallError> {
    let agent = http_agent();
    let mut gone = false;
    let mut errors = Vec::new();
    for matchmaker in matchmakers {
        let url = format!("{}{path}", matchmaker.trim_end_matches('/'));
        match agent.post(&url).send_json(request) {
            Ok(mut response) => match response.body_mut().read_json() {
                Ok(reply) => return Ok(reply),
                Err(e) => errors.push(format!("{url}: bad reply: {e}")),
            },
            Err(ureq::Error::StatusCode(403 | 410)) => gone = true,
            Err(e) => errors.push(format!("{url}: {e}")),
        }
    }
    if gone {
        Err(CallError::Gone)
    } else {
        Err(CallError::Failed(errors.join("; ")))
    }
}

fn join_any(matchmakers: &[String], name: &str) -> Result<JoinResponse, String> {
    let request = JoinRequest {
        name: name.to_owned(),
        region: None,
    };
    post_any(matchmakers, "/v1/join", &request).map_err(|e| match e {
        CallError::Gone => "matchmaker refused the join".to_string(),
        CallError::Failed(e) => e,
    })
}

fn resolve(addr: &str) -> Result<SocketAddr, String> {
    addr.to_socket_addrs()
        .map_err(|e| format!("bad address {addr}: {e}"))?
        .next()
        .ok_or_else(|| format!("bad address {addr}"))
}

impl Session {
    pub fn establish(args: &ClientArgs) -> Result<Self, String> {
        if !args.matchmakers.is_empty() {
            let matchmakers = rank_by_latency(&args.matchmakers);
            let response = join_any(&matchmakers, &args.name)?;
            info!(
                "matched into {} at {}",
                response.match_id, response.server_addr
            );
            return Self::from_response(matchmakers, args.name.clone(), response);
        }
        let server = args.server.as_deref().unwrap_or("127.0.0.1:7000");
        let (private_key, _) = net::private_key_from_env();
        Ok(Self {
            server_addr: resolve(server)?,
            client_id: args.client_id,
            credentials: Credentials::Dev { private_key },
        })
    }

    fn from_response(
        matchmakers: Vec<String>,
        name: String,
        response: JoinResponse,
    ) -> Result<Self, String> {
        let token = base64::engine::general_purpose::STANDARD
            .decode(&response.connect_token)
            .map_err(|e| format!("bad token: {e}"))?;
        Ok(Self {
            server_addr: resolve(&response.server_addr)?,
            client_id: response.client_id,
            credentials: Credentials::Token {
                matchmakers,
                name,
                token,
                match_id: response.match_id,
                reconnect_key: response.reconnect_key,
            },
        })
    }

    /// Fetch a fresh token for the same match (tokens are single-use and short-lived) from
    /// whichever matchmaker replica is up. If the match is gone (its server crashed or it
    /// ended), or `new_match` is set because this one never let us in, join another one.
    fn refresh(&mut self, new_match: bool) -> Result<(), String> {
        let Credentials::Token {
            matchmakers,
            name,
            match_id,
            reconnect_key,
            ..
        } = &self.credentials
        else {
            return Ok(());
        };
        let rejoined = if new_match {
            Err(CallError::Gone)
        } else {
            let request = RejoinRequest {
                match_id: match_id.clone(),
                client_id: self.client_id,
                reconnect_key: reconnect_key.clone(),
            };
            post_any(matchmakers, "/v1/rejoin", &request)
        };
        let response = match rejoined {
            Ok(response) => response,
            Err(CallError::Gone) => {
                warn!("match {match_id} is gone, joining another one");
                join_any(matchmakers, name)?
            }
            Err(CallError::Failed(e)) => return Err(e),
        };
        *self = Self::from_response(matchmakers.clone(), name.clone(), response)?;
        Ok(())
    }

    fn authentication(&self) -> Result<Authentication, String> {
        Ok(match &self.credentials {
            Credentials::Dev { private_key } => Authentication::Manual {
                server_addr: self.server_addr,
                client_id: self.client_id,
                private_key: *private_key,
                protocol_id: PROTOCOL_ID,
            },
            Credentials::Token { token, .. } => Authentication::Token(
                ConnectToken::try_from_bytes(token).map_err(|e| format!("bad token: {e:?}"))?,
            ),
        })
    }
}

#[derive(Resource)]
struct LinkState {
    entity: Option<Entity>,
    since_change: f32,
    attempts: u32,
    ever_connected: bool,
}

/// Failed handshakes on a match we never got into (full, or died while starting) before
/// asking for a different match instead.
const NEW_MATCH_AFTER_ATTEMPTS: u32 = 2;

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(LinkState {
            entity: None,
            since_change: 0.0,
            attempts: 0,
            ever_connected: false,
        });
        app.add_systems(Startup, |world: &mut World| {
            // Predict ourselves with no artificial input delay: lowest perceived latency.
            world.insert_resource(
                InputTimelineConfig::default().with_input_delay(InputDelayConfig::no_input_delay()),
            );
            world.insert_resource(PredictionManager::default());
        });
        app.add_systems(Startup, connect);
        app.add_systems(Update, watch_connection);
    }
}

fn spawn_client(
    commands: &mut Commands,
    session: &Session,
    args: &ClientArgs,
) -> Result<Entity, String> {
    let conditioner = (args.fake_latency_ms > 0).then(|| {
        RecvLinkConditioner::new(LinkConditionerConfig {
            incoming_latency: Duration::from_millis(args.fake_latency_ms),
            incoming_jitter: Duration::from_millis(args.fake_latency_ms / 10),
            ..default()
        })
    });
    let netcode = NetcodeClient::new(
        session.authentication()?,
        NetcodeConfig {
            client_timeout_secs: CONNECTION_TIMEOUT_SECS,
            token_expire_secs: TOKEN_EXPIRE_SECS,
            ..default()
        },
    )
    .map_err(|e| format!("netcode: {e:?}"))?;
    let entity = commands
        .spawn((
            Name::new("Client"),
            Client,
            ReplicationReceiver,
            Link::default().with_conditioner(conditioner),
            LocalAddr(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)),
            PeerAddr(session.server_addr),
            netcode,
            UdpIo::default(),
        ))
        .id();
    commands.trigger(Connect { entity });
    Ok(entity)
}

fn connect(
    mut commands: Commands,
    session: Res<Session>,
    args: Res<ClientArgs>,
    mut link: ResMut<LinkState>,
) {
    match spawn_client(&mut commands, &session, &args) {
        Ok(entity) => {
            info!(
                "connecting to {} as client {}",
                session.server_addr, session.client_id
            );
            link.entity = Some(entity);
        }
        Err(e) => error!("cannot connect: {e}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn watch_connection(
    mut commands: Commands,
    time: Res<Time>,
    mut link: ResMut<LinkState>,
    mut session: ResMut<Session>,
    args: Res<ClientArgs>,
    clients: Query<(Has<Connected>, Has<Disconnected>), With<Client>>,
    replicated: Query<
        Entity,
        Or<(
            With<Player>,
            With<Pickup>,
            With<Projectile>,
            With<Monster>,
            With<MatchState>,
        )>,
    >,
    mut exit: MessageWriter<AppExit>,
) {
    // No live client entity (a previous retry failed half-way) counts as disconnected.
    let (connected, disconnected) = link
        .entity
        .and_then(|e| clients.get(e).ok())
        .unwrap_or((false, true));
    if connected {
        if link.attempts > 0 {
            info!("connection restored");
        }
        link.attempts = 0;
        link.since_change = 0.0;
        link.ever_connected = true;
        return;
    }
    link.since_change += time.delta_secs();
    // Give each attempt time to complete the handshake before retrying, backing off a little.
    let wait = 1.0 + link.attempts.min(4) as f32;
    if !disconnected || link.since_change < wait {
        return;
    }
    if link.attempts >= MAX_RECONNECT_ATTEMPTS {
        error!("giving up after {} reconnect attempts", link.attempts);
        exit.write(AppExit::error());
        return;
    }
    link.attempts += 1;
    link.since_change = 0.0;
    warn!("disconnected, reconnecting (attempt {})", link.attempts);

    // Start from a clean slate; the server resends the full world after the handshake.
    if let Some(entity) = link.entity.take() {
        commands.entity(entity).despawn();
    }
    for e in &replicated {
        commands.entity(e).despawn();
    }
    let new_match = !link.ever_connected && link.attempts > NEW_MATCH_AFTER_ATTEMPTS;
    if let Err(e) = session.refresh(new_match) {
        warn!("could not refresh token: {e}");
        return;
    }
    match spawn_client(&mut commands, &session, &args) {
        Ok(new_entity) => link.entity = Some(new_entity),
        Err(e) => warn!("reconnect failed: {e}"),
    }
}
