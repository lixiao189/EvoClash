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
use crate::connect_ui::{ConnectionStatus, LinkStatus};

const MAX_RECONNECT_ATTEMPTS: u32 = 12;
/// A matchmaker call that has not answered by now is treated as lost (browser fetch can hang).
const CALL_TIMEOUT_SECS: f32 = 12.0;
/// Hold the link open this long after 离开 so `RoomCommand::Leave` leaves the client.
const DEPART_FLUSH_SECS: f32 = 0.4;

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
    /// Reload `/play/` with no `name` query: the player re-enters their name.
    ChangeRoom,
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
    WaitForPlayer {
        notice: String,
        offers: Vec<JoinOffer>,
    },
    /// 503 `recovering`: wait `secs`, then rejoin the same seat.
    /// Not a failed attempt, and not a new match.
    RecoverRetry { notice: String, secs: f32 },
}

fn offer_join_intent(offer: JoinOffer) -> Option<JoinIntent> {
    match offer {
        JoinOffer::QuickStart => Some(JoinIntent::Quick),
        JoinOffer::CreateRoom => Some(JoinIntent::Create),
        // Not a new request: leave the match and reload the name form.
        JoinOffer::ChangeRoom => None,
    }
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
    // Fast-forward. `retry_after` is seconds; never hammer, and never quick-start away.
    if error == "recovering" && status == 503 {
        return JoinAction::RecoverRetry {
            notice: RECOVERING.to_string(),
            secs: retry_after.unwrap_or(1).max(1) as f32,
        };
    }
    if error == "abandoned" {
        return JoinAction::Choose {
            notice: ABANDONED.to_string(),
            offers: vec![JoinOffer::QuickStart],
        };
    }
    if error == "started" && !started_retried {
        return JoinAction::QuickStartAgain {
            notice: message.to_string(),
        };
    }
    if matches!(error, "in_match" | "results" | "starting") {
        return JoinAction::RetrySame {
            notice: server_text(error, message),
            secs: retry_after.unwrap_or(1) as f32,
            offers: vec![JoinOffer::QuickStart, JoinOffer::ChangeRoom],
        };
    }
    if error == "bad_code" {
        return JoinAction::Choose {
            notice: server_text(error, message),
            offers: vec![JoinOffer::ChangeRoom],
        };
    }
    if error == "not_found" {
        return JoinAction::Choose {
            notice: server_text(error, message),
            offers: vec![
                JoinOffer::QuickStart,
                JoinOffer::CreateRoom,
                JoinOffer::ChangeRoom,
            ],
        };
    }
    if error == "closed" {
        return JoinAction::Choose {
            notice: server_text(error, message),
            offers: vec![JoinOffer::CreateRoom, JoinOffer::ChangeRoom],
        };
    }
    let busy = error == "busy" || status == 503;
    let text = if busy {
        "服务器繁忙，请稍后再试。".to_string()
    } else {
        server_text(error, message)
    };
    // A full room already suggests creating one; everything else can at least go back.
    let offers = if error == "full" {
        vec![JoinOffer::CreateRoom, JoinOffer::ChangeRoom]
    } else {
        vec![JoinOffer::ChangeRoom]
    };
    JoinAction::WaitForPlayer {
        notice: format!("{text}\n点击或按空格重试"),
        offers,
    }
}

/// `counted` means the disconnect loop already bumped `attempts` for this reply.
/// Only that try is refunded: the rejoin timer does not bump, so a later 503 must not either.
fn attempts_after_reply(attempts: u32, counted: bool, action: &JoinAction) -> u32 {
    if counted && matches!(action, JoinAction::RecoverRetry { .. }) {
        attempts.saturating_sub(1)
    } else {
        attempts
    }
}

/// 403, and 410 other than `closed` / `abandoned`, means the seat is gone.
fn error_is_gone(status: u16, error: Option<&str>) -> bool {
    status == 403 || (status == 410 && !matches!(error, Some("closed" | "abandoned")))
}

struct Joined {
    response: JoinResponse,
    spectate: bool,
}

/// `JoinResponse::spectate` is `#[serde(default)]`, so a reply without the field is not spectating.
fn parse_join(bytes: &[u8]) -> Result<Joined, String> {
    serde_json::from_slice::<JoinResponse>(bytes)
        .map(|response| {
            let spectate = response.spectate;
            Joined { response, spectate }
        })
        .map_err(|e| format!("bad reply: {e}"))
}

/// True once the matchmaker seated us as a spectator. Drives the loading copy until
/// the replicated [`evoclash_game::protocol::Spectator`] arrives.
#[derive(Resource, Default)]
pub struct SpectateJoin(pub bool);

/// A matchmaker request in flight. Browsers can't block on HTTP, so the reply lands here and
/// `watch_connection` picks it up on a later frame.
type PendingCall = Arc<Mutex<Option<Result<Joined, CallError>>>>;

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
            Ok(response) if response.ok => {
                parse_join(&response.bytes).map_err(|e| CallError::Failed(format!("{url}: {e}")))
            }
            Ok(response) => match serde_json::from_slice::<ErrorBody>(&response.bytes) {
                // 410 `gone` (and a bare 403/410) joins another match.
                // `closed` and `abandoned` stay here so the player chooses.
                Ok(body) => {
                    if error_is_gone(response.status, Some(body.error.as_str())) {
                        Err(CallError::Gone)
                    } else {
                        Err(CallError::Api(ApiError {
                            status: response.status,
                            error: body.error,
                            message: body.message,
                            retry_after: body.retry_after,
                        }))
                    }
                }
                Err(_) if error_is_gone(response.status, None) => Err(CallError::Gone),
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

/// The player pressed 离开 or chose 返回首页. Hold the link open long enough for
/// `RoomCommand::Leave` to flush, then reload `/play/` (the name form) and do not reconnect.
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
    /// Rejoin the seated match when the timer hits zero. Does not spend an attempt.
    Rejoin(f32),
}

/// Full-screen line while `/v1/rejoin` says the server is still fast-forwarding.
pub(crate) const RECOVERING: &str = "服务器恢复中…";
/// 410 `abandoned`: this match was voided. The player picks 快速开始; we do not requeue.
pub(crate) const ABANDONED: &str = "服务器出错，本局已作废。";

/// True while a rejoin got 503 `recovering`. Cleared when that wait ends any other way.
#[derive(Resource, Default)]
pub struct RecoverHold(pub bool);

#[derive(Resource, Default)]
pub struct Session {
    seat: Option<Seat>,
    pending: Option<PendingCall>,
    /// `Time::elapsed_secs` when the call in flight was sent. `None` when nothing is in flight.
    pending_since: Option<f32>,
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
    fn dispatch(&mut self, args: &ClientArgs, now: f32) {
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
        self.pending_since = Some(now);
    }

    /// Ask for a fresh token for the same match (tokens are single-use and short-lived). If the
    /// match is gone, or `new_match` is set because this one never let us in, join another one.
    fn refresh(&mut self, args: &ClientArgs, new_match: bool, now: f32) {
        match &self.seat {
            Some(seat) if !new_match => {
                let request = RejoinRequest {
                    match_id: seat.match_id.clone(),
                    client_id: seat.client_id,
                    reconnect_key: seat.reconnect_key.clone(),
                };
                self.pending = Some(post(&args.matchmaker, "/v1/rejoin", &request));
                self.pending_since = Some(now);
            }
            _ => self.dispatch(args, now),
        }
    }

    /// The reply to the call in flight, once it has arrived.
    fn poll(&mut self) -> Option<Result<Joined, CallError>> {
        let reply = self.pending.as_ref()?.lock().ok()?.take()?;
        self.pending = None;
        self.pending_since = None;
        Some(reply)
    }
}

#[derive(Resource)]
struct LinkState {
    entity: Option<Entity>,
    since_change: f32,
    attempts: u32,
    /// The in-flight reconnect already incremented `attempts`.
    counted: bool,
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
        app.init_resource::<SpectateJoin>();
        app.init_resource::<RecoverHold>();
        app.insert_resource(LinkState {
            entity: None,
            since_change: 0.0,
            attempts: 0,
            counted: false,
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

fn connect(mut session: ResMut<Session>, args: Res<ClientArgs>, time: Res<Time>) {
    session.dispatch(&args, time.elapsed_secs());
}

fn apply_join_action(
    session: &mut Session,
    notice: &mut JoinNotice,
    offers: &mut JoinOffers,
    recover: &mut RecoverHold,
    action: JoinAction,
    args: &ClientArgs,
    now: f32,
) {
    recover.0 = matches!(action, JoinAction::RecoverRetry { .. });
    match action {
        JoinAction::QuickStartAgain { notice: text } => {
            session.started_retried = true;
            session.intent = JoinIntent::Quick;
            session.wait = NoticeWait::Idle;
            notice.0 = text;
            offers.buttons.clear();
            session.dispatch(args, now);
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
        JoinAction::WaitForPlayer {
            notice: text,
            offers: next,
        } => {
            session.wait = NoticeWait::Click;
            notice.0 = text;
            offers.buttons = next;
        }
        JoinAction::RecoverRetry { notice: text, secs } => {
            session.wait = NoticeWait::Rejoin(secs);
            notice.0 = text;
            offers.buttons.clear();
        }
    }
}

/// A failed matchmaker call still leaves the player a way out.
fn failed_notice(error: &str) -> JoinAction {
    JoinAction::WaitForPlayer {
        notice: format!("连接失败：{error}\n点击或按空格重试"),
        offers: vec![JoinOffer::ChangeRoom],
    }
}

/// The pending call has been unanswered for longer than [`CALL_TIMEOUT_SECS`].
fn call_timed_out(pending_since: Option<f32>, now: f32) -> bool {
    pending_since.is_some_and(|since| now - since >= CALL_TIMEOUT_SECS)
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

fn depart(time: Res<Time>, mut depart: ResMut<Depart>) {
    let Some(since) = depart.since.as_mut() else {
        return;
    };
    *since += time.delta_secs();
    if *since >= DEPART_FLUSH_SECS {
        leave_page();
    }
}

#[cfg(target_family = "wasm")]
fn leave_page() {
    let Some(window) = web_sys::window() else {
        return;
    };
    // No `name` query: `/play/` asks for a name instead of re-queueing immediately.
    let _ = window.location().set_href("/play/");
}

#[cfg(not(target_family = "wasm"))]
fn leave_page() {}

fn watch_connection(
    mut commands: Commands,
    time: Res<Time>,
    mut link: ResMut<LinkState>,
    mut session: ResMut<Session>,
    mut notice: ResMut<JoinNotice>,
    mut depart: ResMut<Depart>,
    mut spectating: ResMut<SpectateJoin>,
    args: Res<ClientArgs>,
    mut offers: ResMut<JoinOffers>,
    mut recover: ResMut<RecoverHold>,
    mut status: ResMut<ConnectionStatus>,
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
) {
    // The overlay reads this; attempts/max must stay accurate even while a request is in flight.
    status.attempts = link.attempts;
    status.max = MAX_RECONNECT_ATTEMPTS;
    if depart.pending() {
        return;
    }
    let now = time.elapsed_secs();
    if call_timed_out(session.pending_since, now) {
        warn!("matchmaker call timed out");
        session.pending = None;
        session.pending_since = None;
        session.started_retried = false;
        link.counted = false;
        status.status = LinkStatus::Connecting;
        if recover.0 {
            // The fast-forward reply never arrived. Same seat again, not a new match.
            session.wait = NoticeWait::Rejoin(1.0);
            notice.0 = RECOVERING.to_string();
            offers.buttons.clear();
            return;
        }
        session.wait = NoticeWait::Click;
        notice.0 = "连接超时，请重试。\n点击或按空格重试".to_string();
        offers.buttons = vec![JoinOffer::QuickStart, JoinOffer::ChangeRoom];
        return;
    }
    if let Some(reply) = session.poll() {
        let seat = match reply {
            Ok(joined) => {
                if session.creating(&args) {
                    session.create_used = true;
                }
                session.intent = JoinIntent::FromArgs;
                recover.0 = false;
                link.counted = false;
                notice.0.clear();
                offers.buttons.clear();
                session.wait = NoticeWait::Idle;
                session.started_retried = false;
                spectating.0 = joined.spectate;
                Seat::from_response(joined.response)
            }
            Err(CallError::Gone) => {
                warn!("match is gone, joining another one");
                session.seat = None;
                spectating.0 = false;
                recover.0 = false;
                link.counted = false;
                session.wait = NoticeWait::Idle;
                offers.buttons.clear();
                session.dispatch(&args, now);
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
                link.attempts = attempts_after_reply(link.attempts, link.counted, &action);
                link.counted = false;
                status.attempts = link.attempts;
                if err.error == "abandoned" {
                    // Do not keep rejoining a voided match after 快速开始.
                    session.seat = None;
                    spectating.0 = false;
                }
                apply_join_action(
                    &mut session,
                    &mut notice,
                    &mut offers,
                    &mut recover,
                    action,
                    &args,
                    now,
                );
                // Buttons (or the recover shade) must not sit under the reconnect card.
                status.status = LinkStatus::Connecting;
                link.since_change = 0.0;
                return;
            }
            Err(CallError::Failed(e)) => {
                // The try already counted. Don't let a later 503 refund it.
                link.counted = false;
                Err(e)
            }
        };
        let failed = seat.and_then(|seat| {
            let entity = spawn_client(&mut commands, &seat, &args)?;
            session.seat = Some(seat);
            Ok(entity)
        });
        match failed {
            Ok(entity) => link.entity = Some(entity),
            Err(e) => {
                warn!("could not get into a match: {e}");
                apply_join_action(
                    &mut session,
                    &mut notice,
                    &mut offers,
                    &mut recover,
                    failed_notice(&e),
                    &args,
                    now,
                );
                status.status = LinkStatus::Connecting;
            }
        }
        link.since_change = 0.0;
        return;
    }
    if session.pending.is_some() {
        if status.status == LinkStatus::Idle {
            status.status = LinkStatus::Connecting;
        }
        return;
    }
    // Offer buttons (快速开始 / 创建房间 / 返回首页) cancel a wait without touching the
    // reconnect budget.
    if let Some(offer) = take_join_offer(&offers, &mouse, &offer_buttons) {
        recover.0 = false;
        link.counted = false;
        match offer_join_intent(offer) {
            Some(intent) => {
                session.intent = intent;
                session.wait = NoticeWait::Idle;
                session.started_retried = false;
                notice.0.clear();
                offers.buttons.clear();
                session.dispatch(&args, now);
            }
            None => {
                // 返回首页: flush `RoomCommand::Leave`, then reload the name form.
                session.wait = NoticeWait::Idle;
                session.started_retried = false;
                notice.0.clear();
                offers.buttons.clear();
                depart.request();
            }
        }
        return;
    }
    match session.wait {
        NoticeWait::Retry(left) => {
            // Same request again. Not a reconnect attempt.
            let left = left - time.delta_secs();
            if left <= 0.0 {
                session.wait = NoticeWait::Idle;
                offers.buttons.clear();
                session.dispatch(&args, now);
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
                session.dispatch(&args, now);
            }
            return;
        }
        NoticeWait::Hold => return,
        NoticeWait::Rejoin(left) => {
            // Same seat again. The disconnect loop must not count this.
            let left = left - time.delta_secs();
            if left <= 0.0 {
                session.wait = NoticeWait::Idle;
                offers.buttons.clear();
                session.refresh(&args, false, now);
            } else {
                session.wait = NoticeWait::Rejoin(left);
            }
            return;
        }
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
        link.counted = false;
        link.since_change = 0.0;
        link.ever_connected = true;
        session.started_retried = false;
        session.wait = NoticeWait::Idle;
        session.intent = JoinIntent::FromArgs;
        offers.buttons.clear();
        recover.0 = false;
        notice.0.clear();
        status.status = LinkStatus::Connected;
        status.attempts = 0;
        return;
    }
    if disconnected {
        status.status = LinkStatus::Reconnecting;
        // Show the attempt that is about to start, not the number that already failed.
        status.attempts = (link.attempts + 1).min(MAX_RECONNECT_ATTEMPTS);
    }
    link.since_change += time.delta_secs();
    // Give each attempt time to complete the handshake before retrying, backing off a little.
    let wait = 1.0 + link.attempts.min(4) as f32;
    if !disconnected || link.since_change < wait {
        return;
    }
    if link.attempts >= MAX_RECONNECT_ATTEMPTS {
        // Keep the app alive: the reconnect overlay offers 返回首页 instead of a hard exit.
        if status.status != LinkStatus::Failed {
            error!("giving up after {} reconnect attempts", link.attempts);
        }
        status.status = LinkStatus::Failed;
        status.attempts = link.attempts;
        return;
    }
    link.attempts += 1;
    link.counted = true;
    link.since_change = 0.0;
    recover.0 = false;
    status.status = LinkStatus::Reconnecting;
    status.attempts = link.attempts;
    warn!("disconnected, reconnecting (attempt {})", link.attempts);

    // Start from a clean slate; the server resends the full world after the handshake.
    if let Some(entity) = link.entity.take() {
        commands.entity(entity).despawn();
    }
    for e in &replicated {
        commands.entity(e).despawn();
    }
    let new_match = !link.ever_connected && link.attempts > NEW_MATCH_AFTER_ATTEMPTS;
    session.refresh(&args, new_match, now);
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
                offers: vec![JoinOffer::ChangeRoom],
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
                offers: vec![JoinOffer::QuickStart, JoinOffer::ChangeRoom],
            }
        );
        let playing = classify_join_error("in_match", 409, "对局中", None, false);
        assert_eq!(
            playing,
            JoinAction::RetrySame {
                notice: "对局中".into(),
                secs: 1.0,
                offers: vec![JoinOffer::QuickStart, JoinOffer::ChangeRoom],
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
                offers: vec![JoinOffer::QuickStart, JoinOffer::ChangeRoom],
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
                offers: vec![JoinOffer::ChangeRoom],
            }
        );
        let missing = classify_join_error("not_found", 404, "找不到房间 AB3K", None, false);
        assert_eq!(
            missing,
            JoinAction::Choose {
                notice: "找不到房间 AB3K".into(),
                offers: vec![
                    JoinOffer::QuickStart,
                    JoinOffer::CreateRoom,
                    JoinOffer::ChangeRoom,
                ],
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
                offers: vec![JoinOffer::CreateRoom, JoinOffer::ChangeRoom],
            }
        );
    }

    #[test]
    fn full_and_rate_wait_for_a_click_with_the_server_message() {
        let full = classify_join_error(
            "full",
            409,
            "房间已满（8/8）。请稍后再试，或创建自己的房间。",
            None,
            false,
        );
        assert_eq!(
            full,
            JoinAction::WaitForPlayer {
                notice: "房间已满（8/8）。请稍后再试，或创建自己的房间。\n点击或按空格重试".into(),
                offers: vec![JoinOffer::CreateRoom, JoinOffer::ChangeRoom],
            }
        );
        let rate = classify_join_error("rate", 409, "尝试次数过多，请一分钟后再试。", None, false);
        assert_eq!(
            rate,
            JoinAction::WaitForPlayer {
                notice: "尝试次数过多，请一分钟后再试。\n点击或按空格重试".into(),
                offers: vec![JoinOffer::ChangeRoom],
            }
        );
    }

    #[test]
    fn busy_and_503_ask_the_player_to_retry() {
        for (error, status) in [("busy", 503), ("busy", 409), ("other", 503)] {
            let action = classify_join_error(error, status, "ignored", None, false);
            assert_eq!(
                action,
                JoinAction::WaitForPlayer {
                    notice: "服务器繁忙，请稍后再试。\n点击或按空格重试".into(),
                    offers: vec![JoinOffer::ChangeRoom],
                }
            );
        }
    }

    #[test]
    fn every_join_failure_offers_a_next_step() {
        for (error, status) in [
            ("bad_code", 400),
            ("not_found", 404),
            ("closed", 410),
            ("full", 409),
            ("rate", 409),
            ("busy", 503),
            ("started", 409),
            ("in_match", 409),
            ("results", 409),
            ("other", 500),
        ] {
            let action = classify_join_error(error, status, "ignored", None, true);
            let offers = match action {
                JoinAction::QuickStartAgain { .. } => Vec::new(),
                JoinAction::RetrySame { offers, .. } => offers,
                JoinAction::Choose { offers, .. } => offers,
                JoinAction::WaitForPlayer { offers, .. } => offers,
                JoinAction::RecoverRetry { .. } => Vec::new(),
            };
            assert!(!offers.is_empty(), "{error} left no way out");
            assert!(
                offers.contains(&JoinOffer::ChangeRoom),
                "{error} cannot go back to the name form"
            );
        }
    }

    #[test]
    fn change_room_returns_to_the_name_form() {
        assert_eq!(offer_join_intent(JoinOffer::ChangeRoom), None);
        assert_eq!(
            offer_join_intent(JoinOffer::QuickStart),
            Some(JoinIntent::Quick)
        );
        assert_eq!(
            offer_join_intent(JoinOffer::CreateRoom),
            Some(JoinIntent::Create)
        );
        assert_eq!(
            failed_notice("the network is down"),
            JoinAction::WaitForPlayer {
                notice: "连接失败：the network is down\n点击或按空格重试".into(),
                offers: vec![JoinOffer::ChangeRoom],
            }
        );
    }

    #[test]
    fn matchmaker_calls_time_out_after_twelve_seconds() {
        assert!(!call_timed_out(None, 100.0));
        assert!(!call_timed_out(Some(10.0), 21.9));
        assert!(call_timed_out(Some(10.0), 22.0));
    }

    #[test]
    fn missing_spectate_field_defaults_to_false() {
        let bare = br#"{"match_id":"m","server_addr":"127.0.0.1:9","cert_digest":"ab","client_id":3,"connect_token":"aa","reconnect_key":"k"}"#;
        let joined = parse_join(bare).unwrap();
        assert!(!joined.spectate);
        assert_eq!(joined.response.match_id, "m");
        assert_eq!(joined.response.client_id, 3);
        assert!(joined.response.room_code.is_none());

        let flagged = br#"{"match_id":"m","server_addr":"127.0.0.1:9","cert_digest":"ab","client_id":7,"connect_token":"aa","reconnect_key":"k","room_code":"AB12","spectate":true}"#;
        let joined = parse_join(flagged).unwrap();
        assert!(joined.spectate);
        assert_eq!(joined.response.client_id, 7);
        assert_eq!(joined.response.room_code.as_deref(), Some("AB12"));

        let explicit = br#"{"match_id":"m","server_addr":"127.0.0.1:9","cert_digest":"ab","client_id":1,"connect_token":"aa","reconnect_key":"k","spectate":false}"#;
        assert!(!parse_join(explicit).unwrap().spectate);
    }

    #[test]
    fn recovering_retries_after_retry_after_without_spending_an_attempt() {
        let action = classify_join_error("recovering", 503, "ignored", Some(5), false);
        assert_eq!(
            action,
            JoinAction::RecoverRetry {
                notice: RECOVERING.to_string(),
                secs: 5.0,
            }
        );
        // The disconnect loop already counted this try. Fast-forward gives it back.
        let mut attempts = 4;
        let mut counted = true;
        attempts += 1;
        attempts = attempts_after_reply(attempts, counted, &action);
        counted = false;
        assert_eq!(attempts, 4);
        // The rejoin timer does not count, so the next 503 must not refund a real failure.
        attempts = attempts_after_reply(attempts, counted, &action);
        assert_eq!(attempts, 4);
        assert_eq!(attempts_after_reply(0, true, &action), 0);

        let missing = classify_join_error("recovering", 503, "", None, true);
        assert_eq!(
            missing,
            JoinAction::RecoverRetry {
                notice: RECOVERING.to_string(),
                secs: 1.0,
            }
        );
        // A plain 503 is still a busy failure and still spends the try.
        let busy = classify_join_error("busy", 503, "ignored", Some(5), false);
        assert_eq!(attempts_after_reply(4, true, &busy), 4);
    }

    #[test]
    fn abandoned_stays_put_while_gone_and_closed_keep_their_old_paths() {
        // `true` is CallError::Gone: watch_connection dispatches a new join.
        assert!(error_is_gone(410, Some("gone")));
        assert!(error_is_gone(410, None));
        assert!(error_is_gone(403, Some("gone")));
        assert!(!error_is_gone(410, Some("closed")));
        assert!(!error_is_gone(410, Some("abandoned")));

        let abandoned = classify_join_error("abandoned", 410, "ignored", None, false);
        assert_eq!(
            abandoned,
            JoinAction::Choose {
                notice: ABANDONED.to_string(),
                offers: vec![JoinOffer::QuickStart],
            }
        );
        // Not an automatic requeue, and not a refunded attempt.
        assert_eq!(attempts_after_reply(3, true, &abandoned), 3);

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
                offers: vec![JoinOffer::CreateRoom, JoinOffer::ChangeRoom],
            }
        );
    }
}
