//! Shared core of EvoClash: protocol, arena and the rules both server and client simulate.

pub mod evolution;
pub mod items;
pub mod map;
pub mod nav;
pub mod net;
pub mod protocol;
pub mod rng;
pub mod shop;
pub mod sim;

use bevy::prelude::*;

pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(protocol::ProtocolPlugin);
        app.init_resource::<sim::Arena>();
        app.init_resource::<sim::Obstacles>();
    }
}

/// JSON contract between the matchmaker and match servers / clients.
pub mod api {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct JoinRequest {
        pub name: String,
        /// Preferred region; the matchmaker falls back to any region with capacity.
        #[serde(default)]
        pub region: Option<String>,
        /// Set when joining one match on purpose. Missing or unknown is a quick start.
        #[serde(default)]
        pub match_id: Option<String>,
        /// Private room code. Absent is a quick start. Compared case-insensitively.
        #[serde(default)]
        pub room: Option<String>,
    }

    /// `POST /v1/rooms` body. A public room is an ordinary listed room the creator joins
    /// first. A private room has a code and a host, and is never listed.
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct CreateRoomRequest {
        pub name: String,
        #[serde(default)]
        pub private: bool,
    }

    /// `GET /v1/rooms`: public rooms with at least one human (combatant or spectator).
    /// Empty warm servers are not listed. An empty list means 进入废土 is off.
    #[derive(Serialize, Deserialize, Debug, Clone, Default)]
    pub struct RoomList {
        pub rooms: Vec<RoomSummary>,
    }

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct RoomSummary {
        /// Pass back as `JoinRequest::match_id` to join this room.
        pub match_id: String,
        /// `lobby` | `starting` | `playing` | `results`.
        pub phase: String,
        pub phase_left_secs: Option<u32>,
        pub humans: u32,
        pub bots: u32,
        pub spectators: u32,
        pub max_players: u32,
        pub max_spectators: u32,
        /// `play`: a combat seat is free now. `spectate`: watch until the next round.
        /// `full`: neither.
        pub entry: String,
    }

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct JoinResponse {
        pub match_id: String,
        pub server_addr: String,
        /// Hex SHA-256 of the server's self-signed WebTransport certificate.
        pub cert_digest: String,
        pub client_id: u64,
        /// Base64-encoded netcode connect token.
        pub connect_token: String,
        /// Secret that lets this client get a fresh token for the same match after a drop.
        pub reconnect_key: String,
        /// Set when this seat is a private room. Quick start leaves it empty.
        #[serde(default)]
        pub room_code: Option<String>,
        /// Joined while a round runs: watch it as a spectator, take a seat next round.
        #[serde(default)]
        pub spectate: bool,
    }

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct RejoinRequest {
        pub match_id: String,
        pub client_id: u64,
        pub reconnect_key: String,
    }

    /// Periodic report from a match server; doubles as its liveness signal.
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct Heartbeat {
        pub match_id: String,
        /// Lets a restarted matchmaker re-adopt servers it didn't launch.
        pub public_addr: String,
        /// Hex SHA-256 of the WebTransport certificate, handed to clients so browsers trust it.
        pub cert_digest: String,
        pub players: u32,
        pub max_players: u32,
        /// Whether the server is accepting players (false once it is draining).
        pub accepting: bool,
        /// Average simulation tick time in microseconds, for capacity planning.
        pub tick_us: u32,
        /// `lobby` | `starting` | `playing` | `results` | `recovering`. Required; no default.
        pub phase: String,
        /// Seconds left in this phase. `None` while a lobby countdown has not started.
        pub phase_left_secs: Option<u32>,
        /// Connected and grace-period humans. `players` is the same count.
        pub humans: u32,
        pub bots: u32,
        /// Connected spectators. Not part of `humans` or `players` (those are combat seats).
        #[serde(default)]
        pub spectators: u32,
        /// True for a private room. Quick start never reserves these.
        pub is_private: bool,
        /// Private room code, uppercase. `None` for a public room.
        pub room_code: Option<String>,
        pub room_seed: u64,
        pub match_index: u32,
        /// `none` | `lobby`. The matchmaker trusts this and does not re-derive it.
        pub join_mode: String,
        /// True on the last heartbeats before a normal exit.
        pub exiting: bool,
    }
}
