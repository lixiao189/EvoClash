//! The seat this tab holds, kept in `sessionStorage` so a page refresh rejoins the same match
//! instead of queueing for a new one.
//!
//! Only the ids and the reconnect key go in; the address, certificate and token come from the
//! `/v1/rejoin` reply before anything connects. `sessionStorage` lives exactly as long as the tab,
//! which is the lifetime a seat has, and unlike `window.name` it is not readable by other sites
//! the tab navigates to.
//!
//! Every access is best-effort: a browser that refuses storage (private mode, storage disabled)
//! just gets no remembered seat.

use serde::{Deserialize, Serialize};

/// One key, so a future format change cannot collide with an old value.
const KEY: &str = "evoclash.seat";

#[derive(Serialize, Deserialize)]
pub(crate) struct StoredSeat {
    pub(crate) match_id: String,
    pub(crate) client_id: u64,
    pub(crate) reconnect_key: String,
    /// Private room code, so a reload of a `?room=` page can tell it is the same room.
    #[serde(default)]
    pub(crate) room_code: Option<String>,
}

#[cfg(target_family = "wasm")]
fn storage() -> Option<web_sys::Storage> {
    // The getter throws in a few browsers when storage is blocked; that is not an error here.
    match web_sys::window()?.session_storage() {
        Ok(Some(storage)) => Some(storage),
        _ => None,
    }
}

/// The seat a previous page load left behind, if the tab still has one.
#[cfg(target_family = "wasm")]
pub(crate) fn load() -> Option<StoredSeat> {
    let raw = storage()?.get_item(KEY).ok()??;
    serde_json::from_str(&raw).ok()
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn load() -> Option<StoredSeat> {
    None
}

#[cfg(target_family = "wasm")]
pub(crate) fn save(seat: &StoredSeat) {
    let Some(storage) = storage() else {
        return;
    };
    if let Ok(text) = serde_json::to_string(seat) {
        let _ = storage.set_item(KEY, &text);
    }
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn save(_seat: &StoredSeat) {}

/// Forget the seat: the match is gone, or the player left it on purpose.
#[cfg(target_family = "wasm")]
pub(crate) fn clear() {
    if let Some(storage) = storage() {
        let _ = storage.remove_item(KEY);
    }
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn clear() {}
