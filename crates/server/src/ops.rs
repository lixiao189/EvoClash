//! Operational plumbing: heartbeats to the matchmaker(s), drain requests, signals and tick timing.

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use evoclash_game::api::Heartbeat;

use crate::config::ServerConfig;
use crate::connections::Human;

const HEARTBEAT_EVERY: Duration = Duration::from_secs(2);

#[derive(Resource, Clone)]
pub struct OpsState {
    draining: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Heartbeat>>,
}

impl OpsState {
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Relaxed)
    }

    fn start_draining(&self, why: &str) {
        if !self.draining.swap(true, Ordering::Relaxed) {
            info!("{why}: finishing the current match, then exiting");
        }
    }
}

#[derive(Resource, Default)]
struct TickTimer {
    started: Option<Instant>,
    ema_us: f32,
}

pub struct OpsPlugin;

impl Plugin for OpsPlugin {
    fn build(&self, app: &mut App) {
        let config = app.world().resource::<ServerConfig>().clone();
        let ops = OpsState {
            draining: Arc::default(),
            snapshot: Arc::new(Mutex::new(Heartbeat {
                match_id: config.match_id.clone(),
                public_addr: config.public_addr.to_string(),
                players: 0,
                max_players: config.max_players as u32,
                accepting: true,
                tick_us: 0,
            })),
        };
        for url in &config.matchmakers {
            spawn_heartbeat_thread(url.clone(), ops.clone());
        }
        install_signal_handler(ops.clone());
        app.insert_resource(ops);
        app.init_resource::<TickTimer>();
        app.add_systems(FixedFirst, |mut timer: ResMut<TickTimer>| {
            timer.started = Some(Instant::now());
        });
        app.add_systems(FixedLast, finish_tick_timer);
        app.add_systems(Update, (update_snapshot, exit_when_drained_and_empty));
    }
}

/// SIGTERM / Ctrl-C starts a drain so rolling deploys never cut a match short; a second
/// signal exits immediately for operators who really mean it.
fn install_signal_handler(ops: OpsState) {
    let signals = AtomicU32::new(0);
    let result = ctrlc::set_handler(move || {
        if signals.fetch_add(1, Ordering::Relaxed) == 0 {
            ops.start_draining("termination signal");
        } else {
            std::process::exit(130);
        }
    });
    if let Err(e) = result {
        warn!("cannot install signal handler: {e}");
    }
}

/// A draining server with nobody in it (or holding a reconnect slot) has nothing to finish.
fn exit_when_drained_and_empty(
    ops: Res<OpsState>,
    humans: Query<(), With<Human>>,
    mut exit: MessageWriter<AppExit>,
) {
    if ops.is_draining() && humans.is_empty() {
        info!("drained with no players left, shutting down");
        exit.write(AppExit::Success);
    }
}

fn finish_tick_timer(mut timer: ResMut<TickTimer>) {
    if let Some(started) = timer.started.take() {
        let us = started.elapsed().as_micros() as f32;
        timer.ema_us = timer.ema_us * 0.98 + us * 0.02;
    }
}

fn update_snapshot(
    ops: Res<OpsState>,
    timer: Res<TickTimer>,
    humans: Query<&Human>,
    config: Res<ServerConfig>,
) {
    let players = humans.iter().count() as u32;
    let Ok(mut snapshot) = ops.snapshot.lock() else {
        return;
    };
    snapshot.players = players;
    snapshot.accepting = !ops.is_draining() && (players as usize) < config.max_players;
    snapshot.tick_us = timer.ema_us as u32;
}

/// Every endpoint behind a matchmaker URL. A plain `http://name:port` URL is expanded to each
/// address the name resolves to, re-resolved every round: on Kubernetes a headless Service
/// lists all matchmaker pods, so each replica gets every heartbeat and scaling just works.
/// Anything else (https, unresolvable for now) is used as written.
fn heartbeat_targets(base_url: &str) -> Vec<String> {
    const PATH: &str = "/v1/servers/heartbeat";
    let base = base_url.trim_end_matches('/');
    let resolved: Option<Vec<SocketAddr>> = base
        .strip_prefix("http://")
        .filter(|authority| !authority.contains('/'))
        .and_then(|authority| authority.to_socket_addrs().ok())
        .map(|addrs| addrs.collect());
    match resolved {
        Some(mut addrs) if !addrs.is_empty() => {
            addrs.sort();
            addrs.dedup();
            addrs
                .into_iter()
                .map(|addr| format!("http://{addr}{PATH}"))
                .collect()
        }
        _ => vec![format!("{base}{PATH}")],
    }
}

/// Blocking HTTP on its own thread so a slow matchmaker can never stall the simulation.
fn spawn_heartbeat_thread(base_url: String, ops: OpsState) {
    std::thread::Builder::new()
        .name("heartbeat".into())
        .spawn(move || {
            // Short timeout: one dead replica must not delay the beats to the others.
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(1)))
                .build()
                .into();
            let mut failures = 0u32;
            loop {
                let beat = ops.snapshot.lock().map(|s| s.clone()).ok();
                if let Some(beat) = beat {
                    let mut error = None;
                    for url in heartbeat_targets(&base_url) {
                        match agent.post(&url).send_json(&beat) {
                            Ok(mut response) => {
                                let drain = response
                                    .body_mut()
                                    .read_json::<serde_json::Value>()
                                    .ok()
                                    .and_then(|v| v.get("drain").and_then(|d| d.as_bool()))
                                    .unwrap_or(false);
                                if drain {
                                    ops.start_draining("matchmaker requested drain");
                                }
                            }
                            Err(e) => error = Some(format!("{url}: {e}")),
                        }
                    }
                    match error {
                        None => failures = 0,
                        Some(e) => {
                            failures += 1;
                            // The match keeps running without the matchmaker; just log sparingly.
                            if failures == 1 || failures.is_multiple_of(30) {
                                warn!("heartbeat failed ({failures}x): {e}");
                            }
                        }
                    }
                }
                std::thread::sleep(HEARTBEAT_EVERY);
            }
        })
        .expect("failed to spawn heartbeat thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_targets_expand_hostnames_to_every_address() {
        assert_eq!(
            heartbeat_targets("http://127.0.0.1:8080/"),
            ["http://127.0.0.1:8080/v1/servers/heartbeat"]
        );
        assert_eq!(
            heartbeat_targets("https://mm.example.com"),
            ["https://mm.example.com/v1/servers/heartbeat"]
        );
        assert!(
            heartbeat_targets("http://localhost:8080")
                .iter()
                .all(
                    |url| url.ends_with(":8080/v1/servers/heartbeat") && !url.contains("localhost")
                )
        );
    }
}
