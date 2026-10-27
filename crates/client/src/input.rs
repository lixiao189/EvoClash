//! Keyboard/mouse → `PlayerInput`, written into lightyear's input buffer every tick.
//!
//! The Esc stack lives here: it closes the backpack, then the evolution panel, then toggles the
//! menu, so one key never opens two overlays.

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use evoclash_game::items::{HOTBAR_SLOTS, MELEE_SLOT};
use evoclash_game::protocol::{Loadout, Player, PlayerInput, Pos};
use lightyear::prelude::client::input::InputSystems;
use lightyear::prelude::input::native::{ActionState, InputMarker};
use lightyear::prelude::*;

use crate::buttons::PointerOverUi;
use crate::evo_panel::EvoPanelOpen;
use crate::menu::MenuOpen;

/// Mouse position in world space, filled in by the renderer.
#[derive(Resource, Default)]
pub struct CursorWorld(pub Option<Vec2>);

/// How far above the feet guns are held and shots are drawn, in world pixels. Aim is measured
/// from here so that pointing at someone's chest on screen fires along the ground toward their feet.
pub const GUN_HEIGHT: f32 = 19.0;

/// The combatant this client controls.
#[derive(Component)]
pub struct LocalPlayer;

/// Whether the backpack panel (B) is open: clicks then belong to it, not to the weapon.
#[derive(Resource, Default)]
pub struct BackpackOpen(pub bool);

/// Trackpads scroll in pixels: this much counts as one wheel notch.
const SCROLL_NOTCH_PX: f32 = 60.0;

/// How long a hotbar request is shown before the replicated selection is trusted again.
const SELECT_HOLD: f32 = 0.5;

/// The hotbar slot you asked for. It is shown (HUD, drawn weapon, local shots) at once and
/// sent as `PlayerInput.select = (seq << 2) | slot`; the server acts when the value changes.
#[derive(Resource)]
pub struct Selection {
    seq: u8,
    slot: u8,
    /// When the request was made; `None` once the server has caught up or it timed out.
    asked: Option<f32>,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            // A reconnect resumes the old body, whose last request the server still remembers;
            // a random start keeps the first new request from repeating it.
            seq: rand::random::<u8>() & 0x3f,
            slot: MELEE_SLOT,
            asked: None,
        }
    }
}

impl Selection {
    fn request(&mut self, slot: u8, now: f32) {
        self.seq = (self.seq + 1) & 0x3f;
        self.slot = slot;
        self.asked = Some(now);
    }

    fn wire(&self) -> u8 {
        (self.seq << 2) | self.slot
    }

    /// The slot to show: the pending request, else what the server says.
    pub fn shown(&self, loadout: &Loadout) -> u8 {
        if self.asked.is_some() {
            self.slot
        } else {
            loadout.selected
        }
    }

    /// The loadout as it will look once the request lands.
    pub fn apply(&self, loadout: &Loadout) -> Loadout {
        Loadout {
            selected: self.shown(loadout),
            ..*loadout
        }
    }
}

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CursorWorld>()
            .init_resource::<BackpackOpen>()
            .init_resource::<Selection>();
        app.add_systems(Update, (claim_local_player, hotbar_keys));
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

fn hotbar_keys(
    time: Res<Time>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    scroll: Option<Res<AccumulatedMouseScroll>>,
    mut selection: ResMut<Selection>,
    mut open: ResMut<BackpackOpen>,
    mut evo: ResMut<EvoPanelOpen>,
    mut menu: ResMut<MenuOpen>,
    mut scrolled: Local<f32>,
    me: Query<&Loadout, With<LocalPlayer>>,
) {
    let now = time.elapsed_secs();
    if let (Some(asked), Ok(loadout)) = (selection.asked, me.single())
        && (loadout.selected == selection.slot || now - asked > SELECT_HOLD)
    {
        selection.asked = None;
    }
    let Some(keys) = keys else {
        return;
    };
    if keys.just_pressed(KeyCode::KeyB) && !menu.0 {
        open.0 = !open.0;
    }
    if keys.just_pressed(KeyCode::Escape) {
        if open.0 {
            open.0 = false;
        } else if evo.0 {
            evo.0 = false;
        } else if me.single().is_ok() {
            // No menu over the loading cover / join-failure screen.
            menu.0 = !menu.0;
        }
    }
    let Ok(loadout) = me.single() else {
        return;
    };
    let current = selection.shown(loadout);
    let mut want = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
    ]
    .iter()
    .position(|k| keys.just_pressed(*k))
    .map(|i| i as u8);
    let notch = match scroll.as_deref() {
        Some(s) if s.unit == MouseScrollUnit::Pixel => {
            *scrolled += s.delta.y;
            let notch = (*scrolled / SCROLL_NOTCH_PX).trunc();
            *scrolled -= notch * SCROLL_NOTCH_PX;
            notch
        }
        Some(s) => s.delta.y.signum() * f32::from(s.delta.y != 0.0),
        None => 0.0,
    };
    if notch != 0.0 {
        let step = if notch > 0.0 { HOTBAR_SLOTS - 1 } else { 1 };
        want = Some((current + step) % HOTBAR_SLOTS);
    }
    if let Some(slot) = want.filter(|s| *s != current) {
        selection.request(slot, now);
    }
}

fn axis(keys: &ButtonInput<KeyCode>, neg: [KeyCode; 2], pos: [KeyCode; 2]) -> i8 {
    keys.any_pressed(pos) as i8 - keys.any_pressed(neg) as i8
}

fn buffer_input(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    cursor: Res<CursorWorld>,
    selection: Res<Selection>,
    open: Res<BackpackOpen>,
    menu: Res<MenuOpen>,
    over_ui: Res<PointerOverUi>,
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
        // The mouse belongs to the backpack panel while it is open, and to the menu while that
        // is open: a click there must not fire or throw.
        let mouse = mouse.filter(|_| !open.0 && !menu.0);
        input.throw = keys.pressed(KeyQ)
            || mouse
                .as_ref()
                .is_some_and(|m| m.pressed(MouseButton::Right));
        // Clicking a panel button must not also shoot, and the menu blocks shooting outright.
        input.fire = !menu.0
            && !over_ui.0
            && (keys.pressed(KeyJ) || mouse.as_ref().is_some_and(|m| m.pressed(MouseButton::Left)));
        input.select = selection.wire();
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
