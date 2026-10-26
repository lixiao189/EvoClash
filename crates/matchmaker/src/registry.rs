//! Live view of the match fleet, rebuilt continuously from server heartbeats.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use evoclash_game::api::Heartbeat;
use serde::Serialize;

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
    pub is_private: bool,
    pub room_code: Option<String>,
    pub room_seed: u64,
    pub match_index: u32,
    /// `none` | `lobby`. Trusted from the server; not re-derived here.
    pub join_mode: String,
    pub exiting: bool,
}

/// What `POST /v1/join` should do with an optional `match_id`.
pub enum JoinPlan {
    Seat(String, SocketAddr),
    Started,
    Starting {
        retry_after: u32,
    },
    /// Unknown id, or a lobby that is simply full: quick start.
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
            is_private: false,
            room_code: None,
            room_seed: 0,
            match_index: 0,
            join_mode: "lobby".into(),
            exiting: false,
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

    pub fn overloaded(&self) -> bool {
        self.tick_us > OVERLOADED_TICK_US
    }

    fn joinable(&mut self, now: Instant) -> bool {
        self.healthy(now)
            && self.accepting
            && !self.draining
            && !self.overloaded()
            && self.join_mode == "lobby"
            && self.free_seats(now) > 0
    }

    /// Joinable and nobody in it or on the way: a warm spare for the next burst of joins.
    fn idle(&mut self, now: Instant) -> bool {
        self.joinable(now) && self.players == 0 && self.reservations.is_empty()
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
}

impl Registry {
    /// Quick start: open lobbies only. More humans first; equal humans, more time left;
    /// a countdown that has not started sorts last among that human count.
    pub fn reserve_seat(&mut self) -> Option<(String, SocketAddr)> {
        let now = Instant::now();
        let mut ids: Vec<String> = self.matches.keys().cloned().collect();
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
                .then_with(|| a.cmp(b))
        });
        let id = ids
            .into_iter()
            .find(|id| self.matches.get_mut(id).is_some_and(|m| m.joinable(now)))?;
        let m = self.matches.get_mut(&id)?;
        m.reservations.push(now);
        Some((m.id.clone(), m.addr))
    }

    /// A named match. `started` / `starting` only when that match exists and is not a quick start.
    pub fn plan_join(&mut self, match_id: Option<&str>) -> JoinPlan {
        let now = Instant::now();
        if let Some(id) = match_id.map(str::trim).filter(|id| !id.is_empty())
            && let Some(m) = self.matches.get_mut(id)
        {
            if m.joinable(now) {
                let addr = m.addr;
                m.reservations.push(now);
                return JoinPlan::Seat(id.to_string(), addr);
            }
            let retry_after = m.phase_left_secs.unwrap_or(0);
            if m.phase == "starting" || (m.phase == "lobby" && m.join_mode != "lobby") {
                return JoinPlan::Starting { retry_after };
            }
            if matches!(m.phase.as_str(), "playing" | "results" | "recovering") {
                return JoinPlan::Started;
            }
        }
        JoinPlan::Quick
    }

    pub fn reserve_in(&mut self, id: &str) -> Option<SocketAddr> {
        let m = self.matches.get_mut(id)?;
        m.reservations.push(Instant::now());
        Some(m.addr)
    }

    /// Record a heartbeat. Returns whether the server should drain.
    pub fn heartbeat(&mut self, beat: &Heartbeat) -> bool {
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
        entry.cert_digest = Some(beat.cert_digest.clone());
        entry.players = beat.players;
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
        entry.draining
    }

    /// Forget matches that stopped heartbeating. Returns their ids.
    pub fn reap(&mut self) -> Vec<String> {
        let now = Instant::now();
        let dead: Vec<String> = self
            .matches
            .values()
            .filter(|m| !m.healthy(now))
            .map(|m| m.id.clone())
            .collect();
        for id in &dead {
            self.matches.remove(id);
        }
        dead
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
    fn named_match_reports_started_or_starting_and_unknown_is_quick() {
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
        assert!(matches!(reg.plan_join(Some("play")), JoinPlan::Started));
        assert!(matches!(
            reg.plan_join(Some("go")),
            JoinPlan::Starting { retry_after: 2 }
        ));
        assert!(matches!(reg.plan_join(Some("missing")), JoinPlan::Quick));
        assert!(matches!(reg.plan_join(None), JoinPlan::Quick));
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
}
