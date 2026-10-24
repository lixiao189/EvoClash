//! Keyboard/mouse → `PlayerInput`, written into lightyear's input buffer every tick.

use bevy::prelude::*;
use evoclash_game::protocol::{Player, PlayerInput, Pos};
use lightyear::prelude::client::input::InputSystems;
use lightyear::prelude::input::native::{ActionState, InputMarker};
use lightyear::prelude::*;

/// Mouse position in world space, filled in by the renderer.
#[derive(Resource, Default)]
pub struct CursorWorld(pub Option<Vec2>);

/// How far above the feet guns are held and shots are drawn, in world pixels. Aim is measured
/// from here so that pointing at someone's chest on screen fires along the ground toward their feet.
pub const GUN_HEIGHT: f32 = 26.0;

/// The combatant this client controls.
#[derive(Component)]
pub struct LocalPlayer;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CursorWorld>();
        app.add_systems(Update, claim_local_player);
        app.add_systems(
            FixedPreUpdate,
            buffer_input.in_set(InputSystems::WriteClientInputs),
        );
    }
}

fn claim_local_player(
    mut commands: Commands,
    players: Query<(Entity, &Player), (With<Controlled>, Without<LocalPlayer>)>,
) {
    for (entity, player) in &players {
        info!("controlling {} ({entity:?})", player.name);
        commands
            .entity(entity)
            .insert((LocalPlayer, InputMarker::<PlayerInput>::default()));
    }
}

fn axis(keys: &ButtonInput<KeyCode>, neg: [KeyCode; 2], pos: [KeyCode; 2]) -> i8 {
    keys.any_pressed(pos) as i8 - keys.any_pressed(neg) as i8
}

fn buffer_input(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    cursor: Res<CursorWorld>,
    mut query: Query<(&mut ActionState<PlayerInput>, &Pos), With<InputMarker<PlayerInput>>>,
) {
    let Ok((mut action, pos)) = query.single_mut() else {
        return;
    };
    let mut input = PlayerInput::default();

    if let Some(keys) = keys {
        use KeyCode::*;
        input.move_x = axis(&keys, [KeyA, ArrowLeft], [KeyD, ArrowRight]);
        input.move_y = axis(&keys, [KeyS, ArrowDown], [KeyW, ArrowUp]);
        input.dash = keys.any_pressed([Space, ShiftLeft]);
        input.interact = keys.any_pressed([KeyE, KeyF]);
        input.throw = keys.pressed(KeyQ)
            || mouse
                .as_ref()
                .is_some_and(|m| m.pressed(MouseButton::Right));
        input.fire =
            keys.pressed(KeyJ) || mouse.as_ref().is_some_and(|m| m.pressed(MouseButton::Left));
        // Perk pick while 1/2/3 is held; the server acts on the change, so holding picks once.
        input.pick = [Digit1, Digit2, Digit3]
            .iter()
            .position(|k| keys.pressed(*k))
            .map_or(0, |i| i as u8 + 1);
        input.skill_a = keys.pressed(KeyR);
        input.skill_b = keys.pressed(KeyT);
        input.aim = cursor
            .0
            .map(|c| (c - (pos.0 + Vec2::Y * GUN_HEIGHT)).normalize_or(Vec2::X))
            .unwrap_or(Vec2::X);
    }
    // Always write a value: an absent input means "packet lost", not "standing still".
    action.0 = input;
}
