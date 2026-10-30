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

/// The query a created room's page should carry once it exists: `create` gives way to the
/// room it made (`room=CODE` for a private room, `match=ID` for a public one), so a reload or a
/// shared link goes back to it instead of creating another.
pub(crate) fn created_query(query: &str, match_id: &str, room_code: Option<&str>) -> String {
    let mut pairs: Vec<(String, String)> = query
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (!matches!(key, "create" | "private" | "room" | "match"))
                .then(|| (key.to_string(), value.to_string()))
        })
        .collect();
    match room_code {
        Some(code) => pairs.push(("room".into(), code.to_string())),
        None => pairs.push(("match".into(), match_id.to_string())),
    }
    let joined: Vec<String> = pairs.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("?{}", joined.join("&"))
}

/// Rewrite the address bar in place (no reload, no history entry) after 创建房间 succeeds.
#[cfg(target_family = "wasm")]
pub(crate) fn settle_created_url(match_id: &str, room_code: Option<&str>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(search) = window.location().search() else {
        return;
    };
    let url = created_query(&search, match_id, room_code);
    if let Ok(history) = window.history() {
        let _ =
            history.replace_state_with_url(&web_sys::wasm_bindgen::JsValue::NULL, "", Some(&url));
    }
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn settle_created_url(_match_id: &str, _room_code: Option<&str>) {}

/// Forget the seat: the match is gone, or the player left it on purpose.
#[cfg(target_family = "wasm")]
pub(crate) fn clear() {
    if let Some(storage) = storage() {
        let _ = storage.remove_item(KEY);
    }
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn clear() {}

#[cfg(test)]
mod tests {
    use super::created_query;

    #[test]
    fn a_created_room_page_names_the_room_it_made() {
        // Private: the code, which is also the link to share.
        assert_eq!(
            created_query("?name=Ann&create=1&private=1", "m1", Some("ABCD")),
            "?name=Ann&room=ABCD"
        );
        // Public: the match id, the same thing a room-list click puts there.
        assert_eq!(
            created_query("?name=Ann&create=1", "m1", None),
            "?name=Ann&match=m1"
        );
        // Other parameters (debug `lag`, `fault`) stay; a stale room/match is replaced.
        assert_eq!(
            created_query("?name=Ann&lag=80&create=1&match=old", "m1", None),
            "?name=Ann&lag=80&match=m1"
        );
    }
}
