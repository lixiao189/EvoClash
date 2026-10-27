//! Reconnect overlay: shown while the session is trying to get back into a match.
//!
//! Filled in by the UI workstream. `ConnectionStatus` is the interface the session writes to;
//! the overlay only reads it, so the session never has to know about UI nodes.

use bevy::prelude::*;

/// What the session's connection loop is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkStatus {
    /// Before the first join request.
    #[default]
    Idle,
    /// HTTP join in flight or the first WebTransport handshake.
    Connecting,
    /// A live client link.
    Connected,
    /// The link dropped and the session is retrying.
    Reconnecting,
    /// The session ran out of attempts.
    Failed,
}

/// Shared between the session (writer) and this overlay (reader).
#[derive(Resource, Debug, Default)]
pub struct ConnectionStatus {
    pub status: LinkStatus,
    /// Reconnect attempts started so far.
    pub attempts: u32,
    /// Attempt budget (`MAX_RECONNECT_ATTEMPTS`).
    pub max: u32,
}

pub struct ConnectUiPlugin;

impl Plugin for ConnectUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConnectionStatus>();
    }
}
