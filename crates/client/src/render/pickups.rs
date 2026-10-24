//! Loot lying on the ground.

use super::*;

pub(super) fn sync_pickups(
    mut pickups: Query<(&Pos, &mut Transform), (With<Pickup>, Changed<Pos>)>,
) {
    for (pos, mut transform) in &mut pickups {
        transform.translation = pos.0.extend(Z_PICKUP);
    }
}

pub(super) fn bob_pickups(time: Res<Time>, mut icons: Query<(&PickupIcon, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (icon, mut transform) in &mut icons {
        transform.translation.y = 6.0 + 3.0 * (t * 2.6 + icon.phase).sin();
        transform.rotation = Quat::from_rotation_z(0.12 * (t * 1.3 + icon.phase).sin());
    }
}
