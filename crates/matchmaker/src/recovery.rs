//! Crash detection for the local fleet: when to relaunch with `--recover`, and when to give up.
//!
//! A room is relaunched at most twice. The third crash, or a server that exits
//! [`ABANDON_EXIT`] because the log does not match, closes the room.

/// Replacement processes already launched. The next crash gives up.
pub const MAX_RECOVERIES: u8 = 2;
/// `evoclash-server --recover` exits with this when the log cannot be trusted.
pub const ABANDON_EXIT: i32 = 2;
/// Clients retry `/v1/rejoin` after this many seconds while catch-up is still running.
pub const RETRY_AFTER_SECS: u32 = 2;
pub const RECOVERING_MSG: &str = "服务器恢复中，请稍候。";
pub const ABANDONED_MSG: &str = "服务器出错，本局已作废。";

/// Heartbeat says the replacement process has finished fast-forward (or restored a lobby).
/// During fast-forward the server reports `recovering` with no phase clock.
pub fn catchup_done(phase: &str, phase_left_secs: Option<u32>) -> bool {
    match phase {
        "recovering" => phase_left_secs.is_some(),
        _ => true,
    }
}

pub struct Launch {
    pub id: String,
    pub room_seed: u64,
    pub room_code: Option<String>,
}

pub enum Step {
    None,
    Dropped(String),
    /// Crash recovery. The caller must stop the old process first (`Fleet::stop`, which waits for
    /// it to exit): a heartbeat timeout does not prove it is gone.
    Relaunch(Launch),
    Abandoned(String),
}

pub enum RejoinGate {
    Ready,
    Recovering,
    Abandoned,
    Gone,
}

/// Silence longer than the heartbeat timeout, and the last beat was not a normal exit.
#[cfg(test)]
pub fn heartbeat_gap_is_crash(silent_for: std::time::Duration, exiting: bool) -> bool {
    !exiting && silent_for >= crate::registry::HEARTBEAT_TIMEOUT
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::registry::Registry;

    fn beat(id: &str) -> evoclash_game::api::Heartbeat {
        evoclash_game::api::Heartbeat {
            match_id: id.into(),
            public_addr: "127.0.0.1:7000".into(),
            cert_digest: "ab".into(),
            players: 1,
            max_players: 8,
            accepting: true,
            tick_us: 10,
            phase: "playing".into(),
            phase_left_secs: Some(100),
            humans: 1,
            bots: 7,
            spectators: 0,
            is_private: false,
            room_code: None,
            room_seed: 99,
            match_index: 0,
            join_mode: "spectate".into(),
            exiting: false,
        }
    }

    fn private(id: &str, code: &str) -> evoclash_game::api::Heartbeat {
        let mut beat = beat(id);
        beat.is_private = true;
        beat.room_code = Some(code.into());
        beat.join_mode = "lobby".into();
        beat.phase = "lobby".into();
        beat.phase_left_secs = None;
        beat
    }

    #[test]
    fn nonzero_exit_and_a_signal_crash_immediately() {
        let mut reg = Registry::default();
        reg.heartbeat(&beat("a"));
        assert!(matches!(
            reg.on_process_exit("a", Some(1)),
            Step::Relaunch(_)
        ));
        let m = &reg.matches["a"];
        assert!(m.catching_up);
        assert_eq!(m.recoveries, 1);
        assert_eq!(m.room_seed, 99);
        assert_eq!(m.phase, "recovering");
        assert!(matches!(reg.rejoin_gate("a"), RejoinGate::Recovering));

        reg.heartbeat(&beat("b"));
        assert!(matches!(reg.on_process_exit("b", None), Step::Relaunch(_)));
        assert_eq!(reg.matches["b"].recoveries, 1);
    }

    #[test]
    fn a_clean_exit_and_an_exiting_heartbeat_are_not_recovered() {
        let mut reg = Registry::default();
        reg.heartbeat(&beat("clean"));
        assert!(matches!(
            reg.on_process_exit("clean", Some(0)),
            Step::Dropped(_)
        ));
        assert!(!reg.matches.contains_key("clean"));

        reg.heartbeat(&beat("bye"));
        reg.matches.get_mut("bye").unwrap().exiting = true;
        assert!(matches!(
            reg.on_process_exit("bye", Some(9)),
            Step::Dropped(_)
        ));
        assert!(!reg.matches.contains_key("bye"));

        reg.heartbeat(&beat("gap"));
        {
            let m = reg.matches.get_mut("gap").unwrap();
            m.exiting = true;
            m.last_heartbeat = Some(Instant::now() - Duration::from_secs(7));
        }
        let steps = reg.poll_crashes(Instant::now(), |_| true);
        assert!(matches!(steps.as_slice(), [Step::Dropped(_)]));
        assert!(!reg.matches.contains_key("gap"));
    }

    #[test]
    fn a_six_second_heartbeat_gap_is_a_crash() {
        let gap = crate::registry::HEARTBEAT_TIMEOUT;
        assert!(heartbeat_gap_is_crash(gap, false));
        assert!(!heartbeat_gap_is_crash(gap, true));
        assert!(!heartbeat_gap_is_crash(
            gap - Duration::from_millis(1),
            false
        ));

        let mut reg = Registry::default();
        reg.heartbeat(&beat("a"));
        reg.matches.get_mut("a").unwrap().last_heartbeat =
            Some(Instant::now() - Duration::from_secs(6));
        let steps = reg.poll_crashes(Instant::now(), |_| true);
        assert!(matches!(steps.as_slice(), [Step::Relaunch(_)]));
        assert!(reg.matches["a"].catching_up);
        // Still inside the startup grace: do not count the same outage twice.
        assert!(reg.poll_crashes(Instant::now(), |_| true).is_empty());
    }

    #[test]
    fn an_adopted_match_that_goes_quiet_is_dropped_not_recovered() {
        // Adopted after a matchmaker restart: no child to kill, so it may still be running.
        let mut reg = Registry::default();
        reg.heartbeat(&beat("adopted"));
        reg.matches.get_mut("adopted").unwrap().last_heartbeat =
            Some(Instant::now() - Duration::from_secs(7));
        let steps = reg.poll_crashes(Instant::now(), |_| false);
        assert!(matches!(steps.as_slice(), [Step::Dropped(_)]));
        assert!(!reg.matches.contains_key("adopted"));
    }

    #[test]
    fn the_third_crash_abandons_and_releases_a_private_code() {
        let mut reg = Registry::default();
        reg.heartbeat(&private("room", "ABCD"));
        assert!(matches!(
            reg.on_process_exit("room", Some(1)),
            Step::Relaunch(_)
        ));
        assert!(matches!(
            reg.on_process_exit("room", None),
            Step::Relaunch(_)
        ));
        assert_eq!(reg.matches["room"].recoveries, 2);
        assert!(matches!(
            reg.on_process_exit("room", Some(1)),
            Step::Abandoned(_)
        ));
        assert!(reg.matches["room"].abandoned);
        assert!(reg.matches["room"].room_code.is_none());
        assert!(matches!(reg.rejoin_gate("room"), RejoinGate::Abandoned));
        let err = reg
            .lookup_room("ABCD", "1.1.1.1", Instant::now())
            .unwrap_err();
        assert_eq!(err.status, 410);
        assert_eq!(err.error, "closed");

        // A server that itself gives up does not burn the remaining attempts.
        reg.heartbeat(&private("bad", "WXYZ"));
        assert!(matches!(
            reg.on_process_exit("bad", Some(ABANDON_EXIT)),
            Step::Abandoned(_)
        ));
        assert_eq!(reg.matches["bad"].recoveries, 0);
        assert!(matches!(reg.rejoin_gate("bad"), RejoinGate::Abandoned));
    }

    #[test]
    fn rejoin_stays_recovering_until_the_phase_clock_is_known() {
        let mut reg = Registry::default();
        reg.heartbeat(&beat("a"));
        reg.on_process_exit("a", Some(1));
        let mut mid = beat("a");
        mid.phase = "recovering".into();
        mid.join_mode = "none".into();
        mid.phase_left_secs = None;
        mid.public_addr = "127.0.0.1:7001".into();
        reg.heartbeat(&mid);
        assert!(reg.matches["a"].catching_up);
        assert!(matches!(reg.rejoin_gate("a"), RejoinGate::Recovering));

        mid.phase_left_secs = Some(240);
        reg.heartbeat(&mid);
        assert!(!reg.matches["a"].catching_up);
        assert!(matches!(reg.rejoin_gate("a"), RejoinGate::Ready));
        assert_eq!(
            reg.matches["a"].addr,
            "127.0.0.1:7001".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn quick_start_skips_a_recovering_room_and_a_code_join_is_503() {
        let mut reg = Registry::default();
        reg.heartbeat(&private("room", "ABCD"));
        reg.heartbeat(&beat("pub"));
        reg.matches.get_mut("pub").unwrap().phase = "lobby".into();
        reg.matches.get_mut("pub").unwrap().join_mode = "lobby".into();
        reg.matches.get_mut("pub").unwrap().phase_left_secs = None;
        reg.on_process_exit("pub", Some(1));
        reg.on_process_exit("room", Some(1));
        assert!(reg.reserve_seat().is_none());
        let err = reg
            .lookup_room("ABCD", "8.8.8.8", Instant::now())
            .unwrap_err();
        assert_eq!(err.status, 503);
        assert_eq!(err.error, "recovering");
        assert_eq!(err.retry_after, Some(RETRY_AFTER_SECS));
    }
}
