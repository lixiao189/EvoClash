//! Crates, barrels and safes.

use super::*;

pub(super) fn prop_image(prop: &Prop, textures: &Textures) -> Handle<Image> {
    match prop.kind {
        PropKind::Crate => textures.crate_box.clone(),
        PropKind::Barrel => textures.barrel.clone(),
        PropKind::Safe if prop.open => textures.safe_open.clone(),
        PropKind::Safe => textures.safe.clone(),
    }
}

pub(super) fn dress_prop(
    trigger: On<Add<Prop>>,
    props: Query<(&Prop, &Pos)>,
    mut commands: Commands,
    textures: Res<Textures>,
) {
    let Ok((prop, pos)) = props.get(trigger.entity) else {
        return;
    };
    let (sprite, transform) = pixel_sprite(prop_image(prop, &textures));
    let body = commands
        .spawn((
            sprite,
            Anchor::BOTTOM_CENTER,
            transform.with_translation(Vec3::Y * -FOOT_INSET),
        ))
        .id();
    let r = prop.kind.radius();
    let shadow = commands
        .spawn(shadow(&textures, Vec2::new(r * 2.4, r * 1.1)))
        .id();
    commands
        .entity(trigger.entity)
        .insert((
            Transform::from_translation(pos.0.extend(depth(pos.0.y))),
            Visibility::Visible,
            PropBody(body),
        ))
        .add_children(&[shadow, body]);
}

/// Safes swing open; crates and barrels darken as they take a beating.
pub(super) fn sync_props(
    textures: Res<Textures>,
    props: Query<(&Prop, &Vitals, &PropBody), Or<(Changed<Prop>, Changed<Vitals>)>>,
    mut bodies: Query<&mut Sprite>,
) {
    for (prop, vitals, body) in &props {
        let Ok(mut sprite) = bodies.get_mut(body.0) else {
            continue;
        };
        sprite.image = prop_image(prop, &textures);
        let worn = 1.0 - (vitals.hp / prop.kind.max_hp()).clamp(0.0, 1.0);
        sprite.color = Color::WHITE.mix(&Color::srgb(0.45, 0.35, 0.3), worn * 0.7);
    }
}
