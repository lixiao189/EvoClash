//! Local process fleet: launches one `evoclash-server` per room on this host.
//! Rooms return to the lobby after results, so this does not pass `--single-match`.
//!
//! In a cluster the match servers are pods of a StatefulSet instead (`--k8s-scale`,
//! `scaler.rs`): they heartbeat in and this module stays off (`--no-launch`). The rest of the
//! matchmaker only needs "give me a fresh server address".

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use evoclash_game::net::KEY_ENV;
use evoclash_game::protocol::ROOM_COMBATANTS;
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

/// How long a stopped server gets to disappear after SIGKILL before the relaunch goes ahead
/// anyway. A frozen or deadlocked process leaves in milliseconds; anything slower is a
/// kernel-level stall the janitor must not block on.
const STOP_TIMEOUT: Duration = Duration::from_millis(500);

/// Outcome of [`Fleet::stop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// We held the child: SIGKILL was sent and it exited (`None` when it went down to a signal).
    Killed(Option<i32>),
    /// SIGKILL was sent but the process was still there after [`STOP_TIMEOUT`].
    Linger,
    /// No child under that id: adopt-only (`--no-launch`) or already reaped. Nothing to kill.
    NotOurs,
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

    /// This fleet launched `id` and still holds its process. A match adopted from heartbeats
    /// after a matchmaker restart is not ours: we cannot tell it is dead, nor kill it.
    pub fn owns(&self, id: &str) -> bool {
        self.children.lock().is_ok_and(|c| c.contains_key(id))
    }

    /// Start a public match server on the first free port.
    pub fn launch(&self, used: &[SocketAddr]) -> Result<(String, SocketAddr), String> {
        self.spawn(used, rand::random(), None, None)
    }

    /// `room_code` set means a private room: `--private --room-code CODE --bots 8`, monsters on.
    pub fn launch_room(
        &self,
        used: &[SocketAddr],
        room_seed: u64,
        room_code: Option<&str>,
    ) -> Result<(String, SocketAddr), String> {
        self.spawn(used, room_seed, room_code, None)
    }

    /// Relaunch a crashed room: same id and seed, plus `--recover`. The port is new.
    pub fn launch_recover(
        &self,
        id: &str,
        used: &[SocketAddr],
        room_seed: u64,
        room_code: Option<&str>,
    ) -> Result<SocketAddr, String> {
        self.spawn(used, room_seed, room_code, Some(id))
            .map(|(_, addr)| addr)
    }

    fn spawn(
        &self,
        used: &[SocketAddr],
        room_seed: u64,
        room_code: Option<&str>,
        recover_id: Option<&str>,
    ) -> Result<(String, SocketAddr), String> {
        let mut children = self.children.lock().map_err(|_| "fleet lock poisoned")?;
        if recover_id.is_none() && children.len() >= self.config.max_matches {
            return Err("fleet at capacity".into());
        }
        // Already stopped and waited for by `Fleet::stop` before a relaunch; this is the last
        // resort for a caller that spawned a replacement without stopping the old one first.
        if let Some(id) = recover_id
            && let Some(mut child) = children.remove(id)
        {
            let _ = child.start_kill();
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
        let id = match recover_id {
            Some(id) => id.to_string(),
            None => format!("m{port}-{:06x}", rand::random::<u32>() & 0xff_ffff),
        };
        // Every room is fixed at ROOM_COMBATANTS, private ones included: the host cannot
        // disable bots or change the fill target.
        let bots = if room_code.is_some() {
            u32::from(ROOM_COMBATANTS)
        } else {
            self.config.bots
        };

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
            .arg(bots.to_string())
            .arg("--matchmaker")
            .arg(self.config.matchmaker_urls.join(","))
            .arg("--room-seed")
            .arg(room_seed.to_string())
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            // Matches outlive a matchmaker restart; the new instance re-adopts them via heartbeats.
            .kill_on_drop(false);
        if let Some(code) = room_code {
            command.arg("--private").arg("--room-code").arg(code);
        }
        if recover_id.is_some() {
            command.arg("--recover");
        }
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

    /// SIGKILL a server we launched and wait for it to be gone.
    ///
    /// Used before relaunching a room with `--recover`. A heartbeat timeout does not mean the
    /// process is dead: a SIGSTOPped or deadlocked server is still running and would keep
    /// appending to the same `logs/<id>.log` as its replacement, so it has to be gone first.
    ///
    /// `--no-launch` (a cluster, servers are pods the matchmaker does not own) holds no child:
    /// [`Stopped::NotOurs`] is returned and the caller keeps its old behaviour (no recovery).
    pub async fn stop(&self, id: &str) -> Stopped {
        let child = match self.children.lock() {
            Ok(mut children) => children.remove(id),
            Err(_) => None,
        };
        let Some(mut child) = child else {
            return Stopped::NotOurs;
        };
        let _ = child.start_kill();
        match tokio::time::timeout(STOP_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => Stopped::Killed(status.code()),
            // Already reaped, or no longer our child: either way it is not running any more.
            Ok(Err(_)) => Stopped::Killed(None),
            Err(_) => Stopped::Linger,
        }
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

/// Real child processes: a heartbeat timeout is not an exit, so the kill has to be proven.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn fleet() -> Fleet {
        Fleet::new(FleetConfig {
            server_bin: PathBuf::from("sleep"),
            public_host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ports: 7090..=7099,
            max_matches: 4,
            max_players: 8,
            bots: 0,
            matchmaker_urls: Vec::new(),
            log_dir: std::env::temp_dir(),
        })
    }

    /// A long-running child of ours, stored the way `spawn` stores one.
    fn hold(fleet: &Fleet, id: &str, program: &str, args: &[&str]) -> u32 {
        let child = Command::new(program)
            .args(args)
            .kill_on_drop(false)
            .spawn()
            .expect("spawn test child");
        let pid = child.id().expect("pid");
        fleet.children.lock().unwrap().insert(id.to_string(), child);
        pid
    }

    fn signal(flag: &str, pid: u32) -> bool {
        std::process::Command::new("kill")
            .args([flag, &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn a_frozen_server_is_killed_and_gone_before_a_relaunch() {
        let fleet = fleet();
        let pid = hold(&fleet, "hung", "sleep", &["60"]);
        // What `fault-inject freeze-room` does: the process is alive, just not answering.
        assert!(signal("-STOP", pid));
        assert!(signal("-0", pid));

        assert_eq!(fleet.stop("hung").await, Stopped::Killed(None));
        // Reaped, so not even a zombie: nothing left to write the room log.
        assert!(!signal("-0", pid), "the frozen process outlived the kill");
        assert_eq!(fleet.running(), 0);
        // Second relaunch attempt for the same id has no child to kill any more.
        assert_eq!(fleet.stop("hung").await, Stopped::NotOurs);
    }

    #[tokio::test]
    async fn a_server_that_already_exited_is_only_reaped() {
        let fleet = fleet();
        hold(&fleet, "gone", "true", &[]);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(fleet.stop("gone").await, Stopped::Killed(Some(0)));
        assert_eq!(fleet.running(), 0);
    }

    #[tokio::test]
    async fn a_server_we_never_launched_has_nothing_to_kill() {
        assert_eq!(fleet().stop("pod-0").await, Stopped::NotOurs);
    }
}
