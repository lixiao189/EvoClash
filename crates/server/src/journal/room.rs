//! The room log as a *new* process finds it: which room the volume holds, and whether the last
//! process left on purpose.
//!
//! `--recover` (a local fleet relaunch) is handed the match id by the matchmaker. A StatefulSet
//! pod has no such caller, so it reads the id and the room seed back from the log on its volume
//! and resumes the same room. A process that exits on purpose (drain, certificate retirement)
//! appends [`FINISHED`] first, so the next container start opens a new room instead of resuming
//! one that was closed.

use super::*;

/// Last line of a room log whose process drained or retired instead of crashing.
pub const FINISHED: &str = "end";

/// What the room log says about the room this volume holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomHead {
    pub match_id: String,
    pub room_seed: u64,
}

pub fn room_log_path(dir: &Path, match_id: &str) -> PathBuf {
    dir.join(format!("room-{}.log", sanitize(match_id)))
}

/// The last non-empty line is the end marker: the room was closed, not crashed.
pub fn finished(text: &str) -> bool {
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .and_then(|line| line.strip_prefix(FINISHED))
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
}

/// Close the room log so the next container start does not resume it. A no-op when this process
/// never wrote one.
pub fn mark_finished(dir: &Path, match_id: &str) {
    let path = room_log_path(dir, match_id);
    if !path.exists() {
        return;
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    match OpenOptions::new().append(true).open(&path) {
        Ok(mut file) => {
            let _ = writeln!(file, "{FINISHED} {secs}");
            let _ = file.sync_all();
        }
        Err(e) => warn!("cannot close {}: {e}", path.display()),
    }
}

/// The room this volume last held, if its process did not exit on purpose (crash, SIGKILL,
/// eviction). The newest log wins: one volume holds one room, but an abandoned crash can leave a
/// log next to a newer one.
pub fn unfinished(dir: &Path) -> Option<RoomHead> {
    let mut best: Option<(SystemTime, RoomHead)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let Some(match_id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("room-"))
            .and_then(|name| name.strip_suffix(".log"))
        else {
            continue;
        };
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if finished(&text) {
            continue;
        }
        // A log with no snapshot (the process died before its first tick) names no seed, so
        // there is nothing to resume.
        let Some(room_seed) = last_seed(&text) else {
            continue;
        };
        let at = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(UNIX_EPOCH);
        let head = RoomHead {
            match_id: match_id.to_owned(),
            room_seed,
        };
        if best.as_ref().is_none_or(|(best_at, _)| at > *best_at) {
            best = Some((at, head));
        }
    }
    best.map(|(_, head)| head)
}

/// `room_seed` of the last snapshot line. The seed never changes within a room; the header line
/// and a log cut off mid-write carry none.
fn last_seed(text: &str) -> Option<u64> {
    text.lines().rev().find_map(|line| {
        line.split_whitespace()
            .find_map(|part| part.strip_prefix("room_seed="))
            .and_then(|value| value.parse().ok())
    })
}

/// Delete every room and match log. Called when a pod opens a new room: the old logs can never
/// be resumed (the third crash, or a clean exit that skipped the end marker).
pub fn clear(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let log =
            name.ends_with(".log") && (name.starts_with("room-") || name.starts_with("match-"));
        if log {
            let _ = fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evoclash-room-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn snapshot(seed: u64) -> String {
        format!(
            "room_seed={seed} code=- private=0 bots=8 monsters=1 host=0 match_index=0 phase=playing paused=0"
        )
    }

    fn set_modified(path: &Path, at: SystemTime) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(at).unwrap();
    }

    #[test]
    fn only_a_log_without_the_end_marker_is_unfinished() {
        assert!(!finished("EVOCLASH room v1\n"));
        assert!(!finished(&format!("EVOCLASH room v1\n{}\n", snapshot(7))));
        assert!(finished(&format!(
            "EVOCLASH room v1\n{}\nend 1700\n",
            snapshot(7)
        )));
        // The marker is a whole line, not a prefix of some other line.
        assert!(!finished("EVOCLASH room v1\nended=1\n"));
    }

    #[test]
    fn the_newest_unfinished_log_names_the_room() {
        let dir = scratch("newest");
        let now = SystemTime::now();
        let write = |name: &str, text: String, at: SystemTime| {
            let path = dir.join(name);
            fs::write(&path, text).unwrap();
            set_modified(&path, at);
        };
        write(
            "room-old.log",
            format!("EVOCLASH room v1\n{}\n", snapshot(1)),
            now - std::time::Duration::from_secs(60),
        );
        // A log that died before its first snapshot names no seed; a closed one is not resumed.
        write("room-header.log", "EVOCLASH room v1\n".into(), now);
        write(
            "room-done.log",
            format!("EVOCLASH room v1\n{}\nend 1700\n", snapshot(3)),
            now,
        );
        write(
            "room-new.log",
            format!("EVOCLASH room v1\n{}\n", snapshot(2)),
            now,
        );

        let head = unfinished(&dir).expect("a room to resume");
        assert_eq!(head.match_id, "new");
        assert_eq!(head.room_seed, 2);

        clear(&dir);
        assert!(unfinished(&dir).is_none());
        assert!(!dir.join("room-new.log").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
