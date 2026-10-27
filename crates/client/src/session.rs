//! Getting into a match and staying there: matchmaker join, netcode connect, auto-reconnect.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use evoclash_game::api::{CreateRoomRequest, JoinRequest, JoinResponse, RejoinRequest};
use evoclash_game::net::{CONNECTION_TIMEOUT_SECS, TOKEN_EXPIRE_SECS};
use evoclash_game::protocol::{MatchState, Monster, Pickup, Player, Projectile};
use lightyear::connection::client::{Connected, Disconnected};
use lightyear::link::prelude::{LinkConditionerConfig, RecvLinkConditioner};
use lightyear::netcode::client_plugin::NetcodeConfig;
use lightyear::netcode::{ConnectToken, NetcodeClient};
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use lightyear::webtransport::prelude::client::WebTransportClientIo;
use serde::{Deserialize, Serialize};

use crate::ClientArgs;
use crate::assets::assets_ready;

const MAX_RECONNECT_ATTEMPTS: u32 = 12;

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    retry_after: Option<u32>,
}

struct ApiError {
    status: u16,
    error: String,
    message: String,
    retry_after: Option<u32>,
}

enum CallError {
    /// The matchmaker answered that the match is over or unknown: find a new one.
    Gone,
    Api(ApiError),
    Failed(String),
}

/// Buttons under a join failure. The session applies one when it is clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinOffer {
    QuickStart,
    CreateRoom,
}

#[derive(Component, Clone, Copy, Debug)]
#[require(RelativeCursorPosition)]
pub struct JoinOfferButton(pub JoinOffer);

/// Which failure buttons are on screen. Empty hides them.
#[derive(Resource, Default, Clone, Debug, PartialEq, Eq)]
pub struct JoinOffers {
    pub buttons: Vec<JoinOffer>,
}

#[derive(Debug, PartialEq)]
enum JoinAction {
    /// `started` the first time: quick-start again immediately.
    QuickStartAgain { notice: String },
    /// `in_match` / `results` / `starting`: wait, then repeat this request.
    /// `offers` escapes the wait (快速开始).
    RetrySame {
        notice: String,
        secs: f32,
        offers: Vec<JoinOffer>,
    },
    /// No timer. Only `offers` do anything; an empty list just stays.
    Choose {
        notice: String,
        offers: Vec<JoinOffer>,
    },
    /// Show `notice` and wait for a click or Space. Does not loop.
    WaitForPlayer { notice: String },
}

fn server_text(error: &str, message: &str) -> String {
    if message.is_empty() {
        format!("无法加入（{error}）")
    } else {
        message.to_string()
    }
}

fn classify_join_error(
    error: &str,
    status: u16,
    message: &str,
    retry_after: Option<u32>,
    started_retried: bool,
) -> JoinAction {
    if error == "started" && !started_retried {
        return JoinAction::QuickStartAgain {
            notice: message.to_string(),
        };
    }
    if matches!(error, "in_match" | "results" | "starting") {
        return JoinAction::RetrySame {
            notice: server_text(error, message),
            secs: retry_after.unwrap_or(1) as f32,
            offers: vec![JoinOffer::QuickStart],
        };
    }
    if error == "bad_code" {
        return JoinAction::Choose {
            notice: server_text(error, message),
            offers: Vec::new(),
        };
    }
    if error == "not_found" {
        return JoinAction::Choose {
            notice: server_text(error, message),
            offers: vec![JoinOffer::QuickStart, JoinOffer::CreateRoom],
        };
    }
    if error == "closed" {
        return JoinAction::Choose {
            notice: server_text(error, message),
            offers: vec![JoinOffer::CreateRoom],
        };
    }
    let busy = error == "busy" || status == 503;
    let text = if busy {
        "服务器繁忙，请稍后再试。".to_string()
    } else {
        server_text(error, message)
    };
    JoinAction::WaitForPlayer {
        notice: format!("{text}\n点击或按空格重试"),
    }
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
            Ok(response) => match serde_json::from_slice::<ErrorBody>(&response.bytes) {
                // Rejoin's 410 `gone` (and a bare 403/410) finds another match. A closed
                // room is 410 too, but the player has to choose, not be redirected.
                Ok(body)
                    if response.status == 403
                        || (response.status == 410 && body.error != "closed") =>
                {
                    Err(CallError::Gone)
                }
                Ok(body) => Err(CallError::Api(ApiError {
                    status: response.status,
                    error: body.error,
                    message: body.message,
                    retry_after: body.retry_after,
                })),
                Err(_) if matches!(response.status, 403 | 410) => Err(CallError::Gone),
                Err(_) => Err(CallError::Failed(format!(
                    "{url}: {} {}",
                    response.status,
                    String::from_utf8_lossy(&response.bytes)
                ))),
            },
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

/// Join failure copy for the HUD. Empty while there is nothing to say.
#[derive(Resource, Default)]
pub struct JoinNotice(pub String);

/// The player pressed 离开. Hold the link open long enough for `RoomCommand::Leave` to
/// flush, then go back to the name page and do not reconnect.
#[derive(Resource, Default)]
pub struct Depart {
    since: Option<f32>,
}

impl Depart {
    pub fn request(&mut self) {
        if self.since.is_none() {
            self.since = Some(0.0);
        }
    }

    fn pending(&self) -> bool {
        self.since.is_some()
    }
}

/// What the next fresh join does. Page query until a failure button overrides it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum JoinIntent {
    #[default]
    FromArgs,
    /// `/v1/join` with no room code.
    Quick,
    /// `/v1/rooms`.
    Create,
}

/// Why the connection loop is paused. These waits are not reconnect attempts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum NoticeWait {
    #[default]
    Idle,
    /// Repeat the same request when the timer hits zero.
    Retry(f32),
    /// Click or Space repeats the same request.
    Click,
    /// The notice stays. Only an offer button moves on.
    Hold,
}

#[derive(Resource, Default)]
pub struct Session {
    seat: Option<Seat>,
    pending: Option<PendingCall>,
    /// Already quick-started once after `started` in this attempt.
    started_retried: bool,
    intent: JoinIntent,
    wait: NoticeWait,
    /// `?create=1` already minted a room. Don't mint another on a later fallback.
    create_used: bool,
}

impl Session {
    fn creating(&self, args: &ClientArgs) -> bool {
        match self.intent {
            JoinIntent::Create => true,
            JoinIntent::Quick => false,
            JoinIntent::FromArgs => args.create_room && !self.create_used,
        }
    }

    fn room_code(&self, args: &ClientArgs) -> Option<String> {
        match self.intent {
            JoinIntent::Quick | JoinIntent::Create => None,
            JoinIntent::FromArgs => args.room.clone(),
        }
    }

    /// Create a room, or join (quick start when `room` is absent).
    fn dispatch(&mut self, args: &ClientArgs) {
        if self.creating(args) {
            let request = CreateRoomRequest {
                name: args.name.clone(),
            };
            self.pending = Some(post(&args.matchmaker, "/v1/rooms", &request));
        } else {
            let request = JoinRequest {
                name: args.name.clone(),
                region: None,
                match_id: None,
                room: self.room_code(args),
            };
            self.pending = Some(post(&args.matchmaker, "/v1/join", &request));
        }
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
            _ => self.dispatch(args),
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
        app.init_resource::<JoinNotice>();
        app.init_resource::<JoinOffers>();
        app.init_resource::<Depart>();
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
        // Join only once the files the match needs have loaded.
        app.add_systems(
            Update,
            (connect.run_if(run_once), watch_connection, depart)
                .chain()
                .run_if(assets_ready),
        );
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
    session.dispatch(&args);
}

fn apply_join_action(
    session: &mut Session,
    notice: &mut JoinNotice,
    offers: &mut JoinOffers,
    action: JoinAction,
    args: &ClientArgs,
) {
    match action {
        JoinAction::QuickStartAgain { notice: text } => {
            session.started_retried = true;
            session.intent = JoinIntent::Quick;
            session.wait = NoticeWait::Idle;
            notice.0 = text;
            offers.buttons.clear();
            session.dispatch(args);
        }
        JoinAction::RetrySame {
            notice: text,
            secs,
            offers: next,
        } => {
            session.wait = NoticeWait::Retry(secs);
            notice.0 = text;
            offers.buttons = next;
        }
        JoinAction::Choose {
            notice: text,
            offers: next,
        } => {
            session.wait = NoticeWait::Hold;
            notice.0 = text;
            offers.buttons = next;
        }
        JoinAction::WaitForPlayer { notice: text } => {
            session.wait = NoticeWait::Click;
            notice.0 = text;
            offers.buttons.clear();
        }
    }
}

fn take_join_offer(
    offers: &JoinOffers,
    mouse: &Option<Res<ButtonInput<MouseButton>>>,
    buttons: &Query<(
        &JoinOfferButton,
        &RelativeCursorPosition,
        &InheritedVisibility,
    )>,
) -> Option<JoinOffer> {
    if !mouse
        .as_ref()
        .is_some_and(|m| m.just_pressed(MouseButton::Left))
    {
        return None;
    }
    buttons.iter().find_map(|(button, cursor, vis)| {
        (offers.buttons.contains(&button.0) && vis.get() && cursor.cursor_over())
            .then_some(button.0)
    })
}

fn depart(time: Res<Time>, mut depart: ResMut<Depart>, args: Res<ClientArgs>) {
    let Some(since) = depart.since.as_mut() else {
        return;
    };
    *since += time.delta_secs();
    if *since >= 0.4 {
        leave_page(&args.name);
    }
}

#[cfg(target_family = "wasm")]
fn leave_page(name: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let _ = window
        .location()
        .set_href(&format!("/play/?name={}", query_escape(name)));
}

#[cfg(not(target_family = "wasm"))]
fn leave_page(_name: &str) {}

#[cfg(target_family = "wasm")]
fn query_escape(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn watch_connection(
    mut commands: Commands,
    time: Res<Time>,
    mut link: ResMut<LinkState>,
    mut session: ResMut<Session>,
    mut notice: ResMut<JoinNotice>,
    depart: Res<Depart>,
    args: Res<ClientArgs>,
    mut offers: ResMut<JoinOffers>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    offer_buttons: Query<(
        &JoinOfferButton,
        &RelativeCursorPosition,
        &InheritedVisibility,
    )>,
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
    if depart.pending() {
        return;
    }
    if let Some(reply) = session.poll() {
        let seat = match reply {
            Ok(response) => {
                if session.creating(&args) {
                    session.create_used = true;
                }
                session.intent = JoinIntent::FromArgs;
                notice.0.clear();
                offers.buttons.clear();
                session.wait = NoticeWait::Idle;
                session.started_retried = false;
                Seat::from_response(response)
            }
            Err(CallError::Gone) => {
                warn!("match is gone, joining another one");
                session.seat = None;
                session.wait = NoticeWait::Idle;
                offers.buttons.clear();
                session.dispatch(&args);
                return;
            }
            Err(CallError::Api(err)) => {
                let action = classify_join_error(
                    &err.error,
                    err.status,
                    &err.message,
                    err.retry_after,
                    session.started_retried,
                );
                apply_join_action(&mut session, &mut notice, &mut offers, action, &args);
                link.since_change = 0.0;
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
    // Offer buttons (快速开始 / 创建房间) cancel a wait without touching the reconnect budget.
    if let Some(offer) = take_join_offer(&offers, &mouse, &offer_buttons) {
        session.intent = match offer {
            JoinOffer::QuickStart => JoinIntent::Quick,
            JoinOffer::CreateRoom => JoinIntent::Create,
        };
        session.wait = NoticeWait::Idle;
        session.started_retried = false;
        notice.0.clear();
        offers.buttons.clear();
        session.dispatch(&args);
        return;
    }
    match session.wait {
        NoticeWait::Retry(left) => {
            // Same request again. Not a reconnect attempt.
            let left = left - time.delta_secs();
            if left <= 0.0 {
                session.wait = NoticeWait::Idle;
                offers.buttons.clear();
                session.dispatch(&args);
            } else {
                session.wait = NoticeWait::Retry(left);
            }
            return;
        }
        NoticeWait::Click => {
            let key = keys.is_some_and(|keys| {
                keys.just_pressed(KeyCode::Space) || keys.just_pressed(KeyCode::Enter)
            });
            let click = mouse.is_some_and(|mouse| mouse.just_pressed(MouseButton::Left));
            if key || click {
                session.wait = NoticeWait::Idle;
                session.started_retried = false;
                notice.0.clear();
                session.dispatch(&args);
            }
            return;
        }
        NoticeWait::Hold => return,
        NoticeWait::Idle => {}
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
        session.started_retried = false;
        session.wait = NoticeWait::Idle;
        session.intent = JoinIntent::FromArgs;
        offers.buttons.clear();
        notice.0.clear();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn started_quick_starts_once_then_stops() {
        let again = classify_join_error("started", 409, "这局已经开始", None, false);
        assert_eq!(
            again,
            JoinAction::QuickStartAgain {
                notice: "这局已经开始".into(),
            }
        );
        let stop = classify_join_error("started", 409, "这局已经开始", None, true);
        assert_eq!(
            stop,
            JoinAction::WaitForPlayer {
                notice: "这局已经开始\n点击或按空格重试".into(),
            }
        );
    }

    #[test]
    fn in_match_results_and_starting_retry_the_same_request() {
        let starting =
            classify_join_error("starting", 409, "房间正在开局，请稍候。", Some(2), false);
        assert_eq!(
            starting,
            JoinAction::RetrySame {
                notice: "房间正在开局，请稍候。".into(),
                secs: 2.0,
                offers: vec![JoinOffer::QuickStart],
            }
        );
        let playing = classify_join_error("in_match", 409, "对局中", None, false);
        assert_eq!(
            playing,
            JoinAction::RetrySame {
                notice: "对局中".into(),
                secs: 1.0,
                offers: vec![JoinOffer::QuickStart],
            }
        );
        let results = classify_join_error(
            "results",
            409,
            "本局刚结束，马上回到等待大厅…",
            Some(5),
            true,
        );
        assert_eq!(
            results,
            JoinAction::RetrySame {
                notice: "本局刚结束，马上回到等待大厅…".into(),
                secs: 5.0,
                offers: vec![JoinOffer::QuickStart],
            }
        );
    }

    #[test]
    fn bad_code_stays_and_missing_rooms_offer_a_way_out() {
        let bad = classify_join_error("bad_code", 400, "房间码是 4 位字母或数字。", None, false);
        assert_eq!(
            bad,
            JoinAction::Choose {
                notice: "房间码是 4 位字母或数字。".into(),
                offers: vec![],
            }
        );
        let missing = classify_join_error("not_found", 404, "找不到房间 AB3K", None, false);
        assert_eq!(
            missing,
            JoinAction::Choose {
                notice: "找不到房间 AB3K".into(),
                offers: vec![JoinOffer::QuickStart, JoinOffer::CreateRoom],
            }
        );
        let closed = classify_join_error(
            "closed",
            410,
            "房间已关闭。你可以创建新的私人房间。",
            None,
            false,
        );
        assert_eq!(
            closed,
            JoinAction::Choose {
                notice: "房间已关闭。你可以创建新的私人房间。".into(),
                offers: vec![JoinOffer::CreateRoom],
            }
        );
    }

    #[test]
    fn full_and_rate_wait_for_a_click_with_the_server_message() {
        for (error, message) in [
            ("full", "房间已满（8/8）。请稍后再试，或创建自己的房间。"),
            ("rate", "尝试次数过多，请一分钟后再试。"),
        ] {
            let action = classify_join_error(error, 409, message, None, false);
            assert_eq!(
                action,
                JoinAction::WaitForPlayer {
                    notice: format!("{message}\n点击或按空格重试"),
                }
            );
        }
    }

    #[test]
    fn busy_and_503_ask_the_player_to_retry() {
        for (error, status) in [("busy", 503), ("busy", 409), ("other", 503)] {
            let action = classify_join_error(error, status, "ignored", None, false);
            assert_eq!(
                action,
                JoinAction::WaitForPlayer {
                    notice: "服务器繁忙，请稍后再试。\n点击或按空格重试".into(),
                }
            );
        }
    }
}
