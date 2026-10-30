//! Transport setup and the player session lifecycle: join, drop, reconnect.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Instant;

use bevy::prelude::*;
use evoclash_game::evolution::Evolution;
use evoclash_game::net::{self, CONNECTION_TIMEOUT_SECS, PROTOCOL_ID};
use evoclash_game::protocol::{
    Departed, Departure, MAX_SPECTATORS, MatchState, Phase, Player, Stats,
};
use lightyear::connection::client::Connected;
use lightyear::netcode::{NetcodeServer, TokenUserData};
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use lightyear::webtransport::prelude::Identity;
use lightyear::webtransport::prelude::endpoint::WebTransportEndpoint;

use crate::config::ServerConfig;
use crate::gameplay::{Outbox, SimSet, SimTick, spawn_combatant};
use crate::journal::{CatchingUp, Trace};
use crate::ops::OpsState;
use crate::spectate::{self, Admit, NewSpectator, SpectateSeq, SpectatorSession, first_id};

/// A disconnected player's body stays in the arena this long so a flaky
/// connection can resume with its gear intact.
pub const RECONNECT_GRACE_TICKS: u32 = 64 * 20;

/// Names of humans whose reconnect grace ran out. A rejoin after that is a new body (the seat is
/// gone) but the same player, so it gets its name back instead of a fresh `幸存者N`.
#[derive(Resource, Default)]
pub struct ClearedSeats {
    names: std::collections::HashMap<u64, (String, u32)>,
}

impl ClearedSeats {
    /// Keep a cleared name this long (10 minutes at 64 Hz) at most.
    const KEEP_TICKS: u32 = 64 * 600;

    fn remember(&mut self, client_id: u64, name: &str, tick: u32) {
        self.names.insert(client_id, (name.to_string(), tick));
    }

    /// The name of a cleared seat, if it was cleared recently. One-shot: the reconnect that
    /// takes it spawns a body, and later drops go through that body's own `Human`.
    fn take(&mut self, client_id: u64, tick: u32) -> Option<String> {
        let (name, cleared) = self.names.remove(&client_id)?;
        (tick.saturating_sub(cleared) <= Self::KEEP_TICKS).then_some(name)
    }

    fn prune(&mut self, tick: u32) {
        self.names
            .retain(|_, (_, cleared)| tick.saturating_sub(*cleared) <= Self::KEEP_TICKS);
    }
}

/// Server-side marker for human-controlled combatants.
#[derive(Component)]
pub struct Human {
    pub peer: PeerId,
    pub disconnected_ticks: u32,
    /// Tick they first connected. A reconnect keeps it, so host handoff can pick the oldest.
    pub joined_at: u32,
    /// Leave: drop now, don't wait out the reconnect grace.
    pub left: bool,
}

impl Human {
    pub fn connected(&self) -> bool {
        self.disconnected_ticks == 0
    }
}

/// TLS identity of the WebTransport endpoint. Browsers accept a self-signed certificate only
/// when told its SHA-256 hash up front (the matchmaker passes it on from our heartbeats), and
/// only if it is valid for at most 14 days, so the server retires itself before it expires.
#[derive(Resource)]
pub struct TlsIdentity {
    identity: Identity,
    /// Lowercase hex SHA-256 of the certificate.
    pub digest: String,
    pub created: Instant,
}

impl TlsIdentity {
    pub fn self_signed(config: &ServerConfig) -> Self {
        let public_ip = config.public_addr.ip().to_string();
        let identity = Identity::self_signed(["localhost", "127.0.0.1", "::1", &public_ip])
            .expect("valid subject alt names");
        let digest = identity.certificate_chain().as_slice()[0]
            .hash()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Self {
            identity,
            digest,
            created: Instant::now(),
        }
    }
}

pub struct ConnectionsPlugin;

impl Plugin for ConnectionsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start_server);
        app.add_observer(on_new_link);
        app.add_observer(on_connected);
        app.add_observer(hide_orphaned_control);
        app.add_systems(FixedUpdate, track_presence.in_set(SimSet::Input));
        app.init_resource::<ClearedSeats>();
        #[cfg(debug_assertions)]
        {
            app.init_resource::<FaultDrops>();
            app.add_systems(Update, drop_links_on_timer);
        }
    }
}

/// `--fault-drop-every`: cut one human link per interval, so the drop, grace and rejoin path can
/// be driven from `scripts/fault-inject` without a browser. Debug builds only.
#[cfg(debug_assertions)]
#[derive(Resource, Default)]
struct FaultDrops {
    left: f32,
    next: usize,
}

#[cfg(debug_assertions)]
fn drop_links_on_timer(
    time: Res<Time>,
    mut state: ResMut<FaultDrops>,
    humans: Query<(&Human, &Player, Option<&ControlledBy>)>,
    mut commands: Commands,
) {
    let Some(every) = crate::config::fault_drop_every() else {
        return;
    };
    state.left -= time.delta_secs();
    if state.left > 0.0 {
        return;
    }
    state.left = every.as_secs_f32();
    let mut links: Vec<(Entity, String)> = humans
        .iter()
        .filter(|(human, _, _)| !human.left)
        .filter_map(|(_, player, control)| Some((control?.owner, player.name.clone())))
        .collect();
    links.sort_by(|a, b| a.1.cmp(&b.1));
    let Some((link, name)) = links.get(state.next % links.len().max(1)) else {
        return;
    };
    state.next = state.next.wrapping_add(1);
    info!("fault injection: dropping the link of {name}");
    commands.trigger(Disconnect { entity: *link });
}

fn start_server(mut commands: Commands, config: Res<ServerConfig>, tls: Res<TlsIdentity>) {
    let (private_key, dev_key) = net::private_key_from_env();
    if dev_key {
        warn!(
            "{} not set: using the insecure development key",
            net::KEY_ENV
        );
    }
    let bind = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), config.port);
    let netcode = NetcodeConfig {
        protocol_id: PROTOCOL_ID,
        private_key,
        client_timeout_secs: CONNECTION_TIMEOUT_SECS,
        // Tokens name the public address, which differs from the 0.0.0.0 bind address.
        additional_expected_addresses: vec![config.public_addr],
        ..default()
    }
    .with_max_clients(config.max_players.saturating_add(MAX_SPECTATORS as usize));
    let server = commands
        .spawn((
            Name::new("Server"),
            NetcodeServer::new(netcode),
            LocalAddr(bind),
            WebTransportEndpoint {
                certificate: tls.identity.clone_identity(),
            },
        ))
        .id();
    commands.trigger(Start { entity: server });
    info!(
        "match {} listening on WebTransport https://{bind} (public {}), max {} combatants + {} spectators, cert {}",
        config.match_id, config.public_addr, config.max_players, MAX_SPECTATORS, tls.digest
    );
}

fn on_new_link(trigger: On<Add<LinkOf>>, mut commands: Commands) {
    commands
        .entity(trigger.entity)
        .insert((ReplicationSender, Name::new("ClientLink")));
}

/// Display name travels inside the matchmaker-signed connect token, so it can't be spoofed later.
fn name_from_token(user_data: Option<&TokenUserData>, client_id: u64) -> String {
    user_data
        .and_then(|data| {
            let end = data.0.iter().position(|b| *b == 0).unwrap_or(data.0.len());
            std::str::from_utf8(&data.0[..end]).ok().map(str::to_owned)
        })
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("幸存者{}", client_id % 1000))
}

fn on_connected(
    trigger: On<Add<Connected>>,
    links: Query<(&RemoteId, Option<&TokenUserData>), With<ClientOf>>,
    mut humans: Query<(Entity, &mut Human, &Player)>,
    mut specs: Query<(
        Entity,
        &mut SpectatorSession,
        &mut evoclash_game::protocol::Spectator,
    )>,
    players: Query<(&Player, &Stats)>,
    state: Query<&MatchState>,
    config: Res<ServerConfig>,
    ops: Res<OpsState>,
    tick: Option<Res<SimTick>>,
    mut cleared: ResMut<ClearedSeats>,
    mut seq: ResMut<SpectateSeq>,
    mut outbox: ResMut<Outbox>,
    mut commands: Commands,
    mut trace: Option<ResMut<Trace>>,
) {
    let link = trigger.entity;
    let Ok((remote, user_data)) = links.get(link) else {
        return;
    };
    let peer = remote.0;
    let client_id = peer.to_bits();
    let control = ControlledBy {
        owner: link,
        lifetime: Lifetime::Persistent,
    };

    if let Some((entity, mut human, player)) = humans
        .iter_mut()
        .find(|(_, _, player)| player.client_id == client_id)
    {
        human.disconnected_ticks = 0;
        commands.entity(entity).insert(control);
        info!("{} ({peer:?}) reconnected", player.name);
        outbox.announce(format!("{} 重新连线", player.name));
        if let Some(trace) = trace.as_mut() {
            trace.push(format!("connect {client_id}"));
        }
        return;
    }

    if let Some((entity, mut session, _)) = specs
        .iter_mut()
        .find(|(_, session, _)| session.client_id == client_id)
    {
        session.disconnected_ticks = 0;
        session.link = link;
        session.peer = peer;
        session.watched = true;
        commands
            .entity(entity)
            .insert(Replicate::to_clients(NetworkTarget::Single(peer)));
        info!("{} ({peer:?}) back to spectating", session.name);
        return;
    }

    // Playing and Results never take a combat seat. A full lobby queues instead of
    // refusing, until the watcher cap. Starting stays closed: the roster is already set.
    let Some(phase) = state.iter().next().map(|state| state.phase) else {
        commands.trigger(Disconnect { entity: link });
        return;
    };
    let held = humans.iter().filter(|(_, human, _)| !human.left).count();
    let watching = specs.iter().count();
    let name = name_from_token(user_data, client_id);
    match spectate::admit(phase, ops.is_draining(), held, watching, config.max_players) {
        Admit::Reject => {
            info!("rejecting {client_id}: not accepting new players");
            commands.trigger(Disconnect { entity: link });
        }
        Admit::Combatant => {
            let now = tick.as_ref().map(|tick| tick.0).unwrap_or(0);
            // A seat cleared by the grace timeout is a new body, but the same player.
            let returning = cleared.take(client_id, now);
            let name = returning.clone().unwrap_or(name);
            if returning.is_some() {
                info!("{name} ({peer:?}) came back after the grace expired; new body");
                outbox.announce(format!("{name} 回到废土"));
            } else {
                info!("{name} ({peer:?}) joined");
                outbox.announce(format!("{name} 进入了废土"));
            }
            let player = Player {
                client_id,
                name,
                color: color_for(client_id),
                bot: false,
            };
            let entity = spawn_combatant(&mut commands, player, Some(peer));
            commands.entity(entity).insert((
                control,
                Human {
                    peer,
                    disconnected_ticks: 0,
                    joined_at: now,
                    left: false,
                },
                crate::gameplay::Idle::default(),
            ));
            if let Some(trace) = trace.as_mut() {
                trace.push(format!("connect {client_id}"));
            }
        }
        Admit::Spectator => {
            let following = first_id(
                players
                    .iter()
                    .map(|(player, stats)| (player.client_id, stats)),
            );
            let queue_pos = (watching as u8).saturating_add(1);
            info!("{name} ({peer:?}) is spectating");
            spectate::spawn_spectator(
                &mut commands,
                &mut seq,
                trace.as_deref_mut(),
                NewSpectator {
                    client_id,
                    name,
                    peer,
                    link,
                    order: 0,
                    following,
                    queue_pos,
                    watched: true,
                    replicate: true,
                    log: true,
                },
            );
        }
    }
}

/// When a dropped player's link is despawned, Bevy strips the `ControlledBy` relationship from
/// their body but leaves its `ControlledSend` marker behind. That marker is only filtered by
/// `ControlledBy`, so without it every client would be told it controls the body (and claim it
/// as its local player). `on_connected` puts both back if the player returns.
fn hide_orphaned_control(trigger: On<Remove<ControlledBy>>, mut commands: Commands) {
    if let Ok(mut entity) = commands.get_entity(trigger.entity) {
        entity.try_remove::<ControlledSend>();
    }
}

pub fn color_for(id: u64) -> [f32; 3] {
    let hue = (id.wrapping_mul(2654435761) % 360) as f32;
    let c = Color::hsl(hue, 0.75, 0.55).to_srgba();
    [c.red, c.green, c.blue]
}

/// Count down the grace period of humans whose link went away; remove them once it expires.
pub(crate) fn track_presence(
    mut commands: Commands,
    state: Query<&MatchState>,
    catching: Option<Res<CatchingUp>>,
    tick: Option<Res<SimTick>>,
    mut cleared: ResMut<ClearedSeats>,
    mut departed: Query<&mut Departed>,
    mut humans: Query<(
        Entity,
        &mut Human,
        &Player,
        &Stats,
        &Evolution,
        Option<&ControlledBy>,
    )>,
    live_links: Query<(), With<Connected>>,
    mut outbox: ResMut<Outbox>,
    mut trace: Option<ResMut<Trace>>,
) {
    // Catch-up has no links yet. Recovering keeps its own 20s grace.
    if catching.is_some() || state.iter().any(|s| s.phase == Phase::Recovering) {
        return;
    }
    let playing = state.iter().any(|s| s.phase == Phase::Playing);
    let now = tick.as_ref().map(|tick| tick.0).unwrap_or(0);
    cleared.prune(now);
    let mut expired = Vec::new();
    for (entity, mut human, player, stats, evo, control) in &mut humans {
        if human.left {
            continue;
        }
        let online = control.is_some_and(|c| live_links.contains(c.owner));
        if online {
            human.disconnected_ticks = 0;
            continue;
        }
        if human.disconnected_ticks == 0 {
            info!("{} lost connection, holding slot", player.name);
            if let Some(trace) = trace.as_mut() {
                trace.push(format!("disconnect {}", player.client_id));
            }
        }
        human.disconnected_ticks += 1;
        if human.disconnected_ticks > RECONNECT_GRACE_TICKS {
            info!("{} timed out", player.name);
            outbox.announce(format!("{} 离开了废土", player.name));
            if let Some(trace) = trace.as_mut() {
                trace.push(format!("leave {}", player.client_id));
            }
            if playing {
                expired.push(Departure::from_combatant(player, stats, evo));
            }
            // A rejoin after this point is a new body; keep the name so it is still them.
            cleared.remember(player.client_id, &player.name, now);
            commands.entity(entity).despawn();
        }
    }
    if !expired.is_empty()
        && let Ok(mut departed) = departed.single_mut()
    {
        departed.0.extend(expired);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evoclash_game::protocol::GameEvent;

    #[test]
    fn a_cleared_seat_keeps_its_name_for_one_rejoin() {
        let mut cleared = ClearedSeats::default();
        assert_eq!(cleared.take(7, 0), None, "nothing cleared yet");
        cleared.remember(7, "Alice", 100);
        assert_eq!(cleared.take(7, 150), Some("Alice".into()));
        // One-shot: the reconnect spawned a body, so the next drop goes through its `Human`.
        assert_eq!(cleared.take(7, 151), None);

        cleared.remember(9, "Bob", 10);
        assert_eq!(
            cleared.take(9, 10 + ClearedSeats::KEEP_TICKS + 1),
            None,
            "an old name is not reused"
        );

        cleared.remember(3, "Cara", 0);
        cleared.remember(4, "Drew", 0);
        cleared.prune(ClearedSeats::KEEP_TICKS + 1);
        assert_eq!(cleared.names.len(), 0);
    }

    /// Despawning a dropped player's link must not leave their body marked as controlled for
    /// everyone else.
    #[test]
    fn dropped_link_leaves_no_control_marker() {
        let mut app = App::new();
        app.add_observer(hide_orphaned_control);
        let link = app.world_mut().spawn_empty().id();
        let body = app
            .world_mut()
            .spawn(ControlledBy {
                owner: link,
                lifetime: Lifetime::Persistent,
            })
            .id();
        assert!(app.world().entity(body).contains::<ControlledSend>());

        app.world_mut().despawn(link);
        app.update();
        let body = app.world().entity(body);
        assert!(!body.contains::<ControlledBy>());
        assert!(!body.contains::<ControlledSend>());
    }

    /// A human whose link dies keeps their seat (gear, score) for the grace window, then is
    /// removed and their name remembered for a late rejoin.
    #[test]
    fn a_dropped_seat_is_held_for_the_grace_then_cleared() {
        let mut app = App::new();
        app.init_resource::<Outbox>();
        app.init_resource::<ClearedSeats>();
        app.init_resource::<SimTick>();
        app.init_resource::<NetworkingMetadata>();
        app.world_mut().spawn(MatchState {
            phase: Phase::Playing,
            ..default()
        });
        app.add_systems(Update, track_presence);

        let link = app
            .world_mut()
            .spawn((Connected, RemoteId(PeerId::Netcode(42))))
            .id();
        let body = app
            .world_mut()
            .spawn((
                Player {
                    client_id: 42,
                    name: "Alice".into(),
                    color: [1.0, 0.5, 0.5],
                    bot: false,
                },
                Stats::default(),
                Evolution::default(),
                Human {
                    peer: PeerId::Netcode(42),
                    disconnected_ticks: 0,
                    joined_at: 0,
                    left: false,
                },
                ControlledBy {
                    owner: link,
                    lifetime: Lifetime::Persistent,
                },
            ))
            .id();

        // Still connected: nothing counts down.
        app.update();
        assert_eq!(
            app.world()
                .entity(body)
                .get::<Human>()
                .unwrap()
                .disconnected_ticks,
            0
        );

        // Link gone: the seat is held for the grace.
        app.world_mut().despawn(link);
        for _ in 0..RECONNECT_GRACE_TICKS {
            app.update();
        }
        assert!(
            app.world().get_entity(body).is_ok(),
            "seat held during grace"
        );
        assert!(!app.world().entity(body).get::<Human>().unwrap().connected());

        // Past the grace: removed, and the name is kept for the rejoin that follows.
        app.update();
        assert!(
            app.world().get_entity(body).is_err(),
            "seat cleared after grace"
        );
        let tick = app.world().resource::<SimTick>().0;
        assert_eq!(
            app.world_mut()
                .resource_mut::<ClearedSeats>()
                .take(42, tick),
            Some("Alice".into())
        );
        // Everyone still in the match is told.
        assert!(
            app.world()
                .resource::<Outbox>()
                .reliable
                .iter()
                .any(|event| matches!(event, GameEvent::Announce(text) if text.contains("Alice")))
        );
    }

    /// A link that comes back inside the grace keeps the same body: no new spawn, no lost gear.
    #[test]
    fn a_link_that_returns_inside_the_grace_keeps_the_seat() {
        let mut app = App::new();
        app.init_resource::<Outbox>();
        app.init_resource::<ClearedSeats>();
        app.init_resource::<SimTick>();
        app.init_resource::<NetworkingMetadata>();
        app.world_mut().spawn(MatchState {
            phase: Phase::Playing,
            ..default()
        });
        app.add_systems(Update, track_presence);

        let link = app
            .world_mut()
            .spawn((Connected, RemoteId(PeerId::Netcode(7))))
            .id();
        let body = app
            .world_mut()
            .spawn((
                Player {
                    client_id: 7,
                    name: "Bob".into(),
                    color: [0.5, 1.0, 0.5],
                    bot: false,
                },
                Stats::default(),
                Evolution::default(),
                Human {
                    peer: PeerId::Netcode(7),
                    disconnected_ticks: 0,
                    joined_at: 0,
                    left: false,
                },
                ControlledBy {
                    owner: link,
                    lifetime: Lifetime::Persistent,
                },
            ))
            .id();
        app.update();

        // Ten seconds down, then the same client id connects on a new link.
        app.world_mut().despawn(link);
        for _ in 0..(64 * 10) {
            app.update();
        }
        let returning = app
            .world_mut()
            .spawn((Connected, RemoteId(PeerId::Netcode(7))))
            .id();
        app.world_mut().entity_mut(body).insert(ControlledBy {
            owner: returning,
            lifetime: Lifetime::Persistent,
        });
        app.update();
        assert!(app.world().get_entity(body).is_ok(), "same body, same seat");
        assert!(app.world().entity(body).get::<Human>().unwrap().connected());
        assert!(app.world().resource::<ClearedSeats>().names.is_empty());
    }
}
