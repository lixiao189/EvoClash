//! Clickable UI shared by the evolution panel and the death screen: buttons that send a
//! `PlayerCommand`, their hover/disabled look, and whether the pointer is over any panel (so a
//! click on a button doesn't also fire).
//!
//! Hover comes from `RelativeCursorPosition` (filled by Bevy's UI focus pass); panels are
//! hidden with `Visibility`, never `Display::None`, so a hidden button's stale position can't
//! be clicked.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use evoclash_game::evolution::Evolution;
use evoclash_game::protocol::{CommandChannel, Vitals};
use evoclash_game::shop::{PlayerCommand, Wallet, can_deploy};
use lightyear::prelude::{Client, MessageSender};

use crate::input::LocalPlayer;

pub struct ButtonsPlugin;

impl Plugin for ButtonsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PointerOverUi>();
        app.add_systems(
            Update,
            (
                enable_buttons,
                style_buttons,
                click_buttons,
                pointer_over_ui,
            )
                .chain(),
        );
    }
}

/// True while the cursor is over a visible panel or button; input suppresses firing.
#[derive(Resource, Default, PartialEq)]
pub struct PointerOverUi(pub bool);

/// A panel root that swallows the pointer.
#[derive(Component)]
#[require(RelativeCursorPosition)]
pub struct BlocksPointer;

/// What clicking a button does.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Send(PlayerCommand),
    /// Return the cart's `i`th item.
    Unbuy(usize),
}

#[derive(Component)]
#[require(RelativeCursorPosition)]
pub struct UiButton {
    pub act: Act,
    /// Kept up to date from the local player's state by `enable_buttons`.
    pub enabled: bool,
}

impl UiButton {
    pub fn new(act: Act) -> Self {
        Self {
            act,
            enabled: false,
        }
    }
}

pub const BUTTON_BG: Color = Color::srgba(0.12, 0.12, 0.12, 0.85);
const HOVER_BG: Color = Color::srgba(0.3, 0.28, 0.2, 0.95);
const DISABLED_BG: Color = Color::srgba(0.08, 0.08, 0.08, 0.6);

/// A button node: padded, rounded, with a border the caller may recolor.
pub fn button(act: Act, node: Node) -> impl Bundle {
    (
        UiButton::new(act),
        Node {
            padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..node
        },
        BackgroundColor(BUTTON_BG),
        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.25)),
    )
}

/// Send one command to the server.
pub fn send(
    senders: &mut Query<&mut MessageSender<PlayerCommand>, With<Client>>,
    cmd: PlayerCommand,
) {
    for mut sender in senders.iter_mut() {
        sender.send::<CommandChannel>(cmd.clone());
    }
}

fn enable_buttons(
    me: Query<(&Evolution, &Wallet, &Vitals), With<LocalPlayer>>,
    mut buttons: Query<&mut UiButton>,
) {
    let Ok((evo, wallet, vitals)) = me.single() else {
        return;
    };
    let dead = !vitals.alive;
    for mut button in &mut buttons {
        let enabled = match &button.act {
            Act::Send(cmd) => match cmd {
                PlayerCommand::Spend(perk) => evo.can_spend(*perk),
                PlayerCommand::Refund(perk) => dead && evo.rank(*perk) > 0,
                PlayerCommand::ResetPerks => dead && evo.total_ranks() > 0,
                PlayerCommand::Buy(item) => dead && wallet.can_buy(*item, evo),
                PlayerCommand::Unbuy(item) => dead && wallet.cart.contains(item),
                PlayerCommand::Deploy => dead && can_deploy(vitals.respawn_in),
                PlayerCommand::DeployLast => dead,
            },
            Act::Unbuy(i) => dead && *i < wallet.cart.len(),
        };
        if button.enabled != enabled {
            button.enabled = enabled;
        }
    }
}

fn style_buttons(
    mut buttons: Query<
        (&UiButton, &RelativeCursorPosition, &mut BackgroundColor),
        Or<(Changed<UiButton>, Changed<RelativeCursorPosition>)>,
    >,
) {
    for (button, cursor, mut bg) in &mut buttons {
        bg.0 = match (button.enabled, cursor.cursor_over()) {
            (false, _) => DISABLED_BG,
            (true, true) => HOVER_BG,
            (true, false) => BUTTON_BG,
        };
    }
}

fn click_buttons(
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    me: Query<&Wallet, With<LocalPlayer>>,
    buttons: Query<(&UiButton, &RelativeCursorPosition, &InheritedVisibility)>,
    mut senders: Query<&mut MessageSender<PlayerCommand>, With<Client>>,
) {
    if !mouse.is_some_and(|m| m.just_pressed(MouseButton::Left)) {
        return;
    }
    let Some((button, ..)) = buttons
        .iter()
        .find(|(b, cursor, vis)| b.enabled && vis.get() && cursor.cursor_over())
    else {
        return;
    };
    let cmd = match &button.act {
        Act::Send(cmd) => cmd.clone(),
        Act::Unbuy(i) => match me.single().ok().and_then(|w| w.cart.get(*i)) {
            Some(item) => PlayerCommand::Unbuy(*item),
            None => return,
        },
    };
    send(&mut senders, cmd);
}

fn pointer_over_ui(
    mut over: ResMut<PointerOverUi>,
    nodes: Query<
        (&RelativeCursorPosition, &InheritedVisibility),
        Or<(With<BlocksPointer>, With<UiButton>)>,
    >,
) {
    let value = nodes
        .iter()
        .any(|(cursor, vis)| vis.get() && cursor.cursor_over());
    over.set_if_neq(PointerOverUi(value));
}
