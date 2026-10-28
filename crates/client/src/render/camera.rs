//! Camera follow and shake, and the cursor.

use super::*;

use crate::evo_panel::EvoPanelOpen;
use crate::input::{BackpackOpen, panel_open};
use crate::input_mode::InputMode;
use crate::menu::MenuOpen;

pub(super) fn follow_camera(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    cursor: Res<CursorWorld>,
    backpack: Res<BackpackOpen>,
    menu: Res<MenuOpen>,
    evo: Res<EvoPanelOpen>,
    mode: Res<InputMode>,
    mut shake: ResMut<CameraShake>,
    mut held_lean: Local<Vec2>,
    me: Query<(&Pos, Option<&TickSamples>), With<LocalPlayer>>,
    specs: Query<&Spectator>,
    bodies: Query<(&Player, &Pos, Option<&TickSamples>)>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
) {
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };
    // Spectators sit on the followed combatant with no cursor lean.
    let subject = match me.single().ok() {
        Some((pos, samples)) => Some((
            visual_pos(pos, samples, &fixed) + Vec2::Y * GUN_HEIGHT,
            true,
        )),
        None => specs
            .iter()
            .next()
            .map(|s| s.following)
            .filter(|id| *id != 0)
            .and_then(|id| {
                bodies.iter().find_map(|(player, pos, samples)| {
                    (player.client_id == id).then(|| {
                        (
                            visual_pos(pos, samples, &fixed) + Vec2::Y * GUN_HEIGHT,
                            false,
                        )
                    })
                })
            }),
    };
    let Some((anchor, lean_ok)) = subject else {
        return;
    };
    // Lean toward the cursor. In Ui (a panel, or any other hold) keep the last offset: don't
    // recenter and don't chase the pointer. Leaving Ui feeds a new target into the lerp below.
    // A spectator never leans: the view stays on the followed body.
    let lean = if !lean_ok {
        *held_lean = Vec2::ZERO;
        Vec2::ZERO
    } else if panel_open(backpack.0, menu.0, evo.0) || *mode == InputMode::Ui {
        *held_lean
    } else {
        let live = cursor
            .0
            .map(|c| ((c - anchor) * 0.22).clamp_length_max(160.0))
            .unwrap_or_default();
        *held_lean = live;
        live
    };
    let target = anchor + lean;
    let current = cam.translation.truncate();
    let k = 1.0 - (-12.0 * time.delta_secs()).exp();
    let mut next = current.lerp(target, k);
    if shake.0 > 0.01 {
        let t = time.elapsed_secs() * 60.0;
        next += Vec2::new(t.sin(), (t * 1.3).cos()) * shake.0 * 9.0;
        shake.0 *= (-8.0 * time.delta_secs()).exp();
    }
    cam.translation = next.extend(cam.translation.z);
}

pub(super) fn update_cursor(
    mut cursor: ResMut<CursorWorld>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
) {
    let (Ok(window), Ok((camera, transform))) = (window.single(), camera.single()) else {
        return;
    };
    if let Some(screen) = window.cursor_position() {
        cursor.0 = camera.viewport_to_world_2d(transform, screen).ok();
    }
}
