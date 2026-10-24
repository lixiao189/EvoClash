//! Network constants and shared secrets.

use core::time::Duration;

/// Bump when the wire protocol changes so stale clients are rejected at handshake.
pub const PROTOCOL_ID: u64 = 0x4556_4f43_0000_0002;

/// Simulation rate. 64 Hz keeps input-to-action latency around 16 ms on the server.
pub const TICK_HZ: f64 = 64.0;

/// Server → client replication cadence (every other tick). Lower means less
/// interpolation delay for remote players at the cost of bandwidth.
pub const REPLICATION_INTERVAL: Duration = Duration::from_micros(31_250);

pub const DEFAULT_GAME_PORT: u16 = 7000;
pub const DEFAULT_MATCHMAKER_ADDR: &str = "127.0.0.1:8080";

/// Seconds a connect token stays valid after the matchmaker issues it.
pub const TOKEN_EXPIRE_SECS: i32 = 30;
/// Seconds without packets before netcode drops a connection.
pub const CONNECTION_TIMEOUT_SECS: i32 = 5;

pub fn tick_duration() -> Duration {
    Duration::from_secs_f64(1.0 / TICK_HZ)
}

/// Env var holding the 32-byte netcode key shared by the matchmaker and all match servers, as hex.
pub const KEY_ENV: &str = "EVOCLASH_NETCODE_KEY";

/// Load the netcode private key. Falls back to a fixed dev key (and says so) when unset,
/// which is only acceptable for local development.
pub fn private_key_from_env() -> ([u8; 32], bool) {
    match std::env::var(KEY_ENV) {
        Ok(hex) => (
            parse_hex_key(&hex).expect("EVOCLASH_NETCODE_KEY must be 64 hex chars"),
            false,
        ),
        Err(_) => ([0x45; 32], true),
    }
}

pub fn parse_hex_key(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(key)
}
