//! Getting into a match and staying there: matchmaker join, netcode connect, auto-reconnect.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use bevy::prelude::*;
use evoclash_game::api::{JoinRequest, JoinResponse, RejoinRequest};
use evoclash_game::net::{CONNECTION_TIMEOUT_SECS, TOKEN_EXPIRE_SECS};
use evoclash_game::protocol::{MatchState, Monster, Pickup, Player, Projectile};
use lightyear::connection::client::{Connected, Disconnected};
use lightyear::link::prelude::{LinkConditionerConfig, RecvLinkConditioner};
use lightyear::netcode::client_plugin::NetcodeConfig;
use lightyear::netcode::{ConnectToken, NetcodeClient};
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use lightyear::webtransport::prelude::client::WebTransportClientIo;
use serde::Serialize;

use crate::ClientArgs;

const MAX_RECONNECT_ATTEMPTS: u32 = 12;

enum CallError {
    /// The matchmaker answered that the match is over or unknown: find a new one.
    Gone,
    Failed(String),
}

/// A matchmaker request in flight. Browsers can't block on HTTP, so the reply lands here and
/// `watch_connection` picks it up on a later frame.
type PendingCall = Arc<Mutex<Option<Result<JoinResponse, CallError>>>>;

fn post<T: Serialize>(matchmaker: &str, path: &str, request: &T) -> PendingCall {
    let pending = PendingCall::default();
    let url = format!("{}{path}", matchmaker.trim_end_matches('/'));
    let mut http = ehttp::Request::post(
        &url,
        serde_json::to_vec(request).expect("requests serialize"),
    );
    http.headers.insert("Content-Type", "application/json");
    let slot = pending.clone();
    ehttp::fetch(http, move |result| {
        let reply = match result {
            Ok(response) if response.ok => serde_json::from_slice(&response.bytes)
                .map_err(|e| CallError::Failed(format!("{url}: bad reply: {e}"))),
            Ok(response) if matches!(response.status, 403 | 410) => Err(CallError::Gone),
            Ok(response) => Err(CallError::Failed(format!(
                "{url}: {} {}",
                response.status,
                String::from_utf8_lossy(&response.bytes)
            ))),
            Err(e) => Err(CallError::Failed(format!("{url}: {e}"))),
        };
        if let Ok(mut slot) = slot.lock() {
            *slot = Some(reply);
        }
    });
    pending
}

/// The match we hold a seat in.
#[derive(Clone)]
struct Seat {
    match_id: String,
    server_addr: SocketAddr,
    cert_digest: String,
    client_id: u64,
    token: Vec<u8>,
    reconnect_key: String,
}

impl Seat {
    fn from_response(response: JoinResponse) -> Result<Self, String> {
        let token = base64::engine::general_purpose::STANDARD
            .decode(&response.connect_token)
            .map_err(|e| format!("bad token: {e}"))?;
        Ok(Self {
            server_addr: response
                .server_addr
                .parse()
                .map_err(|e| format!("bad address {}: {e}", response.server_addr))?,
            match_id: response.match_id,
            cert_digest: response.cert_digest,
            client_id: response.client_id,
            token,
            reconnect_key: response.reconnect_key,
        })
    }
}

#[derive(Resource, Default)]
pub struct Session {
    seat: Option<Seat>,
    pending: Option<PendingCall>,
}

impl Session {
    fn join(&mut self, args: &ClientArgs) {
        let request = JoinRequest {
            name: args.name.clone(),
            region: None,
        };
        self.pending = Some(post(&args.matchmaker, "/v1/join", &request));
    }

    /// Ask for a fresh token for the same match (tokens are single-use and short-lived). If the
    /// match is gone, or `new_match` is set because this one never let us in, join another one.
    fn refresh(&mut self, args: &ClientArgs, new_match: bool) {
        match &self.seat {
            Some(seat) if !new_match => {
                let request = RejoinRequest {
                    match_id: seat.match_id.clone(),
                    client_id: seat.client_id,
                    reconnect_key: seat.reconnect_key.clone(),
                };
                self.pending = Some(post(&args.matchmaker, "/v1/rejoin", &request));
            }
            _ => self.join(args),
        }
    }

    /// The reply to the call in flight, once it has arrived.
    fn poll(&mut self) -> Option<Result<JoinResponse, CallError>> {
        let reply = self.pending.as_ref()?.lock().ok()?.take()?;
        self.pending = None;
        Some(reply)
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
        app.init_resource::<Session>();
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

fn spawn_client(commands: &mut Commands, seat: &Seat, args: &ClientArgs) -> Result<Entity, String> {
    let conditioner = (args.fake_latency_ms > 0).then(|| {
        RecvLinkConditioner::new(LinkConditionerConfig {
            incoming_latency: Duration::from_millis(args.fake_latency_ms),
            incoming_jitter: Duration::from_millis(args.fake_latency_ms / 10),
            ..default()
        })
    });
    let token =
        ConnectToken::try_from_bytes(&seat.token).map_err(|e| format!("bad token: {e:?}"))?;
    let netcode = NetcodeClient::new(
        Authentication::Token(token),
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
            PeerAddr(seat.server_addr),
            netcode,
            WebTransportClientIo {
                certificate_digest: seat.cert_digest.clone(),
                target: None,
            },
        ))
        .id();
    commands.trigger(Connect { entity });
    info!(
        "connecting to {} ({}) as client {}",
        seat.server_addr, seat.match_id, seat.client_id
    );
    Ok(entity)
}

fn connect(mut session: ResMut<Session>, args: Res<ClientArgs>) {
    session.join(&args);
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
    if let Some(reply) = session.poll() {
        let seat = match reply {
            Ok(response) => Seat::from_response(response),
            Err(CallError::Gone) => {
                warn!("match is gone, joining another one");
                session.seat = None;
                session.join(&args);
                return;
            }
            Err(CallError::Failed(e)) => Err(e),
        };
        match seat.and_then(|seat| {
            let entity = spawn_client(&mut commands, &seat, &args)?;
            session.seat = Some(seat);
            Ok(entity)
        }) {
            Ok(entity) => link.entity = Some(entity),
            Err(e) => warn!("could not get into a match: {e}"),
        }
        link.since_change = 0.0;
        return;
    }
    if session.pending.is_some() {
        return;
    }

    // No live client entity (a previous attempt failed half-way) counts as disconnected.
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
    session.refresh(&args, new_match);
}
