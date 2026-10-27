//! Camera follow and shake, and the cursor.

use super::*;

use crate::evo_panel::EvoPanelOpen;
use crate::input::{BackpackOpen, panel_open};
use crate::menu::MenuOpen;

pub(super) fn follow_camera(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    cursor: Res<CursorWorld>,
    backpack: Res<BackpackOpen>,
    menu: Res<MenuOpen>,
    evo: Res<EvoPanelOpen>,
    mut shake: ResMut<CameraShake>,
    mut held_lean: Local<Vec2>,
    me: Query<(&Pos, Option<&TickSamples>), With<LocalPlayer>>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
) {
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };
    let Ok((pos, samples)) = me.single() else {
        return;
    };
    let me = visual_pos(pos, samples, &fixed) + Vec2::Y * GUN_HEIGHT;
    // Lean toward the cursor. While a panel is open keep the last offset: don't recenter and
    // don't chase the pointer. Closing the panel feeds a new target into the lerp below.
    let lean = if panel_open(backpack.0, menu.0, evo.0) {
        *held_lean
    } else {
        let live = cursor
            .0
            .map(|c| ((c - me) * 0.22).clamp_length_max(160.0))
            .unwrap_or_default();
        *held_lean = live;
        live
    };
    let target = me + lean;
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
