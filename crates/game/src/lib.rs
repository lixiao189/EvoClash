//! Shared core of EvoClash: protocol, arena and the rules both server and client simulate.

pub mod items;
pub mod map;
pub mod nav;
pub mod net;
pub mod protocol;
pub mod sim;

use bevy::prelude::*;

pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(protocol::ProtocolPlugin);
        app.init_resource::<sim::Arena>();
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
    }

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct JoinResponse {
        pub match_id: String,
        pub server_addr: String,
        pub client_id: u64,
        /// Base64-encoded netcode connect token.
        pub connect_token: String,
        /// Secret that lets this client get a fresh token for the same match after a drop.
        pub reconnect_key: String,
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
        pub players: u32,
        pub max_players: u32,
        /// Whether the server is accepting players (false once it is draining).
        pub accepting: bool,
        /// Average simulation tick time in microseconds, for capacity planning.
        pub tick_us: u32,
    }
}
