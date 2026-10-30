//! Keyboard/mouse → `PlayerInput`, written into lightyear's input buffer every tick.
//!
//! [`InputMode`](crate::input_mode::InputMode) is Ui or Aim. In Ui the mouse does not fire or
//! throw and `aim` stays where it last was while aiming; movement keys still apply. The Esc
//! stack lives here: it closes the backpack, then the evolution panel, then toggles the menu,
//! so one key never opens two overlays. While any of those panels is open the buffered input
//! stands still (aim frozen, weapon unchanged); B, Tab and Esc still toggle the panels.

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use evoclash_game::evolution::seen_from;
use evoclash_game::items::{HOTBAR_SLOTS, MELEE_SLOT};
use evoclash_game::protocol::{Loadout, Monster, Player, PlayerInput, Pos, Spectator, Vitals};
use lightyear::prelude::client::input::InputSystems;
use lightyear::prelude::input::native::{ActionState, InputMarker};
use lightyear::prelude::*;

use crate::aim_assist;
use crate::buttons::PointerOverUi;
use crate::evo_panel::EvoPanelOpen;
use crate::input_mode::InputMode;
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

/// Backpack, evolution panel or pause menu. Aim, movement and the camera lean stay put while
/// this is true.
pub fn panel_open(backpack: bool, menu: bool, evo: bool) -> bool {
    backpack || menu || evo
}

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
    specs: Query<(), With<Spectator>>,
) {
    let now = time.elapsed_secs();
    if let (Some(asked), Ok(loadout)) = (selection.asked, me.single())
        && (loadout.selected == selection.slot || now - asked > SELECT_HOLD)
    {
        selection.asked = None;
    }
    // Spectators keep Esc. Weapon keys, the backpack and the evolution hint do not apply.
    if !specs.is_empty() {
        open.0 = false;
        evo.0 = false;
        if keys.is_some_and(|keys| keys.just_pressed(KeyCode::Escape)) {
            menu.0 = !menu.0;
        }
        return;
    }
    let Some(keys) = keys else {
        return;
    };
    // Captured before B / Esc so the press that closes a panel cannot also switch weapons.
    let panel_was_open = panel_open(open.0, menu.0, evo.0);
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
    // 1-4 and the wheel belong to the panels while one is up (including the frame it opens or
    // closes). Tab toggles the evolution panel in its own system, so treat that press the same.
    if panel_was_open || panel_open(open.0, menu.0, evo.0) || keys.just_pressed(KeyCode::Tab) {
        *scrolled = 0.0;
        return;
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

/// While a panel is open, stand still and keep the current weapon. `aim` stays at the last
/// direction sent before the panel opened (`Vec2::X` if none has been sent). A closed panel
/// passes `input` through and, when `sampled`, remembers its aim for the next lock.
fn apply_panel_lock(
    panel_open: bool,
    mut input: PlayerInput,
    sampled: bool,
    last_aim: &mut Vec2,
) -> PlayerInput {
    if !panel_open {
        if sampled {
            *last_aim = input.aim;
        }
        return input;
    }
    input.move_x = 0;
    input.move_y = 0;
    input.aim = if *last_aim == Vec2::ZERO {
        Vec2::X
    } else {
        *last_aim
    };
    input.fire = false;
    input.dash = false;
    input.interact = false;
    input.throw = false;
    input.skill_a = false;
    input.skill_b = false;
    input
}

/// What aim assist may bend the mouse aim toward: the aim points of the combatants and monsters
/// this client can actually see. Roofs hide a body through its `Visibility`; 隐身 hides it from
/// everyone past `STEALTH_SIGHT` ([`seen_from`]); 复活保护 targets are skipped since they cannot
/// be hurt yet.
fn aim_targets<'a>(
    origin: Vec2,
    players: impl Iterator<Item = (&'a Pos, &'a Vitals, &'a Visibility)>,
    monsters: impl Iterator<Item = (&'a Pos, &'a Vitals, &'a Visibility)>,
) -> impl Iterator<Item = Vec2> {
    let seen = move |pos: &Pos, vitals: &Vitals, vis: &Visibility| {
        vitals.alive
            && vitals.shield == 0
            && !matches!(vis, Visibility::Hidden)
            && seen_from(vitals, pos.0, origin)
    };
    let bodies = players.filter(move |&(pos, vitals, vis)| seen(pos, vitals, vis));
    let beasts = monsters.filter(move |&(pos, vitals, vis)| seen(pos, vitals, vis));
    // The same muzzle-height point the cursor direction is measured from.
    bodies
        .chain(beasts)
        .map(|(pos, _, _)| pos.0 + Vec2::Y * GUN_HEIGHT)
}

fn buffer_input(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    cursor: Res<CursorWorld>,
    selection: Res<Selection>,
    open: Res<BackpackOpen>,
    menu: Res<MenuOpen>,
    evo: Res<EvoPanelOpen>,
    over_ui: Res<PointerOverUi>,
    mode: Res<InputMode>,
    mut last_aim: Local<Vec2>,
    specs: Query<(), With<Spectator>>,
    targets: Query<(&Pos, &Vitals, &Visibility), (With<Player>, Without<LocalPlayer>)>,
    beasts: Query<(&Pos, &Vitals, &Visibility), With<Monster>>,
    mut query: Query<
        (&mut ActionState<PlayerInput>, &Pos, &Loadout),
        With<InputMarker<PlayerInput>>,
    >,
) {
    // A spectator must not emit `PlayerInput`. The server drops it; don't send it.
    if !specs.is_empty() {
        if let Ok((mut action, ..)) = query.single_mut() {
            action.0 = PlayerInput::default();
        }
        return;
    }
    let Ok((mut action, pos, loadout)) = query.single_mut() else {
        return;
    };
    let mut input = PlayerInput::default();
    let panel = panel_open(open.0, menu.0, evo.0);
    let ui = *mode == InputMode::Ui;
    // Don't sample aim in Ui: the next Aim frame still has the direction from before the hold.
    let sampled = keys.is_some() && !ui;

    if let Some(keys) = keys {
        use KeyCode::*;
        input.move_x = axis(&keys, [KeyA, ArrowLeft], [KeyD, ArrowRight]);
        input.move_y = axis(&keys, [KeyS, ArrowDown], [KeyW, ArrowUp]);
        input.dash = keys.any_pressed([Space, ShiftLeft]);
        input.interact = keys.any_pressed([KeyE, KeyF]);
        // Ui (panel, hovered control, Alt, unfocused, cursor outside) keeps mouse buttons from
        // firing or throwing. Keyboard fire / throw still work. `apply_panel_lock` freezes
        // movement only while a panel is open.
        let mouse = mouse.filter(|_| !open.0 && !menu.0 && !ui);
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
        if !ui {
            let origin = pos.0 + Vec2::Y * GUN_HEIGHT;
            // No cursor (outside the window, unfocused) means no assist either: only the mouse
            // aiming bends.
            input.aim = cursor.0.map_or(Vec2::X, |c| {
                let aimed = (c - origin).normalize_or(Vec2::X);
                // Only a gun in hand is assisted (a dry slot falls back to the melee weapon and
                // the throwable slot throws). Everything that draws or predicts a shot reads
                // this same `aim`, so the client's own tracers follow the pull.
                aim_assist::reach(selection.apply(loadout).weapon()).map_or(aimed, |range| {
                    aim_assist::assist(
                        aimed,
                        origin,
                        aim_targets(origin, targets.iter(), beasts.iter()),
                        range,
                    )
                })
            });
        }
    }
    if ui {
        input.aim = if *last_aim == Vec2::ZERO {
            Vec2::X
        } else {
            *last_aim
        };
    }
    if panel {
        // Keys may be missing; still keep the weapon the player already asked for.
        input.select = selection.wire();
    }
    // Always write a value: an absent input means "packet lost", not "standing still".
    // Held keys are read with `pressed`, so closing a panel resumes movement on the next tick.
    action.0 = apply_panel_lock(panel, input, sampled, &mut last_aim);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn busy() -> PlayerInput {
        PlayerInput {
            move_x: 1,
            move_y: -1,
            aim: Vec2::Y,
            fire: true,
            dash: true,
            interact: true,
            throw: true,
            skill_a: true,
            skill_b: true,
            select: 0b0000_1001,
        }
    }

    #[test]
    fn panel_lock_clears_actions_and_freezes_aim() {
        let live = busy();
        let mut last = Vec2::X;
        let locked = apply_panel_lock(true, live.clone(), true, &mut last);
        assert_eq!(
            locked,
            PlayerInput {
                select: live.select,
                aim: Vec2::X,
                ..PlayerInput::default()
            }
        );
        assert_eq!(last, Vec2::X);

        let resumed = apply_panel_lock(false, live.clone(), true, &mut last);
        assert_eq!(resumed, live);
        assert_eq!(last, Vec2::Y);

        let mut unset = Vec2::ZERO;
        let fallback = apply_panel_lock(true, live, true, &mut unset);
        assert_eq!(fallback.aim, Vec2::X);
        assert_eq!(unset, Vec2::ZERO);
    }

    /// `buffer_input` in a bare app: a combatant at the origin, the cursor to its right, the
    /// left button held. Returns the app after one frame in `mode`.
    fn buffering(mode: InputMode) -> App {
        let mut app = App::new();
        app.add_systems(Update, buffer_input)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Selection>()
            .init_resource::<BackpackOpen>()
            .init_resource::<MenuOpen>()
            .init_resource::<EvoPanelOpen>()
            .init_resource::<PointerOverUi>()
            .insert_resource(mode)
            .insert_resource(CursorWorld(Some(Vec2::new(200.0, GUN_HEIGHT))));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut().spawn((
            ActionState::<PlayerInput>::default(),
            InputMarker::<PlayerInput>::default(),
            Pos(Vec2::ZERO),
            Loadout::default(),
        ));
        app.update();
        app
    }

    fn sent(app: &mut App) -> PlayerInput {
        app.world_mut()
            .query::<&ActionState<PlayerInput>>()
            .single(app.world())
            .unwrap()
            .0
            .clone()
    }

    #[test]
    fn aim_mode_clicks_fire_and_ui_mode_clicks_do_not() {
        let mut app = buffering(InputMode::Aim);
        let aimed = sent(&mut app);
        assert!(aimed.fire);
        assert_eq!(aimed.aim, Vec2::X);

        // Hover a control: the held button stops firing and the aim stays put even though the
        // cursor moved above the player. Movement keys still apply.
        app.insert_resource(InputMode::Ui);
        app.insert_resource(CursorWorld(Some(Vec2::new(0.0, 500.0))));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyD);
        app.update();
        let held = sent(&mut app);
        assert!(!held.fire && !held.throw);
        assert_eq!(held.aim, Vec2::X);
        assert_eq!(held.move_x, 1);

        // Back to Aim: the new cursor direction is used at once.
        app.insert_resource(InputMode::Aim);
        app.update();
        let resumed = sent(&mut app);
        assert!(resumed.fire);
        assert!(resumed.aim.y > 0.99);
    }
}
