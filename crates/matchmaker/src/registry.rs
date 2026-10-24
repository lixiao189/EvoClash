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
    pub players: u32,
    pub max_players: u32,
    pub accepting: bool,
    pub tick_us: u32,
    pub draining: bool,
    pub launched_at: Instant,
    pub last_heartbeat: Option<Instant>,
    /// Join tokens issued recently, so concurrent joins don't overfill a match.
    reservations: Vec<Instant>,
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
            players: 0,
            max_players,
            accepting: true,
            tick_us: 0,
            draining: false,
            launched_at: Instant::now(),
            last_heartbeat: None,
            reservations: Vec::new(),
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
            && self.free_seats(now) > 0
    }

    /// Joinable and nobody in it or on the way: a warm spare for the next burst of joins.
    fn idle(&mut self, now: Instant) -> bool {
        self.joinable(now) && self.players == 0 && self.reservations.is_empty()
    }
}

#[derive(Default)]
pub struct Registry {
    pub matches: HashMap<String, MatchInfo>,
}

impl Registry {
    /// Pick the fullest joinable match (fills lobbies quickly instead of spreading players thin)
    /// and reserve a seat in it.
    pub fn reserve_seat(&mut self) -> Option<(String, SocketAddr)> {
        let now = Instant::now();
        let best = self
            .matches
            .values_mut()
            .filter_map(|m| {
                m.joinable(now)
                    .then(|| (m.players + m.reservations.len() as u32, m))
            })
            .max_by_key(|(occupancy, _)| *occupancy)
            .map(|(_, m)| m)?;
        best.reservations.push(now);
        Some((best.id.clone(), best.addr))
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
        entry.players = beat.players;
        entry.max_players = beat.max_players;
        entry.accepting = beat.accepting;
        entry.tick_us = beat.tick_us;
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
            players,
            max_players: 2,
            accepting: true,
            tick_us: 100,
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
