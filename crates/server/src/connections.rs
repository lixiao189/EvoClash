//! Transport setup and the player session lifecycle: join, drop, reconnect.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Instant;

use bevy::prelude::*;
use evoclash_game::net::{self, CONNECTION_TIMEOUT_SECS, PROTOCOL_ID};
use evoclash_game::protocol::Player;
use lightyear::connection::client::Connected;
use lightyear::netcode::{NetcodeServer, TokenUserData};
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use lightyear::webtransport::prelude::Identity;
use lightyear::webtransport::prelude::endpoint::WebTransportEndpoint;

use crate::config::ServerConfig;
use crate::gameplay::{Outbox, SimSet, spawn_combatant};

/// A disconnected player's body stays in the arena this long so a flaky
/// connection can resume with its gear intact.
pub const RECONNECT_GRACE_TICKS: u32 = 64 * 20;

/// Server-side marker for human-controlled combatants.
#[derive(Component)]
pub struct Human {
    pub peer: PeerId,
    pub disconnected_ticks: u32,
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
        app.add_systems(FixedUpdate, track_presence.in_set(SimSet::Input));
    }
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
    .with_max_clients(config.max_players);
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
        "match {} listening on WebTransport https://{bind} (public {}), max {} players, cert {}",
        config.match_id, config.public_addr, config.max_players, tls.digest
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
    mut outbox: ResMut<Outbox>,
    mut commands: Commands,
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
        return;
    }

    let name = name_from_token(user_data, client_id);
    info!("{name} ({peer:?}) joined");
    outbox.announce(format!("{name} 进入了废土"));
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
        },
    ));
}

pub fn color_for(id: u64) -> [f32; 3] {
    let hue = (id.wrapping_mul(2654435761) % 360) as f32;
    let c = Color::hsl(hue, 0.75, 0.55).to_srgba();
    [c.red, c.green, c.blue]
}

/// Count down the grace period of humans whose link went away; remove them once it expires.
fn track_presence(
    mut commands: Commands,
    mut humans: Query<(Entity, &mut Human, &Player, Option<&ControlledBy>)>,
    live_links: Query<(), With<Connected>>,
    mut outbox: ResMut<Outbox>,
) {
    for (entity, mut human, player, control) in &mut humans {
        let online = control.is_some_and(|c| live_links.contains(c.owner));
        if online {
            human.disconnected_ticks = 0;
            continue;
        }
        if human.disconnected_ticks == 0 {
            info!("{} lost connection, holding slot", player.name);
        }
        human.disconnected_ticks += 1;
        if human.disconnected_ticks > RECONNECT_GRACE_TICKS {
            info!("{} timed out", player.name);
            outbox.announce(format!("{} 离开了废土", player.name));
            commands.entity(entity).despawn();
        }
    }
}
