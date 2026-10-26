//! Local process fleet: launches one `evoclash-server` per room on this host.
//! Rooms return to the lobby after results, so this does not pass `--single-match`.
//!
//! In production this is the piece you swap for an orchestrator allocation call
//! (Agones `GameServerAllocation`, Edgegap, Hathora, …); the rest of the matchmaker only
//! needs "give me a fresh server address".

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;

use evoclash_game::net::KEY_ENV;
use tokio::process::{Child, Command};

pub struct FleetConfig {
    pub server_bin: PathBuf,
    pub public_host: IpAddr,
    pub ports: RangeInclusive<u16>,
    pub max_matches: usize,
    pub max_players: u32,
    pub bots: u32,
    /// URLs the servers heartbeat to: this matchmaker plus its peer replicas.
    pub matchmaker_urls: Vec<String>,
    pub log_dir: PathBuf,
}

pub struct Fleet {
    pub config: FleetConfig,
    children: Mutex<HashMap<String, Child>>,
}

impl Fleet {
    pub fn new(config: FleetConfig) -> Self {
        Self {
            config,
            children: Mutex::default(),
        }
    }

    pub fn running(&self) -> usize {
        self.children.lock().map(|c| c.len()).unwrap_or(0)
    }

    /// Start a new match server on the first free port.
    pub fn launch(&self, used: &[SocketAddr]) -> Result<(String, SocketAddr), String> {
        let mut children = self.children.lock().map_err(|_| "fleet lock poisoned")?;
        if children.len() >= self.config.max_matches {
            return Err("fleet at capacity".into());
        }
        // Skip ports another process still holds (e.g. a match from before a restart that hasn't
        // heartbeated in yet), so a fresh server never dies on a bind error with a player en route.
        let port = self
            .config
            .ports
            .clone()
            .find(|p| {
                !used.iter().any(|a| a.port() == *p)
                    && UdpSocket::bind((Ipv4Addr::UNSPECIFIED, *p)).is_ok()
            })
            .ok_or("no free ports")?;
        let addr = SocketAddr::new(self.config.public_host, port);
        let id = format!("m{port}-{:06x}", rand::random::<u32>() & 0xff_ffff);
        let room_seed: u64 = rand::random();

        std::fs::create_dir_all(&self.config.log_dir).map_err(|e| format!("log dir: {e}"))?;
        let log = std::fs::File::create(self.config.log_dir.join(format!("{id}.log")))
            .map_err(|e| format!("log file: {e}"))?;
        let mut command = Command::new(&self.config.server_bin);
        command
            .arg("--port")
            .arg(port.to_string())
            .arg("--public-addr")
            .arg(addr.to_string())
            .arg("--match-id")
            .arg(&id)
            .arg("--max-players")
            .arg(self.config.max_players.to_string())
            .arg("--bots")
            .arg(self.config.bots.to_string())
            .arg("--matchmaker")
            .arg(self.config.matchmaker_urls.join(","))
            .arg("--room-seed")
            .arg(room_seed.to_string())
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            // Matches outlive a matchmaker restart; the new instance re-adopts them via heartbeats.
            .kill_on_drop(false);
        if let Ok(key) = std::env::var(KEY_ENV) {
            command.env(KEY_ENV, key);
        }
        let child = command
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", self.config.server_bin.display()))?;
        children.insert(id.clone(), child);
        Ok((id, addr))
    }

    /// Collect exited processes. Returns ids that exited.
    pub fn reap(&self) -> Vec<(String, Option<i32>)> {
        let Ok(mut children) = self.children.lock() else {
            return Vec::new();
        };
        let mut exited = Vec::new();
        children.retain(|id, child| match child.try_wait() {
            Ok(Some(status)) => {
                exited.push((id.clone(), status.code()));
                false
            }
            Ok(None) => true,
            Err(_) => false,
        });
        exited
    }

    /// Kill a server we launched that stopped responding (hung, not crashed).
    pub fn kill(&self, id: &str) {
        if let Ok(mut children) = self.children.lock()
            && let Some(mut child) = children.remove(id)
        {
            let _ = child.start_kill();
        }
    }
}
