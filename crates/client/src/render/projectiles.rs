//! Bullets, bolts and spores in flight.

use super::*;

/// Every projectile is a sprite: spores and EMP bolts glow, bullets are tinted streaks.
pub(super) fn dress_projectile(
    trigger: On<Add<Projectile>>,
    projectiles: Query<&Projectile>,
    mut commands: Commands,
    textures: Res<Textures>,
) {
    let Ok(projectile) = projectiles.get(trigger.entity) else {
        return;
    };
    let (sprite, anchor) = match projectile.kind {
        ProjectileKind::Spore => (
            Sprite {
                image: textures.spore.clone(),
                custom_size: Some(Vec2::splat(22.0)),
                ..default()
            },
            Anchor::CENTER,
        ),
        ProjectileKind::Acid => (
            Sprite {
                image: textures.acid.clone(),
                custom_size: Some(Vec2::splat(26.0)),
                ..default()
            },
            Anchor::CENTER,
        ),
        ProjectileKind::Bolt => (
            Sprite {
                image: textures.bolt.clone(),
                color: Color::srgb(0.45, 0.95, 1.0),
                custom_size: Some(Vec2::new(26.0, 7.0)),
                ..default()
            },
            Anchor::CENTER_RIGHT,
        ),
        ProjectileKind::Emp => (
            Sprite {
                image: textures.glow.clone(),
                color: Color::srgb(0.4, 0.8, 1.0),
                custom_size: Some(Vec2::splat(30.0)),
                ..default()
            },
            Anchor::CENTER,
        ),
        ProjectileKind::Shot(weapon) => (
            Sprite {
                image: textures.bolt.clone(),
                color: weapon.spec().color(),
                custom_size: Some(tracer_size(weapon)),
                ..default()
            },
            Anchor::CENTER_RIGHT,
        ),
    };
    commands.entity(trigger.entity).insert((
        sprite,
        anchor,
        Transform::from_xyz(0.0, 0.0, Z_PROJECTILE),
        Visibility::Hidden,
    ));
}

pub(super) fn sync_projectiles(
    mut projectiles: Query<(&Pos, &Projectile, &mut Transform, &mut Visibility), With<Sprite>>,
) {
    for (pos, projectile, mut transform, mut visibility) in &mut projectiles {
        transform.translation = (pos.0 + Vec2::Y * GUN_HEIGHT).extend(Z_PROJECTILE);
        if matches!(
            projectile.kind,
            ProjectileKind::Shot(_) | ProjectileKind::Bolt
        ) {
            transform.rotation = Quat::from_rotation_z(projectile.vel.to_angle());
        }
        visibility.set_if_neq(Visibility::Visible);
    }
}

/// Streak sprite size: the bright head sits at the right end, on the bullet.
pub fn tracer_size(weapon: WeaponKind) -> Vec2 {
    let (length, thickness) = match weapon {
        WeaponKind::Railgun => (170.0, 12.0),
        WeaponKind::Laser => (80.0, 8.0),
        WeaponKind::Rifle => (38.0, 7.0),
        WeaponKind::Bow => (32.0, 6.0),
        _ => (22.0, 6.0),
    };
    Vec2::new(length, thickness)
}
