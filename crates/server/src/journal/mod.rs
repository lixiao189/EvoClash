//! Room log, match log, retention, and offline `--replay`.
//!
//! Playing is a pure function of `match_seed`, the opening roster, room settings, and the
//! external inputs after that. The match log records those inputs and a digest every 640
//! ticks. Replay rebuilds Playing from the header and checks the digests.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use evoclash_game::evolution::{Evolution, Perk};
use evoclash_game::items::InventoryMove;
use evoclash_game::net::PROTOCOL_ID;
use evoclash_game::protocol::*;
use evoclash_game::rng::mix;
use evoclash_game::shop::{PlayerCommand, ShopItem};
use evoclash_game::sim::Arena;
use lightyear::prelude::PeerId;

use crate::bots::Bot;
use crate::config::ServerConfig;
use crate::connections::Human;
use crate::gameplay::{
    CurrentInput, FIRST_VAULT_TICKS, MATCH_TICKS, SPAWN_SHIELD_TICKS, SimSet, SimTick, match_flow,
    read_human_inputs, reset_match, spawn_combatant,
};
use crate::rng::SimRng;

mod format;
mod parse;
mod record;
mod replay;
#[cfg(test)]
mod tests;

pub use format::*;
use parse::*;
pub use record::*;
pub use replay::*;

/// Sealed logs older than a match length times two are deleted.
pub const RETAIN_SECS: u64 = 16 * 60;
const DIGEST_EVERY: u32 = 640;
const FLUSH_EVERY: u32 = 16;

pub fn build_id() -> &'static str {
    option_env!("EVOCLASH_BUILD")
        .filter(|id| !id.is_empty())
        .unwrap_or("dev")
}

/// External inputs a test or a replay injects for the upcoming tick.
#[derive(Resource, Default)]
pub struct Injected {
    pub inputs: Vec<(u64, PlayerInput)>,
    pub commands: Vec<(u64, PlayerCommand)>,
    pub moves: Vec<(u64, InventoryMove)>,
    pub rooms: Vec<(u64, RoomCommand)>,
    pub connects: Vec<u64>,
    pub disconnects: Vec<u64>,
    pub leaves: Vec<u64>,
    /// `(client_id, arrival order, following, name)`.
    pub spectates: Vec<(u64, u64, u64, String)>,
    pub unspectates: Vec<u64>,
}

/// Lines stamped with the current sim tick when the match log flushes them.
#[derive(Resource, Default)]
pub struct Trace {
    lines: Vec<String>,
}

impl Trace {
    pub fn push(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }
}

/// Override `SystemTime::now` in tests. Production leaves it unset.
#[derive(Resource, Clone, Copy)]
pub struct LogClock(pub SystemTime);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KillLine {
    pub tick: u32,
    pub killer: String,
    pub victim: String,
    pub how: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreLine {
    pub client_id: u64,
    pub score: u32,
    pub kills: u32,
    pub deaths: u32,
    pub xp: u32,
    /// Tick of the last score gain. 0 if they never scored. Earlier ranks higher.
    pub last_score_tick: u32,
}

#[derive(Clone, Debug)]
struct RosterEntry {
    client_id: u64,
    name: String,
    bot: bool,
    color: [f32; 3],
}

#[derive(Clone, Debug)]
struct SpectatorEntry {
    client_id: u64,
    order: u64,
    following: u64,
    name: String,
}

impl RosterEntry {
    fn line(&self) -> String {
        let name = self.name.replace(['\n', '\r'], " ");
        format!(
            "player {} {} {} {} {} {name}",
            self.client_id,
            u8::from(self.bot),
            self.color[0].to_bits(),
            self.color[1].to_bits(),
            self.color[2].to_bits(),
        )
    }
}

pub struct JournalPlugin;

impl Plugin for JournalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Trace>().init_resource::<Injected>();
        // The multi-thread executor runs conflicting systems out of topological order, so two
        // passes of the same inputs (the live match and its replay) can diverge. One thread
        // keeps FixedUpdate on the schedule's fixed order.
        app.edit_schedule(FixedUpdate, |schedule| {
            schedule.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
        });
        app.add_systems(Startup, open_logs);
        app.add_systems(
            FixedUpdate,
            (
                apply_presence
                    .in_set(SimSet::Input)
                    .before(read_human_inputs),
                log_inputs.in_set(SimSet::Input).after(read_human_inputs),
                journal_flow
                    .in_set(SimSet::Flow)
                    .after(match_flow)
                    .before(reset_match),
            ),
        );
    }
}
