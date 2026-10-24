//! Deployables from path skills: turrets and traps.

use super::*;

pub(super) fn dress_turret(
    trigger: On<Add<Turret>>,
    turrets: Query<Option<&Pos>>,
    mut commands: Commands,
    textures: Res<Textures>,
) {
    let pos = turrets
        .get(trigger.entity)
        .ok()
        .flatten()
        .map_or(Vec2::ZERO, |p| p.0);
    let (sprite, transform) = pixel_sprite(textures.turret_base.clone());
    let base = commands
        .spawn((
            sprite,
            Anchor::BOTTOM_CENTER,
            transform.with_translation(Vec3::Y * -FOOT_INSET),
        ))
        .id();
    let (sprite, transform) = pixel_sprite(textures.turret_head.clone());
    let head = commands
        .spawn((
            sprite,
            transform.with_translation(Vec3::new(0.0, 30.0, Z_STEP)),
        ))
        .id();
    let shadow = commands
        .spawn(shadow(&textures, Vec2::new(44.0, 20.0)))
        .id();
    commands
        .entity(trigger.entity)
        .insert((
            Transform::from_translation(pos.extend(depth(pos.y))),
            Visibility::Hidden,
            TurretHead(head),
        ))
        .add_children(&[shadow, base, head]);
}

pub(super) fn sync_turrets(
    arena: Res<Arena>,
    indoors: Res<Indoors>,
    mut turrets: Query<(&Pos, &Aim, &TurretHead, &mut Transform, &mut Visibility), With<Turret>>,
    mut heads: Query<(&mut Sprite, &mut Transform), Without<Turret>>,
) {
    for (pos, aim, head, mut transform, mut visibility) in &mut turrets {
        transform.translation = pos.0.extend(depth(pos.0.y));
        visibility.set_if_neq(if indoors.conceals(&arena, pos.0) {
            Visibility::Hidden
        } else {
            Visibility::Visible
        });
        if let Ok((mut sprite, mut head)) = heads.get_mut(head.0) {
            sprite.flip_y = aim.0.x < 0.0;
            head.rotation = Quat::from_rotation_z(aim.0.to_angle());
        }
    }
}

pub(super) fn dress_trap(trigger: On<Add<Trap>>, mut commands: Commands, textures: Res<Textures>) {
    let (sprite, transform) = pixel_sprite(textures.trap.clone());
    commands
        .entity(trigger.entity)
        .insert((sprite, transform, Visibility::Hidden));
}

/// Traps only ever reach their owner, but never draw someone else's anyway.
pub(super) fn sync_traps(
    time: Res<Time>,
    me: Query<&Player, With<LocalPlayer>>,
    mut traps: Query<(
        &Trap,
        Option<&Pos>,
        &mut Sprite,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    let me = me.single().ok().map(|p| p.client_id);
    let pulse = 0.6 + 0.2 * (time.elapsed_secs() * 3.0).sin();
    for (trap, pos, mut sprite, mut transform, mut visibility) in &mut traps {
        let Some(pos) = pos.filter(|_| Some(trap.owner) == me) else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        transform.translation = pos.0.extend(Z_DECAL + 1.0);
        sprite.color = Color::WHITE.with_alpha(pulse);
        visibility.set_if_neq(Visibility::Visible);
    }
}
