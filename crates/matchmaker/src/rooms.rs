//! Private room codes and the per-IP lookup budget.
//!
//! Alphabet drops 0/O, 1/I/L, and U/V so a code read aloud doesn't turn into a different room.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// 29 characters. 4 of them is about 700k codes.
pub const ALPHABET: &str = "23456789ABCDEFGHJKMNPQRSTWXYZ";
pub const HOLD: Duration = Duration::from_secs(10 * 60);
pub const RATE_WINDOW: Duration = Duration::from_secs(60);
pub const RATE_MAX: usize = 10;
/// Results lasts 15s. An in-match retry waits out the rest of the round plus that.
pub const RESULTS_SECS: u32 = 15;

pub const BAD_CODE: &str = "房间码是 4 位字母或数字。";
pub const CLOSED: &str = "房间已关闭。你可以创建新的私人房间。";
pub const FULL: &str = "房间已满（8/8）。请稍后再试，或创建自己的房间。";
pub const STARTING: &str = "房间正在开局，请稍候。";
pub const RESULTS: &str = "本局刚结束，马上回到等待大厅…";
pub const RATE: &str = "尝试次数过多，请一分钟后再试。";

#[derive(Debug)]
pub struct RoomSeat {
    pub id: String,
    pub addr: std::net::SocketAddr,
    pub code: String,
}

#[derive(Debug)]
pub struct RoomFailure {
    pub status: u16,
    pub error: &'static str,
    pub message: String,
    pub retry_after: Option<u32>,
}

impl RoomFailure {
    pub fn new(
        status: u16,
        error: &'static str,
        message: impl Into<String>,
        retry_after: Option<u32>,
    ) -> Self {
        Self {
            status,
            error,
            message: message.into(),
            retry_after,
        }
    }

    pub fn rate() -> Self {
        Self::new(429, "rate", RATE, None)
    }
}

/// Trimmed, uppercased, exactly 4 alphabet characters. Anything else is a format error.
pub fn normalize(raw: &str) -> Option<String> {
    let code = raw.trim().to_uppercase();
    if code.chars().count() == 4 && code.chars().all(|c| ALPHABET.contains(c)) {
        Some(code)
    } else {
        None
    }
}

pub fn random_code() -> String {
    let bytes = ALPHABET.as_bytes();
    (0..4)
        .map(|_| {
            let i = rand::random_range(0..bytes.len());
            bytes[i] as char
        })
        .collect()
}

pub fn not_found_message(code: &str) -> String {
    format!("找不到房间 {code}，请检查房间码，或让房主重新发送链接。")
}

pub fn in_match_message(total_secs: u32) -> String {
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    format!("房间正在对局，约 {minutes} 分 {seconds} 秒后结束，结束后自动加入。")
}

/// Lookups and creates share one rolling minute per client IP.
#[derive(Default)]
pub struct Attempts {
    hits: HashMap<String, VecDeque<Instant>>,
}

impl Attempts {
    pub fn allow(&mut self, ip: &str, now: Instant) -> bool {
        let hits = self.hits.entry(ip.to_string()).or_default();
        while hits
            .front()
            .is_some_and(|at| now.duration_since(*at) >= RATE_WINDOW)
        {
            hits.pop_front();
        }
        if hits.len() >= RATE_MAX {
            return false;
        }
        hits.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_window_and_full_copy_match_eight_players() {
        assert_eq!(RESULTS_SECS, 15);
        assert!(FULL.contains("8/8"));
    }
}
