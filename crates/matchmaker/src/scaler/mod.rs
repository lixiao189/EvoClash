//! Buffer autoscaling for the match-server StatefulSet (`--k8s-scale <name>`).
//!
//! It uses two Kubernetes objects the matchmaker already can reach: a `coordination.k8s.io/v1`
//! Lease for leader election and the `statefulsets/<name>/scale` subresource for the replica
//! count. No custom resources, no sidecar.
//!
//! - One matchmaker replica at a time holds the Lease; the others stand by. The holder renews
//!   every [`TICK`] and a new holder takes over once `renewTime` is older than [`LEASE_SECONDS`].
//! - Target replicas = matches with people in them + `buffer`, clamped to `min..=max`.
//! - Scale-up is one PATCH. Scale-down only removes the highest ordinals, and only empty ones:
//!   those matches are marked draining in the registry first (so no join is handed a seat on a
//!   pod that is about to go), and a player on a high-ordinal pod holds the shrink until their
//!   match ends.
//! - A pod hosts one room that loops matches, so the empty pods of the buffer are also the warm
//!   pool: no separate warm-match logic here, and an empty room never exits by itself.
//!
//! The decision itself is a pure function (`plan::plan`) so the policy is testable without a
//! cluster. `ureq` is synchronous, so the HTTP calls run in `spawn_blocking`.

mod api;
mod plan;
mod time;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::registry::Registry;

use api::Api;
use plan::{Plan, Slot};

pub(crate) use plan::ordinal_of;

/// How often the leader renews the Lease and re-evaluates the fleet.
const TICK: Duration = Duration::from_secs(5);
/// The holder is replaced when it has not renewed for this long (three missed ticks).
const LEASE_SECONDS: i64 = 15;
/// After taking over, heartbeats from every match server need a moment to arrive; until they do
/// an empty registry must not look like an empty fleet.
const STARTUP_GRACE: Duration = Duration::from_secs(20);
/// Where the API server mounts the service account.
const SA_DIR: &str = "/var/run/secrets/kubernetes.io/serviceaccount";
/// Overrides [`SA_DIR`] for a projected token mounted somewhere else (or a local test).
const SA_DIR_ENV: &str = "EVOCLASH_SA_DIR";

#[derive(Clone, Debug)]
pub struct ScaleConfig {
    /// StatefulSet whose pods are the matches. Pod names are `<statefulset>-<ordinal>`.
    pub statefulset: String,
    pub namespace: String,
    /// Empty matches kept on standby so a join never waits for a pod to start.
    pub buffer: u32,
    pub min: u32,
    pub max: u32,
}

impl ScaleConfig {
    pub fn lease_name(&self) -> String {
        format!("{}-scaler", self.statefulset)
    }
}

/// Start the scaler loop. Without a service account (local runs, `make run`) it stays off.
pub fn start(registry: Arc<Mutex<Registry>>, cfg: ScaleConfig) {
    tokio::spawn(async move {
        let api = match Api::from_env(&cfg) {
            Ok(api) => api,
            Err(e) => {
                eprintln!("k8s scale: disabled: {e}");
                return;
            }
        };
        println!(
            "k8s scale: StatefulSet {} in namespace {}, buffer {}, {}..={}",
            cfg.statefulset, cfg.namespace, cfg.buffer, cfg.min, cfg.max
        );
        let mut tick = tokio::time::interval(TICK);
        let mut leader: Option<Instant> = None;
        let identity = api.identity.clone();
        loop {
            tick.tick().await;
            let (api, cfg, registry) = (api.clone(), cfg.clone(), registry.clone());
            let allow_shrink = leader.is_some_and(|since| since.elapsed() >= STARTUP_GRACE);
            let outcome =
                tokio::task::spawn_blocking(move || step(&api, &cfg, &registry, allow_shrink))
                    .await;
            match outcome {
                Ok(Ok(Step::Standby)) => {
                    if leader.take().is_some() {
                        println!("k8s scale: not the leader any more");
                    }
                }
                Ok(Ok(Step::Leader { note })) => {
                    if leader.is_none() {
                        println!("k8s scale: leading (holder {identity})");
                        leader = Some(Instant::now());
                    }
                    if let Some(note) = note {
                        println!("k8s scale: {note}");
                    }
                }
                Ok(Err(e)) => eprintln!("k8s scale: {e}"),
                Err(e) => eprintln!("k8s scale: task failed: {e}"),
            }
        }
    });
}

enum Step {
    Standby,
    Leader { note: Option<String> },
}

/// One blocking tick: renew the Lease, read the replica count, act on the plan.
fn step(
    api: &Api,
    cfg: &ScaleConfig,
    registry: &Mutex<Registry>,
    allow_shrink: bool,
) -> Result<Step, String> {
    if !api.renew_lease(cfg)? {
        return Ok(Step::Standby);
    }
    let current = api.replicas(cfg)?;
    let slots = slots(registry, &cfg.statefulset)?;
    let note = match plan::plan(current, cfg, allow_shrink, &slots) {
        Plan::Hold => None,
        Plan::Drain(ids) => {
            let changed = registry
                .lock()
                .map_err(|_| "registry lock poisoned".to_string())?
                .drain_matches(&ids);
            Some(format!(
                "draining {changed} empty match(es) before shrinking from {current}: {}",
                ids.join(", ")
            ))
        }
        Plan::Resize(replicas) => {
            api.set_replicas(cfg, replicas)?;
            Some(format!(
                "scaled {} from {current} to {replicas}",
                cfg.statefulset
            ))
        }
    };
    Ok(Step::Leader { note })
}

fn slots(registry: &Mutex<Registry>, statefulset: &str) -> Result<Vec<Slot>, String> {
    let mut registry = registry
        .lock()
        .map_err(|_| "registry lock poisoned".to_string())?;
    Ok(registry
        .scaler_slots()
        .into_iter()
        .map(|view| Slot {
            ordinal: ordinal_of(&view.id, statefulset),
            id: view.id,
            in_use: view.in_use,
            draining: view.draining,
        })
        .collect())
}

/// Service-account namespace, for when `--namespace` is not given.
pub fn namespace_from_env() -> String {
    if let Ok(namespace) = std::env::var("POD_NAMESPACE")
        && !namespace.trim().is_empty()
    {
        return namespace.trim().to_owned();
    }
    std::fs::read_to_string(sa_dir().join("namespace"))
        .map(|namespace| namespace.trim().to_owned())
        .ok()
        .filter(|namespace| !namespace.is_empty())
        .unwrap_or_else(|| "default".into())
}

/// `EVOCLASH_SA_DIR`, else the standard in-cluster mount.
fn sa_dir() -> PathBuf {
    std::env::var(SA_DIR_ENV)
        .ok()
        .map(|dir| dir.trim().to_owned())
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(SA_DIR))
}
