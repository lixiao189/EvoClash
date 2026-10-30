//! Live view of the match fleet, rebuilt continuously from server heartbeats.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use evoclash_game::api::Heartbeat;
use evoclash_game::protocol::MAX_SPECTATORS;
use serde::Serialize;

use crate::recovery;
use crate::rooms::{self, RoomFailure, RoomSeat};

/// A server that misses heartbeats for this long is considered dead and gets no new players.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(6);
/// Freshly launched servers get this long to send their first heartbeat.
pub const STARTUP_GRACE: Duration = Duration::from_secs(10);
/// Seats handed out but not yet visible in a heartbeat are held this long.
const RESERVATION_TTL: Duration = Duration::from_secs(15);
/// A match whose average tick takes longer than this (about 60% of the 15.6 ms budget) is on a
/// starved host; adding players there would show up as lag, so new players go elsewhere.
pub const OVERLOADED_TICK_US: u32 = 9_400;

#[derive(Debug)]
pub struct MatchInfo {
    pub id: String,
    pub addr: SocketAddr,
    /// WebTransport certificate hash; unknown until the server's first heartbeat.
    pub cert_digest: Option<String>,
    pub players: u32,
    pub max_players: u32,
    pub accepting: bool,
    pub tick_us: u32,
    pub draining: bool,
    pub launched_at: Instant,
    pub last_heartbeat: Option<Instant>,
    /// Join tokens issued recently, so concurrent joins don't overfill a match.
    reservations: Vec<Instant>,
    pub phase: String,
    pub phase_left_secs: Option<u32>,
    pub humans: u32,
    pub bots: u32,
    /// Connected watchers, from the heartbeat. Not part of `players`.
    pub spectators: u32,
    pub is_private: bool,
    pub room_code: Option<String>,
    pub room_seed: u64,
    pub match_index: u32,
    /// `none` | `lobby` | `spectate`. Trusted from the server; not re-derived here.
    pub join_mode: String,
    pub exiting: bool,
    /// Fast-forward in progress, or the replacement process has not heartbeated ready yet.
    pub catching_up: bool,
    /// Log rejected or the room crashed too many times. Rejoin is 410.
    pub abandoned: bool,
    /// How many replacement processes have been launched. The third crash abandons.
    pub recoveries: u8,
    /// Watcher tokens issued recently, so concurrent joins don't overfill the 4 seats.
    spec_reservations: Vec<Instant>,
}

/// Quick start will not spectate a round whose wait (time left plus results) exceeds this.
pub const QUICK_SPECTATE_MAX_SECS: u32 = 120;

/// What `POST /v1/join` should do with an optional `match_id`.
#[derive(Debug)]
pub enum JoinPlan {
    /// Combat seat when `bool` is false, watcher seat when true.
    Seat(String, SocketAddr, bool),
    Starting {
        retry_after: u32,
    },
    /// Named room cannot be entered. Never falls through to quick start.
    Full,
    /// No match id: pick a lobby, a short spectate, or launch.
    Quick,
}

#[derive(Serialize)]
pub struct MatchView {
    pub id: String,
    pub addr: String,
    pub players: u32,
    pub reserved: usize,
    pub max_players: u32,
    pub healthy: bool,
    pub draining: bool,
    pub overloaded: bool,
    pub tick_us: u32,
    pub spectators: u32,
}

/// A match as the StatefulSet scaler sees it: whether anyone is in it, and whether it already
/// stopped taking new players.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotView {
    pub id: String,
    pub in_use: bool,
    pub draining: bool,
}

impl MatchInfo {
    pub fn new(id: String, addr: SocketAddr, max_players: u32) -> Self {
        Self {
            id,
            addr,
            cert_digest: None,
            players: 0,
            max_players,
            accepting: true,
            tick_us: 0,
            draining: false,
            launched_at: Instant::now(),
            last_heartbeat: None,
            reservations: Vec::new(),
            // A process we just launched boots into a public lobby. Heartbeats overwrite this.
            phase: "lobby".into(),
            phase_left_secs: None,
            humans: 0,
            bots: 0,
            spectators: 0,
            is_private: false,
            room_code: None,
            room_seed: 0,
            match_index: 0,
            join_mode: "lobby".into(),
            exiting: false,
            catching_up: false,
            abandoned: false,
            recoveries: 0,
            spec_reservations: Vec::new(),
        }
    }

    pub fn healthy(&self, now: Instant) -> bool {
        match self.last_heartbeat {
            Some(at) => now.duration_since(at) < HEARTBEAT_TIMEOUT,
            None => now.duration_since(self.launched_at) < STARTUP_GRACE,
        }
    }

    fn free_seats(&mut self, now: Instant) -> u32 {
        self.reservations
            .retain(|at| now.duration_since(*at) < RESERVATION_TTL);
        self.max_players
            .saturating_sub(self.players + self.reservations.len() as u32)
    }

    fn free_spec_seats(&mut self, now: Instant) -> u32 {
        self.spec_reservations
            .retain(|at| now.duration_since(*at) < RESERVATION_TTL);
        (MAX_SPECTATORS as u32)
            .saturating_sub(self.spectators + self.spec_reservations.len() as u32)
    }

    pub fn overloaded(&self) -> bool {
        self.tick_us > OVERLOADED_TICK_US
    }

    fn open_for_quick_start(&self) -> bool {
        // `exiting` is the last heartbeat of a process that is on its way out: a seat there is a
        // token for a match that is already over, and in a StatefulSet its pod is restarting.
        !self.catching_up && !self.abandoned && !self.exiting && self.phase != "recovering"
    }

    fn joinable(&mut self, now: Instant) -> bool {
        self.open_for_quick_start()
            && self.healthy(now)
            && self.accepting
            && !self.draining
            && !self.overloaded()
            && !self.is_private
            && self.join_mode == "lobby"
            && self.free_seats(now) > 0
    }

    /// Public Playing or Results the server marked `spectate`, with a watcher seat left.
    fn spectatable(&mut self, now: Instant) -> bool {
        self.open_for_quick_start()
            && self.healthy(now)
            && !self.draining
            && !self.overloaded()
            && !self.is_private
            && self.join_mode == "spectate"
            && self.free_spec_seats(now) > 0
    }

    /// Joinable and nobody in it or on the way: a warm spare for the next burst of joins.
    fn idle(&mut self, now: Instant) -> bool {
        self.joinable(now) && self.players == 0 && self.reservations.is_empty()
    }

    /// Someone is in the room or on the way. Bots are not people: a warm lobby no human has
    /// entered yet is exactly what scale-down is allowed to reclaim.
    fn in_use(&mut self, now: Instant) -> bool {
        self.free_seats(now);
        self.free_spec_seats(now);
        self.humans > 0
            || self.spectators > 0
            || !self.reservations.is_empty()
            || !self.spec_reservations.is_empty()
    }
}

/// Seconds until this room is a lobby again. Playing adds the results screen.
fn spectate_wait(m: &MatchInfo) -> u32 {
    let left = m.phase_left_secs.unwrap_or(u32::MAX / 4);
    if m.phase == "playing" {
        left.saturating_add(rooms::RESULTS_SECS)
    } else {
        left
    }
}

/// Lower StatefulSet ordinal first, so a room on pod 2 is picked before one on pod 10: the
/// highest ordinals are the ones scale-down can reclaim. Ordinals are compared as numbers
/// (`server-10` is after `server-2`, not before it), and matches that are not StatefulSet pods
/// sort last so they never outrank a reclaimable one.
fn cmp_ordinal(a: Option<u32>, b: Option<u32>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// Larger remaining time first. `None` (countdown not started) sorts last.
fn cmp_time_left(a: Option<u32>, b: Option<u32>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

#[derive(Default)]
pub struct Registry {
    pub matches: HashMap<String, MatchInfo>,
    /// Set when the match servers are pods of a StatefulSet (`--k8s-scale`): lobby picking then
    /// prefers the lowest ordinal, so the highest ones drain and can be scaled away.
    statefulset: Option<String>,
    /// Room codes whose process is gone, kept so a lookup is "closed" rather than a fresh mint.
    released: HashMap<String, Instant>,
    attempts: rooms::Attempts,
}

impl Registry {
    /// The match servers are pods of this StatefulSet (`--k8s-scale`), so their ordinals can
    /// order lobby picking and the scaler's drain list.
    pub fn set_statefulset(&mut self, statefulset: &str) {
        self.statefulset = Some(statefulset.to_owned());
    }

    fn ordinal(&self, id: &str) -> Option<u32> {
        crate::scaler::ordinal_of(id, self.statefulset.as_deref()?)
    }

    /// Ordinal per match id, so a sort closure does not borrow `self` while it indexes matches.
    fn ordinals(&self, ids: &[String]) -> HashMap<String, Option<u32>> {
        ids.iter()
            .map(|id| (id.clone(), self.ordinal(id)))
            .collect()
    }

    /// Quick start. Open lobbies first (more humans, then more time left). If none are
    /// taking combatants, a public `spectate` room whose wait is at most two minutes,
    /// shortest wait first (time left, plus the results screen while the round is still on).
    /// `bool` is true when the seat is a watcher. `None` means launch a new room.
    pub fn reserve_seat(&mut self) -> Option<(String, SocketAddr, bool)> {
        let now = Instant::now();
        if let Some(id) = self.pick_lobby(now) {
            let m = self.matches.get_mut(&id)?;
            m.reservations.push(now);
            return Some((m.id.clone(), m.addr, false));
        }
        self.take_spectate(now, Some(QUICK_SPECTATE_MAX_SECS))
    }

    /// Shortest public spectate seat, ignoring the two-minute cap. Used when a launch fails.
    pub fn reserve_any_spectate(&mut self) -> Option<(String, SocketAddr, bool)> {
        self.take_spectate(Instant::now(), None)
    }

    fn take_spectate(
        &mut self,
        now: Instant,
        max_wait: Option<u32>,
    ) -> Option<(String, SocketAddr, bool)> {
        let id = self.pick_spectate(now, max_wait)?;
        let m = self.matches.get_mut(&id)?;
        m.spec_reservations.push(now);
        Some((m.id.clone(), m.addr, true))
    }

    fn pick_lobby(&mut self, now: Instant) -> Option<String> {
        let mut ids: Vec<String> = self.matches.keys().cloned().collect();
        let ordinals = self.ordinals(&ids);
        ids.sort_by(|a, b| {
            let (left_a, humans_a) = {
                let m = &self.matches[a];
                (m.phase_left_secs, m.humans)
            };
            let (left_b, humans_b) = {
                let m = &self.matches[b];
                (m.phase_left_secs, m.humans)
            };
            humans_b
                .cmp(&humans_a)
                .then_with(|| cmp_time_left(left_a, left_b))
                .then_with(|| cmp_ordinal(ordinals[a], ordinals[b]))
                .then_with(|| a.cmp(b))
        });
        ids.into_iter()
            .find(|id| self.matches.get_mut(id).is_some_and(|m| m.joinable(now)))
    }

    fn pick_spectate(&mut self, now: Instant, max_wait: Option<u32>) -> Option<String> {
        let mut ids: Vec<String> = self.matches.keys().cloned().collect();
        let ordinals = self.ordinals(&ids);
        ids.sort_by(|a, b| {
            let wait = |id: &String| spectate_wait(&self.matches[id]);
            wait(a)
                .cmp(&wait(b))
                .then_with(|| cmp_ordinal(ordinals[a], ordinals[b]))
                .then_with(|| a.cmp(b))
        });
        ids.into_iter().find(|id| {
            let within = max_wait.is_none_or(|cap| spectate_wait(&self.matches[id]) <= cap);
            within && self.matches.get_mut(id).is_some_and(|m| m.spectatable(now))
        })
    }

    /// A named room from the list. An empty id is quick start. Anything that is not a
    /// joinable public lobby, a free spectate seat, or the starting countdown is [`JoinPlan::Full`].
    pub fn plan_join(&mut self, match_id: Option<&str>) -> JoinPlan {
        let now = Instant::now();
        let Some(id) = match_id.map(str::trim).filter(|id| !id.is_empty()) else {
            return JoinPlan::Quick;
        };
        let Some(m) = self.matches.get_mut(id) else {
            return JoinPlan::Full;
        };
        if m.is_private {
            return JoinPlan::Full;
        }
        if m.joinable(now) {
            let addr = m.addr;
            m.reservations.push(now);
            return JoinPlan::Seat(id.to_string(), addr, false);
        }
        let retry_after = m.phase_left_secs.unwrap_or(0);
        // Starting, or a lobby the countdown has closed (under 8s) that still has a seat.
        // A full lobby has no free seat and is `Full`, not this.
        let late_lobby = m.phase == "lobby" && m.join_mode != "lobby" && m.free_seats(now) > 0;
        if m.phase == "starting" || late_lobby {
            return JoinPlan::Starting { retry_after };
        }
        if m.spectatable(now) {
            let addr = m.addr;
            m.spec_reservations.push(now);
            return JoinPlan::Seat(id.to_string(), addr, true);
        }
        JoinPlan::Full
    }

    /// Public rooms a human is in or about to be in. Empty warm servers are left out.
    /// Order matches [`Self::pick_lobby`]: more humans, then more time left, then ordinal, then id.
    pub fn room_list(&mut self) -> evoclash_game::api::RoomList {
        let now = Instant::now();
        let mut ids: Vec<String> = self.matches.keys().cloned().collect();
        let ordinals = self.ordinals(&ids);
        ids.sort_by(|a, b| {
            let (left_a, humans_a) = {
                let m = &self.matches[a];
                (m.phase_left_secs, m.humans)
            };
            let (left_b, humans_b) = {
                let m = &self.matches[b];
                (m.phase_left_secs, m.humans)
            };
            humans_b
                .cmp(&humans_a)
                .then_with(|| cmp_time_left(left_a, left_b))
                .then_with(|| cmp_ordinal(ordinals[a], ordinals[b]))
                .then_with(|| a.cmp(b))
        });
        let mut rooms = Vec::new();
        for id in ids {
            let Some(summary) = self.room_summary(&id, now) else {
                continue;
            };
            rooms.push(summary);
        }
        evoclash_game::api::RoomList { rooms }
    }

    fn room_summary(&mut self, id: &str, now: Instant) -> Option<evoclash_game::api::RoomSummary> {
        let m = self.matches.get_mut(id)?;
        if m.is_private
            || !m.healthy(now)
            || m.abandoned
            || m.catching_up
            || m.phase == "recovering"
        {
            return None;
        }
        m.free_seats(now);
        m.free_spec_seats(now);
        let reserved = (m.reservations.len() + m.spec_reservations.len()) as u32;
        if m.humans + m.spectators + reserved == 0 {
            return None;
        }
        let entry = if m.joinable(now) {
            "play"
        } else if m.spectatable(now) {
            "spectate"
        } else {
            "full"
        };
        Some(evoclash_game::api::RoomSummary {
            match_id: m.id.clone(),
            phase: m.phase.clone(),
            phase_left_secs: m.phase_left_secs,
            humans: m.humans,
            bots: m.bots,
            spectators: m.spectators,
            max_players: m.max_players,
            max_spectators: u32::from(MAX_SPECTATORS),
            entry: entry.into(),
        })
    }

    /// Take one public lobby with nobody in it and no live reservation. Reserves a combat seat.
    /// Used when this replica cannot launch (`--no-launch`).
    pub fn claim_idle_public(&mut self) -> Option<(String, SocketAddr)> {
        let now = Instant::now();
        let mut ids: Vec<String> = self.matches.keys().cloned().collect();
        let ordinals = self.ordinals(&ids);
        ids.sort_by(|a, b| cmp_ordinal(ordinals[a], ordinals[b]).then_with(|| a.cmp(b)));
        let id = ids.into_iter().find(|id| {
            self.matches.get_mut(id).is_some_and(|m| {
                m.spectators == 0
                    && m.free_spec_seats(now) == u32::from(MAX_SPECTATORS)
                    && m.idle(now)
            })
        })?;
        let m = self.matches.get_mut(&id)?;
        let addr = m.addr;
        m.reservations.push(now);
        Some((id, addr))
    }

    pub fn reserve_in(&mut self, id: &str) -> Option<SocketAddr> {
        let m = self.matches.get_mut(id)?;
        m.reservations.push(Instant::now());
        Some(m.addr)
    }

    /// Record a heartbeat. Returns whether the server should drain.
    pub fn heartbeat(&mut self, beat: &Heartbeat) -> bool {
        if self
            .matches
            .get(&beat.match_id)
            .is_some_and(|m| m.abandoned)
        {
            return false;
        }
        let now = Instant::now();
        let entry = self
            .matches
            .entry(beat.match_id.clone())
            .or_insert_with(|| {
                // A server we didn't launch (or launched before a restart): adopt it.
                let addr = beat
                    .public_addr
                    .parse()
                    .unwrap_or(SocketAddr::from(([127, 0, 0, 1], 0)));
                MatchInfo::new(beat.match_id.clone(), addr, beat.max_players)
            });
        // Players that show up in the heartbeat no longer need their reservation.
        let arrived = beat.players.saturating_sub(entry.players) as usize;
        let drop = arrived.min(entry.reservations.len());
        entry.reservations.drain(..drop);
        let spec_arrived = beat.spectators.saturating_sub(entry.spectators) as usize;
        let spec_drop = spec_arrived.min(entry.spec_reservations.len());
        entry.spec_reservations.drain(..spec_drop);
        entry.cert_digest = Some(beat.cert_digest.clone());
        entry.players = beat.players;
        entry.spectators = beat.spectators;
        entry.max_players = beat.max_players;
        entry.accepting = beat.accepting;
        entry.tick_us = beat.tick_us;
        entry.phase = beat.phase.clone();
        entry.phase_left_secs = beat.phase_left_secs;
        entry.humans = beat.humans;
        entry.bots = beat.bots;
        entry.is_private = beat.is_private;
        entry.room_code = beat.room_code.clone();
        entry.room_seed = beat.room_seed;
        entry.match_index = beat.match_index;
        entry.join_mode = beat.join_mode.clone();
        entry.exiting = beat.exiting;
        entry.last_heartbeat = Some(now);
        if entry.catching_up {
            // The replacement process bound a new port.
            if let Ok(addr) = beat.public_addr.parse() {
                entry.addr = addr;
            }
            if recovery::catchup_done(&entry.phase, entry.phase_left_secs) {
                entry.catching_up = false;
            }
        }
        entry.draining
    }

    /// Forget matches that stopped heartbeating. Returns their ids.
    /// A private code is held so the next lookup is "closed", not a new room.
    pub fn reap(&mut self) -> Vec<String> {
        let now = Instant::now();
        let dead: Vec<String> = self
            .matches
            .values()
            .filter(|m| !m.healthy(now))
            .map(|m| m.id.clone())
            .collect();
        for id in &dead {
            self.forget_at(id, now);
        }
        dead
    }

    /// Drop one match and hold its room code. Missing ids are fine (already reaped).
    pub fn forget(&mut self, id: &str) {
        self.forget_at(id, Instant::now());
    }

    fn forget_at(&mut self, id: &str, now: Instant) {
        if let Some(m) = self.matches.remove(id)
            && let Some(code) = m.room_code
        {
            self.released.insert(code, now);
        }
    }

    fn purge_released(&mut self, now: Instant) {
        self.released
            .retain(|_, at| now.duration_since(*at) < rooms::HOLD);
    }

    fn code_taken(&self, code: &str, now: Instant) -> bool {
        let held = self
            .released
            .get(code)
            .is_some_and(|at| now.duration_since(*at) < rooms::HOLD);
        held || self
            .matches
            .values()
            .any(|m| m.room_code.as_deref() == Some(code))
    }

    fn pick_code(&mut self, now: Instant, mut draw: impl FnMut() -> String) -> Option<String> {
        self.purge_released(now);
        for _ in 0..64 {
            let code = draw();
            if !self.code_taken(&code, now) {
                return Some(code);
            }
        }
        None
    }

    /// A fresh 4-character code, skipping anything in use or released under 10 minutes ago.
    pub fn mint_code(&mut self) -> Option<String> {
        let now = Instant::now();
        self.pick_code(now, rooms::random_code)
    }

    /// Shared budget for room lookups and creates. A format failure must not call this.
    pub fn allow_ip(&mut self, ip: &str, now: Instant) -> bool {
        self.attempts.allow(ip, now)
    }

    /// Join a private room by code. Does not fall through to quick start.
    pub fn lookup_room(
        &mut self,
        raw: &str,
        ip: &str,
        now: Instant,
    ) -> Result<RoomSeat, RoomFailure> {
        let Some(code) = rooms::normalize(raw) else {
            return Err(RoomFailure::new(400, "bad_code", rooms::BAD_CODE, None));
        };
        if !self.attempts.allow(ip, now) {
            return Err(RoomFailure::rate());
        }
        self.purge_released(now);
        let found = self
            .matches
            .iter()
            .find(|(_, m)| m.room_code.as_deref() == Some(code.as_str()))
            .map(|(id, m)| {
                (
                    id.clone(),
                    m.addr,
                    m.phase.clone(),
                    m.phase_left_secs,
                    m.join_mode.clone(),
                    m.humans,
                    m.healthy(now),
                    m.accepting,
                    m.draining,
                    m.overloaded(),
                    m.catching_up,
                )
            });
        let Some((
            id,
            addr,
            phase,
            left,
            join_mode,
            humans,
            healthy,
            accepting,
            draining,
            overloaded,
            catching_up,
        )) = found
        else {
            return Err(if self.released.contains_key(&code) {
                RoomFailure::new(410, "closed", rooms::CLOSED, None)
            } else {
                RoomFailure::new(404, "not_found", rooms::not_found_message(&code), None)
            });
        };
        if catching_up || phase == "recovering" {
            return Err(RoomFailure::new(
                503,
                "recovering",
                recovery::RECOVERING_MSG,
                Some(recovery::RETRY_AFTER_SECS),
            ));
        }
        if phase == "playing" || phase == "results" {
            let retry_after = if phase == "playing" {
                left.unwrap_or(0).saturating_add(rooms::RESULTS_SECS)
            } else {
                left.unwrap_or(0)
            };
            let open = join_mode == "spectate" && healthy && !draining && !overloaded;
            if open {
                let m = self.matches.get_mut(&id).expect("match");
                if m.free_spec_seats(now) > 0 {
                    m.spec_reservations.push(now);
                    return Ok(RoomSeat {
                        id,
                        addr,
                        code,
                        spectate: true,
                    });
                }
                let error = if phase == "playing" {
                    "in_match"
                } else {
                    "results"
                };
                return Err(RoomFailure::new(
                    409,
                    error,
                    rooms::spectate_full_message(retry_after),
                    Some(retry_after),
                ));
            }
            if phase == "playing" {
                return Err(RoomFailure::new(
                    409,
                    "in_match",
                    rooms::in_match_message(retry_after),
                    Some(retry_after),
                ));
            }
            return Err(RoomFailure::new(
                409,
                "results",
                rooms::RESULTS,
                Some(retry_after),
            ));
        }
        // Host already started: lobby countdown or the 3s Starting phase.
        let countdown =
            phase == "starting" || (phase == "lobby" && left.is_some() && join_mode != "lobby");
        if countdown {
            return Err(RoomFailure::new(
                409,
                "starting",
                rooms::STARTING,
                Some(left.unwrap_or(0)),
            ));
        }
        if humans >= 8 {
            return Err(RoomFailure::new(409, "full", rooms::FULL, None));
        }
        if healthy
            && accepting
            && !draining
            && !overloaded
            && join_mode == "lobby"
            && phase == "lobby"
        {
            let m = self.matches.get_mut(&id).expect("match");
            if m.free_seats(now) == 0 {
                return Err(RoomFailure::new(409, "full", rooms::FULL, None));
            }
            m.reservations.push(now);
            return Ok(RoomSeat {
                id,
                addr,
                code,
                spectate: false,
            });
        }
        if !healthy {
            return Err(RoomFailure::new(410, "closed", rooms::CLOSED, None));
        }
        Err(RoomFailure::new(
            503,
            "busy",
            "服务器繁忙，请稍后再试。",
            None,
        ))
    }

    /// Non-zero exit or a signal. `Some(0)` and `exiting` are a normal shutdown.
    /// Exit code [`recovery::ABANDON_EXIT`] gives up immediately.
    pub fn on_process_exit(&mut self, id: &str, code: Option<i32>) -> recovery::Step {
        let Some((abandoned, exiting)) = self.matches.get(id).map(|m| (m.abandoned, m.exiting))
        else {
            return recovery::Step::None;
        };
        if abandoned {
            return recovery::Step::None;
        }
        if exiting || code == Some(0) {
            self.forget(id);
            return recovery::Step::Dropped(id.to_string());
        }
        if code == Some(recovery::ABANDON_EXIT) {
            return self.abandon(id);
        }
        self.begin_crash(id)
    }

    /// Heartbeat gap. A process already marked `exiting` is removed. Others crash.
    pub fn poll_crashes(&mut self, now: Instant) -> Vec<recovery::Step> {
        let ids: Vec<String> = self.matches.keys().cloned().collect();
        let mut steps = Vec::new();
        for id in ids {
            let Some((abandoned, healthy, exiting)) = self
                .matches
                .get(&id)
                .map(|m| (m.abandoned, m.healthy(now), m.exiting))
            else {
                continue;
            };
            if abandoned || healthy {
                continue;
            }
            if exiting {
                self.forget(&id);
                steps.push(recovery::Step::Dropped(id));
                continue;
            }
            steps.push(self.begin_crash(&id));
        }
        steps
    }

    fn begin_crash(&mut self, id: &str) -> recovery::Step {
        let Some((abandoned, recoveries)) =
            self.matches.get(id).map(|m| (m.abandoned, m.recoveries))
        else {
            return recovery::Step::None;
        };
        if abandoned {
            return recovery::Step::None;
        }
        if recoveries >= recovery::MAX_RECOVERIES {
            return self.abandon(id);
        }
        let Some(m) = self.matches.get_mut(id) else {
            return recovery::Step::None;
        };
        m.recoveries = m.recoveries.saturating_add(1);
        m.catching_up = true;
        m.phase = "recovering".into();
        m.join_mode = "none".into();
        m.phase_left_secs = None;
        m.accepting = false;
        m.cert_digest = None;
        m.last_heartbeat = None;
        m.launched_at = Instant::now();
        let launch = recovery::Launch {
            id: m.id.clone(),
            room_seed: m.room_seed,
            room_code: m.room_code.clone(),
        };
        recovery::Step::Relaunch(launch)
    }

    fn abandon(&mut self, id: &str) -> recovery::Step {
        if let Some(m) = self.matches.get_mut(id) {
            m.abandoned = true;
            m.catching_up = false;
            m.accepting = false;
            m.join_mode = "none".into();
            if let Some(code) = m.room_code.take() {
                self.released.insert(code, Instant::now());
            }
        }
        recovery::Step::Abandoned(id.to_string())
    }

    /// What `/v1/rejoin` should do before it tries to mint a token.
    pub fn rejoin_gate(&self, id: &str) -> recovery::RejoinGate {
        let Some(m) = self.matches.get(id) else {
            return recovery::RejoinGate::Gone;
        };
        if m.abandoned {
            return recovery::RejoinGate::Abandoned;
        }
        if m.catching_up || !m.healthy(Instant::now()) {
            return recovery::RejoinGate::Recovering;
        }
        recovery::RejoinGate::Ready
    }

    pub fn views(&mut self) -> Vec<MatchView> {
        let now = Instant::now();
        let mut views: Vec<MatchView> = self
            .matches
            .values_mut()
            .map(|m| {
                m.free_seats(now);
                MatchView {
                    id: m.id.clone(),
                    addr: m.addr.to_string(),
                    players: m.players,
                    reserved: m.reservations.len(),
                    max_players: m.max_players,
                    healthy: m.healthy(now),
                    draining: m.draining,
                    overloaded: m.overloaded(),
                    tick_us: m.tick_us,
                    spectators: m.spectators,
                }
            })
            .collect();
        views.sort_by(|a, b| a.id.cmp(&b.id));
        views
    }

    pub fn idle_matches(&mut self) -> usize {
        let now = Instant::now();
        self.matches
            .values_mut()
            .map(|m| m.idle(now))
            .filter(|idle| *idle)
            .count()
    }

    pub fn used_addrs(&self) -> Vec<SocketAddr> {
        self.matches.values().map(|m| m.addr).collect()
    }

    /// Every match, for the StatefulSet scaler: who is in it and whether it is on its way out.
    pub fn scaler_slots(&mut self) -> Vec<SlotView> {
        let now = Instant::now();
        let slots: Vec<SlotView> = self
            .matches
            .values_mut()
            .map(|m| {
                let in_use = m.in_use(now);
                SlotView {
                    id: m.id.clone(),
                    in_use,
                    draining: m.draining,
                }
            })
            .collect();
        // Ordinal order, so the scaler's drain list runs lowest pod to highest, numerically.
        let mut keyed: Vec<(Option<u32>, SlotView)> = slots
            .into_iter()
            .map(|slot| (self.ordinal(&slot.id), slot))
            .collect();
        keyed.sort_by(|(left_ord, left), (right_ord, right)| {
            cmp_ordinal(*left_ord, *right_ord).then_with(|| left.id.cmp(&right.id))
        });
        keyed.into_iter().map(|(_, slot)| slot).collect()
    }

    /// Stop handing new players to these matches: their pods are about to be scaled away.
    /// Returns how many entries changed.
    pub fn drain_matches(&mut self, ids: &[String]) -> usize {
        let mut changed = 0;
        for id in ids {
            if let Some(m) = self.matches.get_mut(id)
                && !m.draining
            {
                m.draining = true;
                changed += 1;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beat(id: &str, players: u32) -> Heartbeat {
        Heartbeat {
            match_id: id.into(),
            public_addr: "127.0.0.1:7000".into(),
            cert_digest: "00".repeat(32),
            players,
            max_players: 2,
            accepting: true,
            tick_us: 100,
            phase: "lobby".into(),
            phase_left_secs: None,
            humans: players,
            bots: 0,
            spectators: 0,
            is_private: false,
            room_code: None,
            room_seed: 1,
            match_index: 0,
            join_mode: "lobby".into(),
            exiting: false,
        }
    }

    #[test]
    fn reservations_prevent_overfill_and_clear_on_arrival() {
        let mut reg = Registry::default();
        reg.heartbeat(&beat("a", 0));
        assert!(reg.reserve_seat().is_some());
        assert!(reg.reserve_seat().is_some());
        assert!(reg.reserve_seat().is_none(), "two seats only");
        reg.heartbeat(&beat("a", 1));
        assert_eq!(reg.matches["a"].reservations.len(), 1);
    }

    #[test]
    fn prefers_fuller_matches() {
        let mut reg = Registry::default();
        reg.heartbeat(&beat("a", 0));
        let mut b = beat("b", 1);
        b.max_players = 6;
        reg.heartbeat(&b);
        assert_eq!(reg.reserve_seat().unwrap().0, "b");
    }

    #[test]
    fn overloaded_matches_get_no_players() {
        let mut reg = Registry::default();
        let mut hot = beat("a", 0);
        hot.tick_us = OVERLOADED_TICK_US + 1;
        reg.heartbeat(&hot);
        assert!(reg.reserve_seat().is_none());
        reg.heartbeat(&beat("a", 0));
        assert!(
            reg.reserve_seat().is_some(),
            "recovers once ticks are fast again"
        );
    }

    #[test]
    fn idle_counts_only_empty_joinable_matches() {
        let mut reg = Registry::default();
        reg.heartbeat(&beat("a", 0));
        reg.heartbeat(&beat("b", 1));
        reg.heartbeat(&beat("c", 0));
        reg.matches.get_mut("c").unwrap().draining = true;
        assert_eq!(reg.idle_matches(), 1);
        reg.reserve_in("a");
        assert_eq!(reg.idle_matches(), 0, "a reserved seat means it is taken");
    }

    #[test]
    fn quick_start_orders_humans_then_time_left_and_skips_closed_rooms() {
        let mut reg = Registry::default();
        let mut a = beat("a", 1);
        a.phase_left_secs = Some(20);
        a.max_players = 2;
        let mut b = beat("b", 1);
        b.phase_left_secs = Some(12);
        b.max_players = 2;
        let mut c = beat("c", 1);
        c.phase_left_secs = None;
        c.max_players = 2;
        let mut d = beat("d", 2);
        d.humans = 2;
        d.phase_left_secs = Some(9);
        d.max_players = 3;
        let mut closed = beat("e", 3);
        closed.humans = 3;
        closed.join_mode = "none".into();
        closed.phase = "playing".into();
        closed.max_players = 8;
        for beat in [a, b, c, d, closed] {
            reg.heartbeat(&beat);
        }
        assert_eq!(reg.reserve_seat().unwrap().0, "d");
        assert_eq!(reg.reserve_seat().unwrap().0, "a");
        assert_eq!(reg.reserve_seat().unwrap().0, "b");
        assert_eq!(reg.reserve_seat().unwrap().0, "c");
        assert!(
            reg.reserve_seat().is_none(),
            "join_mode none is never a candidate"
        );
    }

    #[test]
    fn named_match_starting_stays_starting_and_a_closed_room_is_full() {
        let mut reg = Registry::default();
        let mut playing = beat("play", 2);
        playing.phase = "playing".into();
        playing.join_mode = "none".into();
        playing.phase_left_secs = Some(100);
        let mut starting = beat("go", 1);
        starting.phase = "starting".into();
        starting.join_mode = "none".into();
        starting.phase_left_secs = Some(2);
        reg.heartbeat(&playing);
        reg.heartbeat(&starting);
        assert!(matches!(reg.plan_join(Some("play")), JoinPlan::Full));
        assert!(matches!(
            reg.plan_join(Some("go")),
            JoinPlan::Starting { retry_after: 2 }
        ));
        assert!(matches!(reg.plan_join(Some("missing")), JoinPlan::Full));
        assert!(matches!(reg.plan_join(None), JoinPlan::Quick));
        assert!(matches!(reg.plan_join(Some("  ")), JoinPlan::Quick));
    }

    #[test]
    fn draining_matches_get_no_players() {
        let mut reg = Registry::default();
        reg.heartbeat(&beat("a", 0));
        reg.matches.get_mut("a").unwrap().draining = true;
        assert!(reg.reserve_seat().is_none());
        assert!(
            reg.heartbeat(&beat("a", 0)),
            "heartbeat reply asks to drain"
        );
    }

    fn private_room(id: &str, code: &str, humans: u32) -> Heartbeat {
        let mut beat = beat(id, humans);
        beat.max_players = 8;
        beat.humans = humans;
        beat.is_private = true;
        beat.room_code = Some(code.into());
        beat
    }

    #[test]
    fn minted_codes_are_four_alphabet_chars() {
        assert_eq!(rooms::ALPHABET.chars().count(), 29);
        assert!(!"0O1ILUV".chars().any(|c| rooms::ALPHABET.contains(c)));
        let mut reg = Registry::default();
        let now = Instant::now();
        for _ in 0..32 {
            let code = reg.mint_code().expect("code");
            assert_eq!(code.chars().count(), 4, "{code}");
            assert!(code.chars().all(|c| rooms::ALPHABET.contains(c)), "{code}");
            reg.released.insert(code, now);
        }
        // A held code is skipped; the next draw is used.
        reg.released.insert("AAAA".into(), now);
        let mut draws = 0;
        let code = reg.pick_code(now, || {
            draws += 1;
            if draws == 1 {
                "AAAA".into()
            } else {
                "BBBB".into()
            }
        });
        assert_eq!(code.as_deref(), Some("BBBB"));
    }

    #[test]
    fn quick_start_never_returns_a_private_match() {
        let mut reg = Registry::default();
        reg.heartbeat(&private_room("priv", "ABCD", 3));
        assert!(reg.reserve_seat().is_none());
        assert!(matches!(reg.plan_join(Some("priv")), JoinPlan::Full));
        reg.heartbeat(&beat("pub", 0));
        assert_eq!(reg.reserve_seat().unwrap().0, "pub");
    }

    #[test]
    fn bad_code_is_400_and_does_not_count_as_a_lookup() {
        let mut reg = Registry::default();
        let now = Instant::now();
        for raw in ["no", "ABC0", "ab1d", "!!!!", ""] {
            let err = reg.lookup_room(raw, "9.9.9.9", now).unwrap_err();
            assert_eq!(err.status, 400, "{raw}");
            assert_eq!(err.error, "bad_code");
            assert_eq!(err.message, rooms::BAD_CODE);
        }
        for _ in 0..11 {
            assert_eq!(
                reg.lookup_room("nope", "9.9.9.9", now).unwrap_err().status,
                400
            );
        }
        let err = reg.lookup_room("ZZZZ", "9.9.9.9", now).unwrap_err();
        assert_eq!(err.status, 404, "format failures are not lookups");
    }

    #[test]
    fn missing_room_is_404_and_closed_room_is_410() {
        let mut reg = Registry::default();
        let now = Instant::now();
        let err = reg.lookup_room("abcd", "1.1.1.1", now).unwrap_err();
        assert_eq!(err.status, 404);
        assert_eq!(err.error, "not_found");
        assert_eq!(
            err.message,
            "找不到房间 ABCD，请检查房间码，或让房主重新发送链接。"
        );
        reg.heartbeat(&private_room("priv", "WXYZ", 0));
        reg.forget("priv");
        let err = reg
            .lookup_room("wxyz", "1.1.1.2", Instant::now())
            .unwrap_err();
        assert_eq!(err.status, 410);
        assert_eq!(err.error, "closed");
        assert_eq!(err.message, rooms::CLOSED);
        assert!(reg.code_taken("WXYZ", Instant::now()));
        let later = Instant::now() + rooms::HOLD;
        let err = reg.lookup_room("wxyz", "1.1.1.3", later).unwrap_err();
        assert_eq!(err.status, 404, "a code can be new again after 10 minutes");
    }

    #[test]
    fn full_private_room_is_409() {
        let mut reg = Registry::default();
        let mut room = private_room("priv", "ABCD", 8);
        room.accepting = false;
        room.join_mode = "none".into();
        room.phase_left_secs = None;
        reg.heartbeat(&room);
        let err = reg
            .lookup_room("ABCD", "1.1.1.1", Instant::now())
            .unwrap_err();
        assert_eq!(err.status, 409);
        assert_eq!(err.error, "full");
        assert_eq!(err.message, rooms::FULL);
    }

    #[test]
    fn playing_private_room_is_in_match_with_results_padding() {
        let mut reg = Registry::default();
        let mut room = private_room("priv", "ABCD", 2);
        room.phase = "playing".into();
        room.join_mode = "none".into();
        room.phase_left_secs = Some(100);
        reg.heartbeat(&room);
        let err = reg
            .lookup_room("abcd", "1.1.1.1", Instant::now())
            .unwrap_err();
        assert_eq!(err.status, 409);
        assert_eq!(err.error, "in_match");
        assert_eq!(err.retry_after, Some(100 + rooms::RESULTS_SECS));
        assert_eq!(
            err.message,
            "房间正在对局，约 1 分 55 秒后结束，结束后自动加入。"
        );
    }

    #[test]
    fn eleventh_lookup_from_one_ip_is_429() {
        let mut reg = Registry::default();
        let now = Instant::now();
        for _ in 0..10 {
            assert_eq!(
                reg.lookup_room("ZZZZ", "8.8.8.8", now).unwrap_err().status,
                404
            );
        }
        let err = reg.lookup_room("YYYY", "8.8.8.8", now).unwrap_err();
        assert_eq!(err.status, 429);
        assert_eq!(err.error, "rate");
        assert_eq!(err.message, rooms::RATE);
        assert_eq!(
            reg.lookup_room("ZZZZ", "7.7.7.7", now).unwrap_err().status,
            404,
            "another IP still has its own budget"
        );
    }

    fn spectate_room(id: &str, phase: &str, left: u32, watchers: u32) -> Heartbeat {
        let mut beat = beat(id, 8);
        beat.max_players = 8;
        beat.humans = 8;
        beat.phase = phase.into();
        beat.phase_left_secs = Some(left);
        beat.join_mode = "spectate".into();
        beat.spectators = watchers;
        beat.accepting = false;
        beat
    }

    #[test]
    fn quick_start_takes_a_lobby_then_the_soonest_spectate_then_nothing() {
        let mut reg = Registry::default();
        reg.heartbeat(&beat("lobby", 1));
        reg.heartbeat(&spectate_room("late", "playing", 100, 0));
        reg.heartbeat(&spectate_room("soon", "results", 5, 0));
        let (id, _, spectate) = reg.reserve_seat().unwrap();
        assert_eq!(id, "lobby");
        assert!(!spectate, "an open lobby beats a watcher seat");
        for _ in 0..MAX_SPECTATORS {
            let (id, _, spectate) = reg.reserve_seat().unwrap();
            assert_eq!(
                id, "soon",
                "results left is shorter than playing plus results"
            );
            assert!(spectate);
        }
        for _ in 0..MAX_SPECTATORS {
            assert_eq!(reg.reserve_seat().unwrap().0, "late");
        }
        assert_eq!(
            reg.reserve_seat().map(|seat| seat.0),
            None,
            "no lobby and no watcher seat: open a room"
        );
    }

    #[test]
    fn spectate_reservations_stop_at_four_and_clear_when_a_watcher_arrives() {
        let mut reg = Registry::default();
        reg.heartbeat(&spectate_room("a", "playing", 40, 0));
        assert!(reg.reserve_seat().unwrap().2);
        assert!(reg.reserve_seat().is_some());
        let arrived = spectate_room("a", "playing", 40, 2);
        reg.heartbeat(&arrived);
        assert!(
            reg.matches["a"].spec_reservations.is_empty(),
            "connected watchers clear their reservations"
        );
        assert!(reg.reserve_seat().is_some());
        assert!(reg.reserve_seat().is_some());
        assert!(
            reg.reserve_seat().is_none(),
            "2 connected + 2 reserved is the cap"
        );
    }

    #[test]
    fn playing_private_room_seats_a_watcher_until_the_cap() {
        let mut reg = Registry::default();
        let mut room = private_room("priv", "ABCD", 8);
        room.phase = "playing".into();
        room.phase_left_secs = Some(100);
        room.join_mode = "spectate".into();
        room.spectators = 3;
        room.accepting = false;
        reg.heartbeat(&room);
        let seat = reg.lookup_room("abcd", "1.1.1.1", Instant::now()).unwrap();
        assert!(seat.spectate);
        assert_eq!(seat.code, "ABCD");
        let err = reg
            .lookup_room("ABCD", "1.1.1.2", Instant::now())
            .unwrap_err();
        assert_eq!(err.status, 409);
        assert_eq!(err.error, "in_match");
        assert_eq!(err.retry_after, Some(100 + rooms::RESULTS_SECS));
        assert!(err.message.contains("4/4"));
    }

    #[test]
    fn results_private_room_without_spectate_still_retries() {
        let mut reg = Registry::default();
        let mut room = private_room("priv", "ABCD", 2);
        room.phase = "results".into();
        room.phase_left_secs = Some(8);
        room.join_mode = "none".into();
        reg.heartbeat(&room);
        let err = reg
            .lookup_room("ABCD", "1.1.1.1", Instant::now())
            .unwrap_err();
        assert_eq!(err.error, "results");
        assert_eq!(err.retry_after, Some(8));
        assert_eq!(err.message, rooms::RESULTS);
    }

    #[test]
    fn quick_start_spectates_only_within_two_minutes_and_fallback_has_no_cap() {
        let mut reg = Registry::default();
        // 105s left + 15s results = 120s, still a quick spectate.
        reg.heartbeat(&spectate_room("edge", "playing", 105, 0));
        // 106s + 15s = 121s, over the cap: launch instead.
        reg.heartbeat(&spectate_room("long", "playing", 106, 0));
        reg.heartbeat(&spectate_room("soon", "results", 4, 0));
        for _ in 0..MAX_SPECTATORS {
            assert_eq!(
                reg.reserve_seat().unwrap().0,
                "soon",
                "shortest wait inside the cap"
            );
        }
        for _ in 0..MAX_SPECTATORS {
            assert_eq!(reg.reserve_seat().unwrap().0, "edge");
        }
        assert!(
            reg.reserve_seat().is_none(),
            "a wait over 120s is not a quick spectate"
        );
        let (id, _, spectate) = reg.reserve_any_spectate().unwrap();
        assert_eq!(id, "long");
        assert!(spectate);
    }

    #[test]
    fn named_lobby_plays_and_a_long_round_spectates() {
        let mut reg = Registry::default();
        let mut lobby = beat("lobby", 1);
        lobby.max_players = 8;
        reg.heartbeat(&lobby);
        reg.heartbeat(&spectate_room("late", "playing", 400, 0));
        match reg.plan_join(Some("lobby")) {
            JoinPlan::Seat(id, _, spectate) => {
                assert_eq!(id, "lobby");
                assert!(!spectate);
            }
            other => panic!("expected a combat seat, got {other:?}"),
        }
        match reg.plan_join(Some("late")) {
            JoinPlan::Seat(id, _, spectate) => {
                assert_eq!(id, "late");
                assert!(spectate, "named join ignores the two-minute cap");
            }
            other => panic!("expected a watcher seat, got {other:?}"),
        }
        let mut late = spectate_room("late", "playing", 400, 0);
        late.spectators = u32::from(MAX_SPECTATORS);
        reg.heartbeat(&late);
        assert!(
            matches!(reg.plan_join(Some("late")), JoinPlan::Full),
            "a full watcher bench is full, not a quick start"
        );
    }

    #[test]
    fn named_late_lobby_is_starting_and_a_full_lobby_is_full() {
        let mut reg = Registry::default();
        let mut late = beat("late", 3);
        late.max_players = 8;
        late.join_mode = "none".into();
        late.phase_left_secs = Some(5);
        let mut full = beat("full", 8);
        full.max_players = 8;
        full.humans = 8;
        full.join_mode = "none".into();
        full.phase_left_secs = Some(12);
        let mut recovering = beat("rec", 2);
        recovering.phase = "recovering".into();
        recovering.join_mode = "none".into();
        reg.heartbeat(&late);
        reg.heartbeat(&full);
        reg.heartbeat(&recovering);
        assert!(matches!(
            reg.plan_join(Some("late")),
            JoinPlan::Starting { retry_after: 5 }
        ));
        assert!(matches!(reg.plan_join(Some("full")), JoinPlan::Full));
        assert!(matches!(reg.plan_join(Some("rec")), JoinPlan::Full));
    }

    #[test]
    fn room_list_hides_empty_warm_servers_and_orders_like_the_lobby_pick() {
        let mut reg = Registry::default();
        reg.heartbeat(&beat("warm", 0));
        let mut few = beat("few", 1);
        few.max_players = 8;
        few.phase_left_secs = Some(20);
        few.bots = 7;
        let mut many = beat("many", 3);
        many.max_players = 8;
        many.humans = 3;
        many.phase_left_secs = Some(4);
        many.bots = 5;
        reg.heartbeat(&few);
        reg.heartbeat(&many);
        reg.heartbeat(&spectate_room("watch", "playing", 30, 1));
        reg.heartbeat(&private_room("priv", "ABCD", 2));
        let mut dead = beat("dead", 2);
        dead.max_players = 8;
        reg.heartbeat(&dead);
        reg.matches.get_mut("dead").unwrap().launched_at = Instant::now() - STARTUP_GRACE;
        reg.matches.get_mut("dead").unwrap().last_heartbeat =
            Some(Instant::now() - HEARTBEAT_TIMEOUT);
        let mut catching = beat("catch", 2);
        catching.max_players = 8;
        reg.heartbeat(&catching);
        reg.matches.get_mut("catch").unwrap().catching_up = true;
        let mut abandoned = beat("gone", 2);
        abandoned.max_players = 8;
        reg.heartbeat(&abandoned);
        reg.matches.get_mut("gone").unwrap().abandoned = true;
        let mut recovering = beat("rec", 2);
        recovering.phase = "recovering".into();
        recovering.max_players = 8;
        reg.heartbeat(&recovering);

        let mut fresh = MatchInfo::new("fresh".into(), "127.0.0.1:7001".parse().unwrap(), 8);
        fresh.phase = "lobby".into();
        fresh.join_mode = "lobby".into();
        reg.matches.insert("fresh".into(), fresh);
        reg.reserve_in("fresh");

        let list = reg.room_list();
        let ids: Vec<&str> = list
            .rooms
            .iter()
            .map(|room| room.match_id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["watch", "many", "few", "fresh"],
            "more humans first; empty, private, dead, catching up, abandoned, recovering stay out"
        );
        let watch = &list.rooms[0];
        assert_eq!(watch.entry, "spectate");
        assert_eq!(watch.phase, "playing");
        assert_eq!(watch.spectators, 1);
        assert_eq!(watch.humans, 8);
        assert_eq!(watch.max_spectators, u32::from(MAX_SPECTATORS));
        assert_eq!(list.rooms[1].entry, "play");
        assert_eq!(list.rooms[1].bots, 5);
        assert_eq!(list.rooms[1].phase_left_secs, Some(4));
        let created = list
            .rooms
            .iter()
            .find(|room| room.match_id == "fresh")
            .unwrap();
        assert_eq!(created.entry, "play");
        assert_eq!(created.humans, 0, "the creator is still only a reservation");
        assert_eq!(created.phase, "lobby");
    }

    #[test]
    fn claim_idle_public_takes_only_an_empty_joinable_lobby() {
        let mut reg = Registry::default();
        reg.heartbeat(&beat("warm", 0));
        reg.heartbeat(&beat("busy", 1));
        reg.heartbeat(&private_room("priv", "WXYZ", 0));
        reg.heartbeat(&beat("held", 0));
        reg.reserve_in("held");
        let (id, _) = reg.claim_idle_public().unwrap();
        assert_eq!(id, "warm");
        assert!(
            reg.claim_idle_public().is_none(),
            "the claimed lobby now has a reservation, and the others are not empty"
        );
        let list = reg.room_list();
        assert!(
            list.rooms
                .iter()
                .any(|room| room.match_id == "warm" && room.entry == "play"),
            "a claimed empty lobby shows up immediately"
        );
    }

    #[test]
    fn create_and_lookup_share_one_ip_budget() {
        let mut reg = Registry::default();
        let now = Instant::now();
        for _ in 0..rooms::RATE_MAX {
            assert!(reg.allow_ip("4.4.4.4", now));
        }
        assert!(
            !reg.allow_ip("4.4.4.4", now),
            "a create spends the same budget"
        );
        assert_eq!(
            reg.lookup_room("ZZZZ", "4.4.4.4", now).unwrap_err().status,
            429
        );
        assert!(reg.allow_ip("5.5.5.5", now));
    }
}
