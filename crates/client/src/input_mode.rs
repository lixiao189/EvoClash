//! Ui vs Aim. Ui shows the system cursor and ignores mouse fire; Aim hides the cursor and draws
//! the reticle.
//!
//! Decided every [`Update`] after [`pointer_over_ui`](crate::buttons::pointer_over_ui), before
//! the reticle and the camera lean. A missing combatant, a spectator, an open panel, a hovered
//! control, Alt, or a window that is unfocused or has no cursor is Ui. Otherwise Aim.

use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};
use evoclash_game::protocol::Spectator;

use crate::buttons::{PointerOverUi, pointer_over_ui};
use crate::evo_panel::EvoPanelOpen;
use crate::input::{BackpackOpen, LocalPlayer, panel_open};
use crate::menu::MenuOpen;

/// What the pointer is doing.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InputMode {
    /// System cursor, no reticle. Mouse buttons do not fire or throw.
    Ui,
    /// Hidden system cursor, reticle, mouse buttons shoot.
    #[default]
    Aim,
}

/// [`sync_input_mode`], so readers can run after the mode for this frame is known.
#[derive(SystemSet, Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct SyncInputMode;

/// Inputs to [`decide_input_mode`]. Any one Ui reason wins.
#[derive(Clone, Copy, Debug)]
pub struct ModeFacts {
    pub has_local_player: bool,
    pub spectator: bool,
    pub panel_open: bool,
    pub pointer_over_ui: bool,
    pub window_focused: bool,
    pub cursor_inside: bool,
    pub alt_held: bool,
}

/// `Ui` when any hold is set, otherwise `Aim`.
pub fn decide_input_mode(facts: ModeFacts) -> InputMode {
    if !facts.has_local_player
        || facts.spectator
        || facts.panel_open
        || facts.pointer_over_ui
        || !facts.window_focused
        || !facts.cursor_inside
        || facts.alt_held
    {
        InputMode::Ui
    } else {
        InputMode::Aim
    }
}

pub struct InputModePlugin;

impl Plugin for InputModePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputMode>();
        app.add_systems(
            Update,
            (sync_input_mode.in_set(SyncInputMode), apply_cursor)
                .chain()
                .after(pointer_over_ui),
        );
    }
}

fn sync_input_mode(
    mut mode: ResMut<InputMode>,
    windows: Query<&Window, With<PrimaryWindow>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    me: Query<(), With<LocalPlayer>>,
    specs: Query<(), With<Spectator>>,
    backpack: Res<BackpackOpen>,
    menu: Res<MenuOpen>,
    evo: Res<EvoPanelOpen>,
    over_ui: Res<PointerOverUi>,
) {
    let window = windows.single().ok();
    let next = decide_input_mode(ModeFacts {
        has_local_player: !me.is_empty(),
        spectator: !specs.is_empty(),
        panel_open: panel_open(backpack.0, menu.0, evo.0),
        pointer_over_ui: over_ui.0,
        window_focused: window.is_some_and(|window| window.focused),
        cursor_inside: window.is_some_and(|window| window.cursor_position().is_some()),
        alt_held: keys.is_some_and(|keys| keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])),
    });
    mode.set_if_neq(next);
}

/// `CursorOptions` is its own component on the window. Write it only when the bit flips so
/// winit is not told to restyle the cursor every frame.
fn apply_cursor(mode: Res<InputMode>, mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    let Ok(mut options) = windows.single_mut() else {
        return;
    };
    let visible = *mode == InputMode::Ui;
    if options.visible != visible {
        options.visible = visible;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aiming() -> ModeFacts {
        ModeFacts {
            has_local_player: true,
            spectator: false,
            panel_open: false,
            pointer_over_ui: false,
            window_focused: true,
            cursor_inside: true,
            alt_held: false,
        }
    }

    #[test]
    fn aim_while_fighting() {
        assert_eq!(decide_input_mode(aiming()), InputMode::Aim);
    }

    #[test]
    fn ui_for_panel_hover_alt_cursor_focus_spectator_and_loading() {
        let cases = [
            ModeFacts {
                panel_open: true,
                ..aiming()
            },
            ModeFacts {
                pointer_over_ui: true,
                ..aiming()
            },
            ModeFacts {
                alt_held: true,
                ..aiming()
            },
            ModeFacts {
                cursor_inside: false,
                ..aiming()
            },
            ModeFacts {
                window_focused: false,
                ..aiming()
            },
            ModeFacts {
                spectator: true,
                ..aiming()
            },
            ModeFacts {
                has_local_player: false,
                ..aiming()
            },
        ];
        for facts in cases {
            assert_eq!(decide_input_mode(facts), InputMode::Ui);
        }
    }

    /// The plugin in a bare app: a focused window with the cursor inside, a local combatant,
    /// and nothing open. Returns the app after one frame.
    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(InputModePlugin)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<BackpackOpen>()
            .init_resource::<MenuOpen>()
            .init_resource::<EvoPanelOpen>()
            .init_resource::<PointerOverUi>();
        let mut window = Window {
            focused: true,
            ..default()
        };
        window.set_physical_cursor_position(Some(bevy::math::DVec2::new(100.0, 100.0)));
        app.world_mut()
            .spawn((window, PrimaryWindow, CursorOptions::default()));
        app.world_mut().spawn(LocalPlayer);
        app.update();
        app
    }

    fn state(app: &mut App) -> (InputMode, bool) {
        let visible = app
            .world_mut()
            .query_filtered::<&CursorOptions, With<PrimaryWindow>>()
            .single(app.world())
            .unwrap()
            .visible;
        (*app.world().resource::<InputMode>(), visible)
    }

    #[test]
    fn system_cursor_follows_mode_frame_by_frame() {
        let mut app = app();
        assert_eq!(state(&mut app), (InputMode::Aim, false));

        // Hovering a button (the lobby's leave chip, say) frees the cursor; leaving it aims again.
        app.world_mut().resource_mut::<PointerOverUi>().0 = true;
        app.update();
        assert_eq!(state(&mut app), (InputMode::Ui, true));
        app.world_mut().resource_mut::<PointerOverUi>().0 = false;
        app.update();
        assert_eq!(state(&mut app), (InputMode::Aim, false));

        // Holding Alt is a manual release.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::AltLeft);
        app.update();
        assert_eq!(state(&mut app), (InputMode::Ui, true));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::AltLeft);
        app.update();
        assert_eq!(state(&mut app), (InputMode::Aim, false));

        // The backpack panel.
        app.world_mut().resource_mut::<BackpackOpen>().0 = true;
        app.update();
        assert_eq!(state(&mut app), (InputMode::Ui, true));
        app.world_mut().resource_mut::<BackpackOpen>().0 = false;
        app.update();
        assert_eq!(state(&mut app), (InputMode::Aim, false));

        // The pointer leaving the canvas.
        let mut windows = app
            .world_mut()
            .query_filtered::<&mut Window, With<PrimaryWindow>>();
        windows
            .single_mut(app.world_mut())
            .unwrap()
            .set_physical_cursor_position(None);
        app.update();
        assert_eq!(state(&mut app), (InputMode::Ui, true));
    }

    #[test]
    fn no_combatant_is_ui() {
        let mut app = app();
        let me = app
            .world_mut()
            .query_filtered::<Entity, With<LocalPlayer>>()
            .single(app.world())
            .unwrap();
        app.world_mut().despawn(me);
        app.update();
        assert_eq!(state(&mut app), (InputMode::Ui, true));
    }
}
