//! EvoClash browser client (wasm32). Served by the matchmaker, which it joins through the same
//! origin it was loaded from: `http://127.0.0.1:8080/play/?name=Alice` (`&lag=100` simulates latency).
//!
//! The crate still compiles natively so its tests (sprite previews) run with `cargo test`, but
//! the game itself only runs in a browser: `make run`.
// Natively only the tests use the game code; the wasm build (checked in CI) catches dead code.
#![cfg_attr(not(target_family = "wasm"), allow(dead_code, unused_imports))]

mod assets;
mod backpack;
mod board_ui;
mod buttons;
mod connect_ui;
mod death_screen;
mod effects;
mod evo_panel;
mod finale_ui;
mod hud;
mod input;
mod input_mode;
mod loading_ui;
mod lobby_ui;
mod lpc;
mod menu;
mod minimap;
mod perf;
mod predict;
mod recover_ui;
mod render;
mod results_ui;
mod session;
mod spectate_ui;
mod textures;

use bevy::asset::AssetMetaCheck;
use bevy::prelude::*;
use evoclash_game::net::tick_duration;
use lightyear::prelude::client::ClientPlugins;

#[derive(Resource, Clone, Debug)]
pub struct ClientArgs {
    pub name: String,
    /// Matchmaker base URL: the origin the page was served from.
    pub matchmaker: String,
    /// Simulated one-way latency in ms for testing prediction (dev only).
    pub fake_latency_ms: u64,
    /// Private room code from `?room=`. Absent for a quick start.
    pub room: Option<String>,
    /// `?create=1` posts `/v1/rooms` instead of joining.
    pub create_room: bool,
    /// `?match=` joins that room. Absent for a quick start.
    pub match_id: Option<String>,
    /// `?private=1` with `?create=1`: `CreateRoomRequest::private`.
    pub private_room: bool,
}

/// Read the page URL. `None` until the player has picked a name (index.html asks for one).
#[cfg(target_family = "wasm")]
fn page_args() -> Option<ClientArgs> {
    let location = web_sys::window()?.location();
    let params = web_sys::UrlSearchParams::new_with_str(&location.search().ok()?).ok()?;
    let name = params.get("name").filter(|name| !name.trim().is_empty())?;
    let room = params
        .get("room")
        .map(|code| code.trim().to_uppercase())
        .filter(|code| !code.is_empty());
    let match_id = params
        .get("match")
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());
    Some(ClientArgs {
        name,
        matchmaker: location.origin().ok()?,
        fake_latency_ms: params
            .get("lag")
            .and_then(|lag| lag.parse().ok())
            .unwrap_or(0),
        room,
        create_room: params.get("create").as_deref() == Some("1"),
        match_id,
        private_room: params.get("private").as_deref() == Some("1"),
    })
}

#[cfg(target_family = "wasm")]
fn main() {
    let Some(args) = page_args() else {
        return;
    };

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("EvoClash — {}", args.name),
                    canvas: Some("#game".into()),
                    fit_canvas_to_parent: true,
                    // Keep browser shortcuts (F5, Ctrl+W, …) working.
                    prevent_default_event_handling: false,
                    ..default()
                }),
                ..default()
            })
            // Files come from `/play/assets/` (see assets.rs); there are no `.meta` files, so
            // don't ask the server for one per image.
            .set(AssetPlugin {
                meta_check: AssetMetaCheck::Never,
                ..default()
            }),
    );
    app.add_plugins(ClientPlugins {
        tick_duration: tick_duration(),
    });
    app.add_plugins(evoclash_game::SharedPlugin);
    app.insert_resource(args);
    app.add_plugins((
        assets::AssetsPlugin,
        session::SessionPlugin,
        connect_ui::ConnectUiPlugin,
        loading_ui::LoadingUiPlugin,
        menu::MenuPlugin,
        input::InputPlugin,
        input_mode::InputModePlugin,
        predict::PredictPlugin,
        render::RenderPlugin,
    ));
    // Bevy's Plugins tuple tops out at 15; keep this list in two groups.
    app.add_plugins((
        effects::EffectsPlugin,
        hud::HudPlugin,
        finale_ui::FinaleUiPlugin,
        results_ui::ResultsUiPlugin,
        lobby_ui::LobbyUiPlugin,
        board_ui::BoardUiPlugin,
        recover_ui::RecoverUiPlugin,
        backpack::BackpackPlugin,
        minimap::MinimapPlugin,
        buttons::ButtonsPlugin,
        evo_panel::EvoPanelPlugin,
        death_screen::DeathScreenPlugin,
        spectate_ui::SpectateUiPlugin,
    ));
    app.run();
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    eprintln!(
        "the EvoClash client runs in the browser: `make run`, then open http://127.0.0.1:8080"
    );
    std::process::exit(1);
}
