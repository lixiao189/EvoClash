//! Spectator overlay: who we follow, where we sit in the queue, and the keys that cycle targets.
//!
//! A connection with [`Spectator`] has no combatant. Game keys do nothing here; Esc still
//! toggles the pause menu from `input`.

use bevy::prelude::*;
use evoclash_game::protocol::{CommandChannel, MatchState, Phase, Player, RoomCommand, Spectator};
use lightyear::prelude::{Client, MessageSender};

use crate::hud::{GOLD, PANEL_BG};
use crate::input::LocalPlayer;
use crate::menu::MenuOpen;
use crate::render::UiFont;

/// Shown on the loading cover and the reconnect card when the join seat is a spectator.
pub const SPECTATE_JOIN: &str = "对局进行中，先旁观，下一局上场";

pub struct SpectateUiPlugin;

impl Plugin for SpectateUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_spectate_hud);
        app.add_systems(Update, (sync_spectate_hud, cycle_target));
    }
}

/// Left / A and right / D cycle the followed combatant. Every other key is ignored.
fn spectate_command(key: KeyCode) -> Option<RoomCommand> {
    match key {
        KeyCode::ArrowLeft | KeyCode::KeyA => Some(RoomCommand::SpectatePrev),
        KeyCode::ArrowRight | KeyCode::KeyD => Some(RoomCommand::SpectateNext),
        _ => None,
    }
}

/// Watch line: name, an ideographic space, then the two arrow keys.
fn watch_line(name: &str) -> String {
    format!("旁观中：{name}\u{3000}← → 切换")
}

/// Queue copy under the watch line.
/// `None` when `queue_pos` is 0, or this client already has a lobby combat seat.
/// Results replaces the queue sentence with the short ready line.
fn queue_line(phase: Phase, queue_pos: u8, seated_in_lobby: bool) -> Option<String> {
    if queue_pos == 0 || seated_in_lobby {
        return None;
    }
    Some(if phase == Phase::Results {
        "即将上场".to_string()
    } else {
        format!("排队第 {queue_pos} 位，下一局上场")
    })
}

#[derive(Component)]
struct SpectateRoot;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum SpectateLine {
    Watch,
    Queue,
}

fn spawn_spectate_hud(mut commands: Commands, font: Res<UiFont>) {
    commands
        .spawn((
            SpectateRoot,
            Visibility::Hidden,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(18.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(2.0),
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
            ))
            .with_children(|card| {
                card.spawn((
                    SpectateLine::Watch,
                    Text::new(""),
                    font.sized(18.0),
                    TextColor(GOLD),
                    TextLayout::justify(Justify::Center),
                    TextShadow::default(),
                    Node::default(),
                ));
                card.spawn((
                    SpectateLine::Queue,
                    Text::new(""),
                    font.sized(15.0),
                    TextColor(Color::WHITE),
                    TextLayout::justify(Justify::Center),
                    TextShadow::default(),
                    Node::default(),
                ));
            });
        });
}

fn sync_spectate_hud(
    specs: Query<&Spectator>,
    players: Query<&Player>,
    me: Query<(), With<LocalPlayer>>,
    state: Query<&MatchState>,
    mut root: Query<&mut Visibility, With<SpectateRoot>>,
    mut lines: Query<(&SpectateLine, &mut Text, &mut Node)>,
) {
    let spec = specs.iter().next().copied();
    let phase = state.iter().next().map(|s| s.phase).unwrap_or(Phase::Lobby);
    // A combat seat in the lobby means this client is no longer waiting to spawn.
    let seated = !me.is_empty() && phase == Phase::Lobby;
    let (watch, queue) = match spec {
        Some(spec) => {
            let name = players
                .iter()
                .find(|player| player.client_id == spec.following)
                .map(|player| player.name.as_str())
                .unwrap_or("无人");
            let queue = queue_line(phase, spec.queue_pos, seated).unwrap_or_default();
            (watch_line(name), queue)
        }
        None => (String::new(), String::new()),
    };
    let show = if spec.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut root {
        visibility.set_if_neq(show);
    }
    for (kind, mut text, mut node) in &mut lines {
        let (value, on) = match kind {
            SpectateLine::Watch => (watch.as_str(), spec.is_some()),
            SpectateLine::Queue => (queue.as_str(), !queue.is_empty()),
        };
        let display = if on { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        if text.0 != value {
            text.0 = value.to_string();
        }
    }
}

fn cycle_target(
    menu: Res<MenuOpen>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    specs: Query<(), With<Spectator>>,
    mut senders: Query<&mut MessageSender<RoomCommand>, With<Client>>,
) {
    if menu.0 || specs.is_empty() {
        return;
    }
    let Some(keys) = keys else {
        return;
    };
    // One cycle per frame if both directions land together.
    let Some(cmd) = [
        KeyCode::ArrowLeft,
        KeyCode::KeyA,
        KeyCode::ArrowRight,
        KeyCode::KeyD,
    ]
    .into_iter()
    .find(|key| keys.just_pressed(*key))
    .and_then(spectate_command) else {
        return;
    };
    for mut sender in &mut senders {
        sender.send::<CommandChannel>(cmd.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectate_keys_map_to_cycle_commands() {
        assert_eq!(
            spectate_command(KeyCode::ArrowLeft),
            Some(RoomCommand::SpectatePrev)
        );
        assert_eq!(
            spectate_command(KeyCode::KeyA),
            Some(RoomCommand::SpectatePrev)
        );
        assert_eq!(
            spectate_command(KeyCode::ArrowRight),
            Some(RoomCommand::SpectateNext)
        );
        assert_eq!(
            spectate_command(KeyCode::KeyD),
            Some(RoomCommand::SpectateNext)
        );
        for key in [
            KeyCode::KeyW,
            KeyCode::KeyS,
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
            KeyCode::Space,
            KeyCode::Escape,
            KeyCode::KeyB,
            KeyCode::Tab,
            KeyCode::KeyE,
            KeyCode::KeyQ,
            KeyCode::Digit1,
        ] {
            assert_eq!(spectate_command(key), None, "{key:?}");
        }
    }

    #[test]
    fn watch_line_names_the_target_and_the_keys() {
        assert_eq!(watch_line("Ada"), "旁观中：Ada\u{3000}← → 切换");
        assert_eq!(watch_line("无人"), "旁观中：无人\u{3000}← → 切换");
    }

    #[test]
    fn queue_line_hides_at_zero_or_when_seated_and_changes_in_results() {
        assert_eq!(
            queue_line(Phase::Playing, 1, false).as_deref(),
            Some("排队第 1 位，下一局上场")
        );
        assert_eq!(
            queue_line(Phase::Lobby, 4, false).as_deref(),
            Some("排队第 4 位，下一局上场")
        );
        assert_eq!(
            queue_line(Phase::Starting, 2, false).as_deref(),
            Some("排队第 2 位，下一局上场")
        );
        assert_eq!(
            queue_line(Phase::Results, 2, false).as_deref(),
            Some("即将上场")
        );
        assert_eq!(queue_line(Phase::Playing, 0, false), None);
        assert_eq!(queue_line(Phase::Results, 0, false), None);
        assert_eq!(queue_line(Phase::Lobby, 1, true), None);
        assert_eq!(queue_line(Phase::Results, 3, true), None);
    }
}
