//! Getting into a match and staying there: matchmaker join, netcode connect, auto-reconnect.

use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use base64::Engine;
use bevy::prelude::*;
use evoclash_game::api::{JoinRequest, JoinResponse, RejoinRequest};
use evoclash_game::net::{self, CONNECTION_TIMEOUT_SECS, PROTOCOL_ID, TOKEN_EXPIRE_SECS};
use evoclash_game::protocol::{MatchState, Pickup, Player, Projectile};
use lightyear::connection::client::{Connected, Disconnected};
use lightyear::link::prelude::{LinkConditionerConfig, RecvLinkConditioner};
use lightyear::netcode::client_plugin::NetcodeConfig;
use lightyear::netcode::{ConnectToken, NetcodeClient};
use lightyear::prelude::client::*;
use lightyear::prelude::*;

use crate::ClientArgs;

const MAX_RECONNECT_ATTEMPTS: u32 = 12;

#[derive(Clone)]
enum Credentials {
    /// Local development: build our own token from the shared dev key.
    Dev { private_key: [u8; 32] },
    /// Production: a token signed by the matchmaker.
    Token {
        matchmaker: String,
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

fn resolve(addr: &str) -> Result<SocketAddr, String> {
    addr.to_socket_addrs()
        .map_err(|e| format!("bad address {addr}: {e}"))?
        .next()
        .ok_or_else(|| format!("bad address {addr}"))
}

impl Session {
    pub fn establish(args: &ClientArgs) -> Result<Self, String> {
        if let Some(matchmaker) = &args.matchmaker {
            let url = format!("{}/v1/join", matchmaker.trim_end_matches('/'));
            let request = JoinRequest {
                name: args.name.clone(),
                region: None,
            };
            let response: JoinResponse = http_agent()
                .post(&url)
                .send_json(&request)
                .map_err(|e| format!("matchmaker {url}: {e}"))?
                .body_mut()
                .read_json()
                .map_err(|e| format!("matchmaker reply: {e}"))?;
            info!("matched into {} at {}", response.match_id, response.server_addr);
            return Self::from_response(matchmaker.clone(), response);
        }
        let server = args.server.as_deref().unwrap_or("127.0.0.1:7000");
        let (private_key, _) = net::private_key_from_env();
        Ok(Self {
            server_addr: resolve(server)?,
            client_id: args.client_id,
            credentials: Credentials::Dev { private_key },
        })
    }

    fn from_response(matchmaker: String, response: JoinResponse) -> Result<Self, String> {
        let token = base64::engine::general_purpose::STANDARD
            .decode(&response.connect_token)
            .map_err(|e| format!("bad token: {e}"))?;
        Ok(Self {
            server_addr: resolve(&response.server_addr)?,
            client_id: response.client_id,
            credentials: Credentials::Token {
                matchmaker,
                token,
                match_id: response.match_id,
                reconnect_key: response.reconnect_key,
            },
        })
    }

    /// Fetch a fresh token for the same match (tokens are single-use and short-lived).
    fn refresh(&mut self) -> Result<(), String> {
        let Credentials::Token {
            matchmaker,
            match_id,
            reconnect_key,
            ..
        } = &self.credentials
        else {
            return Ok(());
        };
        let url = format!("{}/v1/rejoin", matchmaker.trim_end_matches('/'));
        let request = RejoinRequest {
            match_id: match_id.clone(),
            client_id: self.client_id,
            reconnect_key: reconnect_key.clone(),
        };
        let response: JoinResponse = http_agent()
            .post(&url)
            .send_json(&request)
            .map_err(|e| format!("rejoin {url}: {e}"))?
            .body_mut()
            .read_json()
            .map_err(|e| format!("rejoin reply: {e}"))?;
        *self = Self::from_response(matchmaker.clone(), response)?;
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
}

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(LinkState {
            entity: None,
            since_change: 0.0,
            attempts: 0,
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

fn spawn_client(commands: &mut Commands, session: &Session, args: &ClientArgs) -> Result<Entity, String> {
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
            info!("connecting to {} as client {}", session.server_addr, session.client_id);
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
    replicated: Query<Entity, Or<(With<Player>, With<Pickup>, With<Projectile>, With<MatchState>)>>,
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
    if let Err(e) = session.refresh() {
        warn!("could not refresh token: {e}");
        return;
    }
    match spawn_client(&mut commands, &session, &args) {
        Ok(new_entity) => link.entity = Some(new_entity),
        Err(e) => warn!("reconnect failed: {e}"),
    }
}
