//! Agones integration (`--agones`): run as a GameServer pod on Kubernetes.
//!
//! Talks to the Agones SDK sidecar over its local REST API, so no gRPC stack is needed:
//! - startup: read our GameServer name and public `node-ip:host-port`, then mark it `Ready`;
//! - `Health` pings only while the simulation keeps ticking, so a hung process gets replaced;
//! - the first human to join self-`Allocate`s the GameServer, which protects it from
//!   fleet scale-down while a match is in progress (empty `Ready` servers are the warm pool);
//! - `Shutdown` once the process is done, so Agones replaces it with a fresh pod.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bevy::prelude::*;

use crate::connections::Human;

const HEALTH_EVERY: Duration = Duration::from_secs(2);
const DISCOVER_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Resource, Clone)]
pub struct AgonesSdk {
    base: String,
    agent: ureq::Agent,
}

impl AgonesSdk {
    pub fn from_env() -> Self {
        let port = std::env::var("AGONES_SDK_HTTP_PORT").unwrap_or_else(|_| "9358".into());
        Self {
            base: format!("http://127.0.0.1:{port}"),
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(2)))
                .build()
                .into(),
        }
    }

    fn post(&self, path: &str) -> Result<(), String> {
        self.agent
            .post(&format!("{}{path}", self.base))
            .send_json(serde_json::json!({}))
            .map(|_| ())
            .map_err(|e| format!("agones {path}: {e}"))
    }

    pub fn ready(&self) -> Result<(), String> {
        self.post("/ready")
    }

    pub fn allocate(&self) -> Result<(), String> {
        self.post("/allocate")
    }

    pub fn shutdown(&self) -> Result<(), String> {
        self.post("/shutdown")
    }

    fn health(&self) -> Result<(), String> {
        self.post("/health")
    }

    /// Wait for the sidecar, then return our GameServer name and the address clients reach us
    /// on. The pod binds a fixed container port; Agones maps a dynamic host port to it.
    pub fn discover(&self) -> Result<(String, SocketAddr), String> {
        let deadline = Instant::now() + DISCOVER_TIMEOUT;
        loop {
            match self.try_discover() {
                Ok(found) => return Ok(found),
                Err(e) if Instant::now() > deadline => return Err(e),
                Err(_) => std::thread::sleep(Duration::from_millis(500)),
            }
        }
    }

    fn try_discover(&self) -> Result<(String, SocketAddr), String> {
        let gs: serde_json::Value = self
            .agent
            .get(&format!("{}/gameserver", self.base))
            .call()
            .map_err(|e| format!("agones /gameserver: {e}"))?
            .body_mut()
            .read_json()
            .map_err(|e| format!("agones /gameserver reply: {e}"))?;
        parse_gameserver(&gs)
    }
}

/// The REST gateway has used both snake_case and camelCase field names across versions.
fn field<'a>(value: &'a serde_json::Value, snake: &str, camel: &str) -> &'a serde_json::Value {
    let found = &value[snake];
    if found.is_null() {
        &value[camel]
    } else {
        found
    }
}

fn parse_gameserver(gs: &serde_json::Value) -> Result<(String, SocketAddr), String> {
    let name = field(gs, "object_meta", "objectMeta")["name"]
        .as_str()
        .filter(|n| !n.is_empty())
        .ok_or("GameServer has no name yet")?;
    let status = &gs["status"];
    let ip: IpAddr = status["address"]
        .as_str()
        .ok_or("GameServer has no address yet")?
        .parse()
        .map_err(|e| format!("GameServer address: {e}"))?;
    let port = status["ports"]
        .as_array()
        .and_then(|ports| ports.first())
        .and_then(|p| p["port"].as_u64())
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)
        .ok_or("GameServer has no port yet")?;
    Ok((name.to_owned(), SocketAddr::new(ip, port)))
}

/// Advanced every simulation tick; the health thread only pings while it moves.
#[derive(Resource, Clone, Default)]
struct TickCount(Arc<AtomicU64>);

pub struct AgonesPlugin(pub AgonesSdk);

impl Plugin for AgonesPlugin {
    fn build(&self, app: &mut App) {
        let ticks = TickCount::default();
        spawn_health_thread(self.0.clone(), ticks.clone());
        app.insert_resource(self.0.clone());
        app.insert_resource(ticks);
        app.add_systems(FixedLast, |ticks: Res<TickCount>| {
            ticks.0.fetch_add(1, Ordering::Relaxed);
        });
        // PostStartup: the UDP socket is open by then, so allocated players can connect.
        app.add_systems(PostStartup, |sdk: Res<AgonesSdk>| match sdk.ready() {
            Ok(()) => info!("agones: GameServer ready"),
            Err(e) => error!("{e}"),
        });
        app.add_systems(Update, allocate_on_first_player);
    }
}

fn allocate_on_first_player(
    sdk: Res<AgonesSdk>,
    humans: Query<(), With<Human>>,
    mut allocated: Local<bool>,
) {
    if *allocated || humans.is_empty() {
        return;
    }
    *allocated = true;
    match sdk.allocate() {
        Ok(()) => info!("agones: GameServer allocated"),
        Err(e) => warn!("{e}"),
    }
}

fn spawn_health_thread(sdk: AgonesSdk, ticks: TickCount) {
    std::thread::Builder::new()
        .name("agones-health".into())
        .spawn(move || {
            let mut last = 0;
            loop {
                std::thread::sleep(HEALTH_EVERY);
                let now = ticks.0.load(Ordering::Relaxed);
                if now == last {
                    warn!("agones: simulation stalled, withholding health ping");
                    continue;
                }
                last = now;
                if let Err(e) = sdk.health() {
                    warn!("{e}");
                }
            }
        })
        .expect("failed to spawn agones health thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_name_and_public_address() {
        let gs = serde_json::json!({
            "object_meta": { "name": "evoclash-abc12" },
            "status": {
                "state": "Scheduled",
                "address": "10.0.0.7",
                "ports": [{ "name": "game", "port": 7342 }]
            }
        });
        let (name, addr) = parse_gameserver(&gs).unwrap();
        assert_eq!(name, "evoclash-abc12");
        assert_eq!(addr, "10.0.0.7:7342".parse().unwrap());
    }

    #[test]
    fn waits_until_address_is_assigned() {
        let gs = serde_json::json!({
            "objectMeta": { "name": "evoclash-abc12" },
            "status": { "address": "", "ports": [] }
        });
        assert!(parse_gameserver(&gs).is_err());
    }
}
