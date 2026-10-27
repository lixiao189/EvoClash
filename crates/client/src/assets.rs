//! Image files shipped next to the client, and the gate that holds the session back until they
//! have loaded.
//!
//! The files live in `crates/client/assets/lpc/`; Trunk copies that folder to
//! `/play/assets/lpc/` (see index.html) and Bevy fetches them over HTTP from there. Anything
//! that needs a file before the match is shown registers its handle in [`PendingAssets`] during
//! `Startup`; the session joins a match only once every one of them has loaded. Every file
//! must be listed in `assets/CREDITS.md` (checked by `tests/credits.rs`).

use bevy::asset::LoadState;
use bevy::prelude::*;

pub struct AssetsPlugin;

impl Plugin for AssetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingAssets>();
        app.add_systems(Update, track_loading);
    }
}

/// Files the match can't be shown without, and how far along loading them is.
#[derive(Resource, Default)]
pub struct PendingAssets {
    handles: Vec<UntypedHandle>,
    pub state: Loading,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Loading {
    #[default]
    Waiting,
    Ready,
    /// The path that failed.
    Failed(String),
}

impl PendingAssets {
    pub fn add(&mut self, handle: impl Into<UntypedHandle>) {
        self.handles.push(handle.into());
    }

    /// How many files the boot cover waits for.
    pub fn total(&self) -> usize {
        self.handles.len()
    }
}

/// Run condition: every registered file has loaded.
pub fn assets_ready(pending: Res<PendingAssets>) -> bool {
    pending.state == Loading::Ready
}

fn track_loading(server: Res<AssetServer>, mut pending: ResMut<PendingAssets>) {
    if pending.state != Loading::Waiting {
        return;
    }
    let mut waiting = false;
    for handle in &pending.handles {
        match server.load_state(handle.id()) {
            LoadState::Loaded => {}
            LoadState::Failed(err) => {
                let path = server
                    .get_path(handle.id())
                    .map_or_else(|| "?".to_string(), |p| p.to_string());
                error!("could not load {path}: {err}");
                pending.state = Loading::Failed(path);
                return;
            }
            _ => waiting = true,
        }
    }
    if !waiting {
        if !pending.handles.is_empty() {
            info!("{} asset files loaded", pending.handles.len());
        }
        pending.state = Loading::Ready;
    }
}
