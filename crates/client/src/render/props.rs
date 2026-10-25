//! Crates, barrels and safes.

use super::*;

/// LPC containers, drawn at one texture pixel per world pixel.
fn prop_image(prop: &Prop, lpc_sprites: &LpcSprites) -> Handle<Image> {
    match prop.kind {
        PropKind::Crate => lpc_sprites.crate_box.clone(),
        PropKind::Barrel => lpc_sprites.barrel.clone(),
        PropKind::Safe if prop.open => lpc_sprites.safe_open.clone(),
        PropKind::Safe => lpc_sprites.safe.clone(),
    }
}

/// Every barrel explodes, so they're painted red.
fn prop_color(kind: PropKind) -> Color {
    match kind {
        PropKind::Barrel => Color::srgb(1.0, 0.5, 0.42),
        _ => Color::WHITE,
    }
}

pub(super) fn dress_prop(
    trigger: On<Add<Prop>>,
    props: Query<(&Prop, &Pos)>,
    mut commands: Commands,
    textures: Res<Textures>,
    lpc_sprites: Res<LpcSprites>,
) {
    let Ok((prop, pos)) = props.get(trigger.entity) else {
        return;
    };
    let mut sprite = Sprite::from_image(prop_image(prop, &lpc_sprites));
    sprite.color = prop_color(prop.kind);
    let body = commands
        .spawn((
            sprite,
            Anchor::BOTTOM_CENTER,
            Transform::from_translation(Vec3::Y * -FOOT_INSET),
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
    lpc_sprites: Res<LpcSprites>,
    props: Query<(&Prop, &Vitals, &PropBody), Or<(Changed<Prop>, Changed<Vitals>)>>,
    mut bodies: Query<&mut Sprite>,
) {
    for (prop, vitals, body) in &props {
        let Ok(mut sprite) = bodies.get_mut(body.0) else {
            continue;
        };
        sprite.image = prop_image(prop, &lpc_sprites);
        let worn = 1.0 - (vitals.hp / prop.kind.max_hp()).clamp(0.0, 1.0);
        sprite.color = prop_color(prop.kind).mix(&Color::srgb(0.45, 0.35, 0.3), worn * 0.7);
    }
}
