//! Operational plumbing: heartbeats to the matchmaker, drain requests and tick timing.

use std::sync::atomic::{AtomicBool, Ordering};
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
        if let Some(url) = config.matchmaker.clone() {
            spawn_heartbeat_thread(url, ops.clone());
        }
        app.insert_resource(ops);
        app.init_resource::<TickTimer>();
        app.add_systems(FixedFirst, |mut timer: ResMut<TickTimer>| {
            timer.started = Some(Instant::now());
        });
        app.add_systems(FixedLast, finish_tick_timer);
        app.add_systems(Update, update_snapshot);
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

/// Blocking HTTP on its own thread so a slow matchmaker can never stall the simulation.
fn spawn_heartbeat_thread(base_url: String, ops: OpsState) {
    let url = format!("{}/v1/servers/heartbeat", base_url.trim_end_matches('/'));
    std::thread::Builder::new()
        .name("heartbeat".into())
        .spawn(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(2)))
                .build()
                .into();
            let mut failures = 0u32;
            loop {
                let beat = ops.snapshot.lock().map(|s| s.clone()).ok();
                if let Some(beat) = beat {
                    match agent.post(&url).send_json(&beat) {
                        Ok(mut response) => {
                            failures = 0;
                            let drain = response
                                .body_mut()
                                .read_json::<serde_json::Value>()
                                .ok()
                                .and_then(|v| v.get("drain").and_then(|d| d.as_bool()))
                                .unwrap_or(false);
                            if drain && !ops.is_draining() {
                                info!("matchmaker requested drain: finishing the current match");
                                ops.draining.store(true, Ordering::Relaxed);
                            }
                        }
                        Err(e) => {
                            failures += 1;
                            // The match keeps running without the matchmaker; just log sparingly.
                            if failures == 1 || failures.is_multiple_of(30) {
                                warn!("heartbeat to {url} failed ({failures}x): {e}");
                            }
                        }
                    }
                }
                std::thread::sleep(HEARTBEAT_EVERY);
            }
        })
        .expect("failed to spawn heartbeat thread");
}
