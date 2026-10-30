//! Kubernetes StatefulSet mode (`--statefulset`): identity, port, public address and liveness.
//!
//! The chart runs one match process per pod of a StatefulSet, on the host network:
//! - **identity**: `HOSTNAME` is the pod name `<statefulset>-<ordinal>`, so the ordinal is its
//!   last `-` segment. The UDP port is `EVOCLASH_BASE_PORT + ordinal`, which never collides with
//!   another pod of the set on the same node, and the address clients dial is `status.hostIP`
//!   (downward API) or `EVOCLASH_PUBLIC_HOST` when the node's public IP differs from its
//!   internal one.
//! - **match id**: `<pod>-<suffix>` with a suffix drawn once per process start. The room loops
//!   matches, so a pod keeps its id while it is healthy; after a crash the StatefulSet restarts
//!   the container under the same pod name, and the matchmaker must not hand the new process the
//!   dead one's registry entry (seats, private room code, an `abandoned` flag that would make it
//!   ignore heartbeats forever).
//! - **liveness**: a thread stamps a health file only while the simulation ticks, and the
//!   `--healthcheck` exec probe fails when the stamp is old. The probe runs this same binary
//!   because the runtime image has no shell. It is an exec probe rather than an HTTP one
//!   because `hostNetwork` pods share the node's ports, so no fixed probe port fits every pod.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bevy::prelude::*;

/// Stamp refreshed by the match process, read back by the probe.
pub const DEFAULT_HEALTH_FILE: &str = "/tmp/evoclash-health";
/// Ordinal 0 binds this port; pod N binds `base + N`.
pub const DEFAULT_BASE_PORT: u16 = 7000;
/// A stamp older than this means the simulation is stuck.
pub const MAX_STAMP_AGE: Duration = Duration::from_secs(10);
/// Stamp interval. Short enough that `failureThreshold` catches a stall quickly.
const STAMP_EVERY: Duration = Duration::from_secs(2);

pub const HEALTH_FILE_ENV: &str = "EVOCLASH_HEALTH_FILE";
pub const BASE_PORT_ENV: &str = "EVOCLASH_BASE_PORT";
pub const PUBLIC_HOST_ENV: &str = "EVOCLASH_PUBLIC_HOST";
pub const NODE_IP_ENV: &str = "EVOCLASH_NODE_IP";
pub const POD_IP_ENV: &str = "POD_IP";

/// Everything `--statefulset` derives from the pod it runs in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub pod: String,
    pub ordinal: u32,
    pub port: u16,
    pub public_addr: SocketAddr,
    pub match_id: String,
}

/// `evoclash-server-3` → `3`. Anything else is a misconfigured pod template.
pub fn parse_ordinal(pod: &str) -> Result<u32, String> {
    let tail = pod.rsplit('-').next().unwrap_or_default();
    tail.parse::<u32>()
        .map_err(|_| format!("pod name {pod:?} has no `-<ordinal>` suffix"))
}

pub fn port_for(base: u16, ordinal: u32) -> Result<u16, String> {
    u16::try_from(u32::from(base) + ordinal)
        .map_err(|_| format!("{BASE_PORT_ENV}={base} + ordinal {ordinal} overflows a port"))
}

/// The match id of one process, not of one pod: the next container start gets a new one.
pub fn match_id_for(pod: &str, suffix: u32) -> String {
    format!("{pod}-{suffix:06x}")
}

/// Unique per container start, and not an OS random draw: the room seed stays the only one.
/// The start time folded into 24 bits; two restarts of the same pod cannot collide.
fn start_suffix() -> u32 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let mixed = (now.as_secs() as u32).wrapping_mul(2_654_435_761) ^ now.subsec_nanos();
    mixed & 0xff_ffff
}

pub fn identity_from_env() -> Result<Identity, String> {
    let pod = std::env::var("HOSTNAME")
        .ok()
        .filter(|pod| !pod.trim().is_empty())
        .ok_or_else(|| "HOSTNAME (the pod name) is not set".to_string())?;
    let ordinal = parse_ordinal(&pod)?;
    let base = match std::env::var(BASE_PORT_ENV) {
        Ok(value) => value
            .trim()
            .parse()
            .map_err(|e| format!("{BASE_PORT_ENV}={value:?}: {e}"))?,
        Err(_) => DEFAULT_BASE_PORT,
    };
    let port = port_for(base, ordinal)?;
    let public_addr = SocketAddr::new(public_host()?, port);
    Ok(Identity {
        match_id: match_id_for(&pod, start_suffix()),
        pod,
        ordinal,
        port,
        public_addr,
    })
}

/// The first non-empty of: explicit override, the node's IP, the pod's IP.
fn public_host() -> Result<IpAddr, String> {
    for key in [PUBLIC_HOST_ENV, NODE_IP_ENV, POD_IP_ENV] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            return value
                .parse()
                .map_err(|e| format!("{key}={value:?} is not an IP: {e}"));
        }
    }
    Err(format!(
        "no public address: set {PUBLIC_HOST_ENV}, {NODE_IP_ENV} (downward API status.hostIP) or {POD_IP_ENV}"
    ))
}

pub fn health_file() -> PathBuf {
    std::env::var(HEALTH_FILE_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_HEALTH_FILE))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Write-then-rename, so a probe never reads half a stamp.
fn stamp(path: &Path, at: u64) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, at.to_string())?;
    std::fs::rename(&tmp, path)
}

/// The probe's whole decision, so it is testable without a cluster.
pub fn is_fresh(stamp: Option<u64>, now: u64) -> bool {
    stamp.is_some_and(|at| now.saturating_sub(at) <= MAX_STAMP_AGE.as_secs())
}

/// `--healthcheck`: 0 while the match process ticks, 1 once the stamp is stale.
pub fn healthcheck(path: &Path) -> i32 {
    let stamp = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok());
    let now = now_secs();
    if is_fresh(stamp, now) {
        return 0;
    }
    eprintln!(
        "unhealthy: {} holds {stamp:?}, now {now} (simulation stalled for more than {}s)",
        path.display(),
        MAX_STAMP_AGE.as_secs()
    );
    1
}

/// Advanced every fixed tick; the stamp thread only writes while it moves.
#[derive(Resource, Clone, Default)]
struct TickCount(Arc<AtomicU64>);

pub struct K8sPlugin {
    pub health_file: PathBuf,
}

impl K8sPlugin {
    pub fn from_env() -> Self {
        Self {
            health_file: health_file(),
        }
    }
}

impl Plugin for K8sPlugin {
    fn build(&self, app: &mut App) {
        let ticks = TickCount::default();
        spawn_stamp_thread(self.health_file.clone(), ticks.clone());
        app.insert_resource(ticks);
        app.add_systems(FixedLast, |ticks: Res<TickCount>| {
            ticks.0.fetch_add(1, Ordering::Relaxed);
        });
    }
}

fn spawn_stamp_thread(path: PathBuf, ticks: TickCount) {
    std::thread::Builder::new()
        .name("k8s-health".into())
        .spawn(move || {
            let mut last: Option<u64> = None;
            loop {
                std::thread::sleep(STAMP_EVERY);
                let now = ticks.0.load(Ordering::Relaxed);
                if last == Some(now) {
                    // Leaving the old stamp in place is what fails the probe.
                    warn!("simulation stalled, not refreshing {}", path.display());
                    continue;
                }
                last = Some(now);
                if let Err(e) = stamp(&path, now_secs()) {
                    warn!("cannot write {}: {e}", path.display());
                }
            }
        })
        .expect("failed to spawn the k8s health thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_ordinal_from_the_pod_name() {
        assert_eq!(parse_ordinal("evoclash-server-0").unwrap(), 0);
        assert_eq!(parse_ordinal("evoclash-server-12").unwrap(), 12);
        assert_eq!(parse_ordinal("my.release-evoclash-server-7").unwrap(), 7);
        assert!(parse_ordinal("evoclash-server").is_err());
        assert!(parse_ordinal("evoclash-server-abc").is_err());
        assert!(parse_ordinal("").is_err());
    }

    #[test]
    fn port_follows_the_ordinal() {
        assert_eq!(port_for(7000, 0).unwrap(), 7000);
        assert_eq!(port_for(7000, 3).unwrap(), 7003);
        assert_eq!(port_for(65534, 1).unwrap(), 65535);
        assert!(port_for(65535, 1).is_err());
    }

    #[test]
    fn match_ids_differ_between_restarts_of_the_same_pod() {
        let first = match_id_for("evoclash-server-3", 0x00ab_cdef);
        let second = match_id_for("evoclash-server-3", 0x0000_1234);
        assert_eq!(first, "evoclash-server-3-abcdef");
        assert_ne!(first, second);
        assert!(first.starts_with("evoclash-server-3-"));
    }

    #[test]
    fn only_a_recent_stamp_is_healthy() {
        let fresh = MAX_STAMP_AGE.as_secs();
        assert!(is_fresh(Some(100), 100));
        assert!(is_fresh(Some(100), 100 + fresh));
        assert!(!is_fresh(Some(100), 101 + fresh));
        assert!(!is_fresh(None, 100));
    }

    #[test]
    fn a_missing_file_fails_the_probe() {
        let dir = std::env::temp_dir().join(format!("evoclash-health-{}", std::process::id()));
        let path = dir.join("health");
        assert_eq!(healthcheck(&path), 1);
        std::fs::create_dir_all(&dir).unwrap();
        stamp(&path, now_secs()).unwrap();
        assert_eq!(healthcheck(&path), 0);
        std::fs::write(&path, "not a number").unwrap();
        assert_eq!(healthcheck(&path), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
