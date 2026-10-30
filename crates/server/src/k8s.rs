//! Kubernetes StatefulSet mode (`--statefulset`): identity, port, public address and liveness.
//!
//! The chart runs one match process per pod of a StatefulSet, on the host network:
//! - **identity**: `HOSTNAME` is the pod name `<statefulset>-<ordinal>`, so the ordinal is its
//!   last `-` segment. The UDP port is `EVOCLASH_BASE_PORT + ordinal`, which never collides with
//!   another pod of the set on the same node, and the address clients dial is `status.hostIP`
//!   (downward API) or `EVOCLASH_PUBLIC_HOST` when the node's public IP differs from its
//!   internal one.
//! - **match id**: `<pod>-<suffix>`, drawn when the pod has no room to resume. The room loops
//!   matches, so a pod keeps its id while it is healthy. After a crash the StatefulSet restarts
//!   the container under the same pod name, and the process resumes the room from the journal on
//!   its own volume (same match id, same seed, same address and port) instead of opening a new
//!   one; see [`start_from_journal`]. Only after [`MAX_RECOVERIES`] crashes does it start a new
//!   room, and a room that exited on purpose (drain, certificate retirement) is never resumed.
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

/// A StatefulSet pod resumes the room on its volume at most this many times, then opens a new
/// one (gameplay.md: the third crash voids the room). Mirrors `MAX_RECOVERIES` in
/// `crates/matchmaker/src/recovery.rs`, which counts for a local fleet; a pod has no matchmaker
/// child to count for it, so the count lives on the volume.
pub const MAX_RECOVERIES: u8 = 2;
/// Counter file on the journal volume: `<match id> <recoveries>`.
const RECOVERIES_FILE: &str = "recoveries";

/// What `--statefulset` should do with the journal it finds on its volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// Nothing to resume: open a new room under a new match id and delete the old logs.
    Fresh,
    /// Crash recovery: resume this room, same match id and seed, like `--recover`.
    Resume { match_id: String, room_seed: u64 },
}

/// The whole startup decision, from the journal directory alone, so it is testable without a
/// cluster. A resumed room keeps the pod's address and port (`basePort + ordinal`); only the
/// process changed.
pub fn start_from_journal(dir: &Path) -> Start {
    let Some(head) = crate::journal::unfinished(dir) else {
        crate::journal::clear(dir);
        clear_recoveries(dir);
        return Start::Fresh;
    };
    let done = recoveries(dir, &head.match_id);
    if done >= MAX_RECOVERIES {
        println!(
            "match {} crashed {} times; opening a new room",
            head.match_id,
            done + 1
        );
        crate::journal::clear(dir);
        clear_recoveries(dir);
        return Start::Fresh;
    }
    write_recoveries(dir, &head.match_id, done + 1);
    Start::Resume {
        match_id: head.match_id,
        room_seed: head.room_seed,
    }
}

/// Replacement processes this volume has already seen for `match_id`.
fn recoveries(dir: &Path, match_id: &str) -> u8 {
    let Ok(text) = std::fs::read_to_string(dir.join(RECOVERIES_FILE)) else {
        return 0;
    };
    let mut parts = text.split_whitespace();
    let (Some(id), Some(count)) = (parts.next(), parts.next()) else {
        return 0;
    };
    if id != match_id {
        return 0;
    }
    count.parse().unwrap_or(0)
}

/// A volume that cannot hold this file cannot hold a room log either, so failing to write is
/// worth a warning, not a reason to refuse to start.
fn write_recoveries(dir: &Path, match_id: &str, count: u8) {
    let path = dir.join(RECOVERIES_FILE);
    if let Err(e) = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&path, format!("{match_id} {count}\n")))
    {
        warn!("cannot write {}: {e}", path.display());
    }
}

fn clear_recoveries(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(RECOVERIES_FILE));
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

    /// A scratch journal directory, wiped before and after the test.
    fn journal_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evoclash-journal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Room log with one snapshot line, as the journal writes it.
    fn write_room(dir: &Path, match_id: &str, seed: u64) {
        std::fs::write(
            dir.join(format!("room-{match_id}.log")),
            format!(
                "EVOCLASH room v1\nroom_seed={seed} code=- private=0 bots=8 monsters=1 host=0 match_index=0 phase=playing paused=0\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn an_unfinished_journal_resumes_the_same_match_and_a_finished_one_starts_fresh() {
        let dir = journal_dir("resume");
        assert_eq!(start_from_journal(&dir), Start::Fresh);

        write_room(&dir, "evoclash-server-0-00ab12", 4242);
        assert_eq!(
            start_from_journal(&dir),
            Start::Resume {
                match_id: "evoclash-server-0-00ab12".into(),
                room_seed: 4242,
            }
        );

        // A process that exited on purpose marks the log: the next start opens a new room and
        // the old logs go away.
        crate::journal::mark_finished(&dir, "evoclash-server-0-00ab12");
        assert_eq!(start_from_journal(&dir), Start::Fresh);
        assert!(!dir.join("room-evoclash-server-0-00ab12.log").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_third_crash_opens_a_new_room() {
        let dir = journal_dir("crashes");
        write_room(&dir, "pod-0-abc", 7);
        assert!(matches!(start_from_journal(&dir), Start::Resume { .. }));
        assert_eq!(recoveries(&dir, "pod-0-abc"), 1);
        assert!(matches!(start_from_journal(&dir), Start::Resume { .. }));
        assert_eq!(recoveries(&dir, "pod-0-abc"), 2);

        // Third crash: the room is void. A new room, and the counter and logs are gone.
        assert_eq!(start_from_journal(&dir), Start::Fresh);
        assert!(!dir.join("room-pod-0-abc.log").exists());
        assert_eq!(recoveries(&dir, "pod-0-abc"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_finished_log_and_a_stale_counter_start_fresh() {
        let dir = journal_dir("stale");
        // Log from an older room, already closed, next to a counter for it.
        write_room(&dir, "pod-0-old", 3);
        crate::journal::mark_finished(&dir, "pod-0-old");
        write_recoveries(&dir, "pod-0-old", 2);
        assert_eq!(start_from_journal(&dir), Start::Fresh);
        assert_eq!(recoveries(&dir, "pod-0-old"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
