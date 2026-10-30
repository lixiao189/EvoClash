//! Reconnect overlay: shown while the session is trying to get back into a match.
//!
//! `ConnectionStatus` is the interface the session writes to; the overlay only reads it, so
//! the session never has to know about UI nodes.
//!
//! The overlay never pauses the game (it is multiplayer) and never touches input: it only
//! swallows the pointer with [`BlocksPointer`] and hit-tests its own 返回首页 button by hand,
//! like the results bar in `hud.rs`.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use crate::buttons::BlocksPointer;
use crate::hud::{GOLD, PANEL_BG};
use crate::render::UiFont;
use crate::session::{Depart, SpectateJoin};
use crate::spectate_ui::SPECTATE_JOIN;

/// What the session's connection loop is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkStatus {
    /// Before the first join request.
    #[default]
    Idle,
    /// HTTP join in flight or the first WebTransport handshake.
    Connecting,
    /// A live client link.
    Connected,
    /// The link dropped and the session is retrying.
    Reconnecting,
    /// The session ran out of attempts.
    Failed,
}

/// Shared between the session (writer) and this overlay (reader).
#[derive(Resource, Debug, Default)]
pub struct ConnectionStatus {
    pub status: LinkStatus,
    /// Reconnect attempts started so far.
    pub attempts: u32,
    /// Seconds the link has been down (0 while it is up).
    pub down_secs: u32,
    /// Reconnect budget in seconds: the session gives up after this long, not after N tries.
    pub budget_secs: u32,
}

pub struct ConnectUiPlugin;

impl Plugin for ConnectUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConnectionStatus>();
        app.init_resource::<OverlayState>();
        app.add_systems(Startup, spawn_overlay);
        // Sync first so the click below can leave the overlay hidden for the rest of the
        // frame; 返回首页 only records the departure.
        app.add_systems(
            Update,
            (sync_overlay, style_back_button, click_back).chain(),
        );
    }
}

/// The 返回首页 button was pressed; keep the overlay hidden until the page leaves.
#[derive(Resource, Default)]
struct OverlayState {
    dismissed: bool,
}

/// Nodes whose visibility `sync_overlay` owns.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum OverlayNode {
    Root,
    /// 连接中断，正在重连…
    Reconnecting,
    /// 连接失败.
    Failed,
}

/// The lines that change while reconnecting.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum OverlayText {
    Title,
    Attempt,
    /// Failed-card subtitle. Swapped for the spectate join line when that seat is one.
    Detail,
}

#[derive(Component)]
#[require(RelativeCursorPosition)]
struct BackHomeButton;

const SCRIM: Color = Color::srgba(0.0, 0.0, 0.0, 0.72);
const HOVER_BG: Color = Color::srgba(0.3, 0.28, 0.2, 0.95);

fn label(font: &UiFont, size: f32, value: &str, color: Color) -> impl Bundle {
    (
        Text::new(value),
        font.sized(size),
        TextColor(color),
        TextShadow::default(),
    )
}

/// The centered dark card both states share.
fn card() -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(10.0),
            padding: UiRect::all(Val::Px(24.0)),
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
        BorderColor::all(GOLD),
    )
}

fn spawn_overlay(mut commands: Commands, font: Res<UiFont>) {
    let font = &*font;
    commands
        .spawn((
            OverlayNode::Root,
            BlocksPointer,
            Visibility::Hidden,
            GlobalZIndex(45),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(SCRIM),
        ))
        .with_children(|root| {
            root.spawn((OverlayNode::Reconnecting, Visibility::Hidden, card()))
                .with_children(|card| {
                    card.spawn((
                        OverlayText::Title,
                        label(font, 26.0, "连接中断，正在重连…", GOLD),
                    ));
                    card.spawn((
                        OverlayText::Attempt,
                        label(font, 18.0, "第 0/0 次", Color::WHITE),
                    ));
                });
            root.spawn((OverlayNode::Failed, Visibility::Hidden, card()))
                .with_children(|card| {
                    card.spawn(label(font, 26.0, "连接失败", GOLD));
                    card.spawn((
                        OverlayText::Detail,
                        label(font, 18.0, "对局仍在服务器上进行", Color::WHITE),
                    ));
                    card.spawn((
                        BackHomeButton,
                        Node {
                            margin: UiRect::top(Val::Px(6.0)),
                            padding: UiRect::axes(Val::Px(18.0), Val::Px(8.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(PANEL_BG),
                        BorderColor::all(GOLD),
                        children![(
                            Text::new("返回首页"),
                            font.sized(18.0),
                            TextColor(GOLD),
                            TextShadow::default(),
                        )],
                    ));
                });
        });
}

fn sync_overlay(
    time: Res<Time>,
    status: Res<ConnectionStatus>,
    spectate: Res<SpectateJoin>,
    mut state: ResMut<OverlayState>,
    mut nodes: Query<(&OverlayNode, &mut Visibility)>,
    mut texts: Query<(&OverlayText, &mut Text)>,
) {
    // A fresh link starts a new episode; a late Reconnecting must not undo 返回首页.
    if state.dismissed
        && matches!(
            status.status,
            LinkStatus::Idle | LinkStatus::Connecting | LinkStatus::Connected
        )
    {
        state.dismissed = false;
    }
    let reconnect = status.status == LinkStatus::Reconnecting && !state.dismissed;
    let failed = status.status == LinkStatus::Failed && !state.dismissed;
    let show = reconnect || failed;
    for (node, mut visibility) in &mut nodes {
        let visible = match node {
            OverlayNode::Root => show,
            OverlayNode::Reconnecting => reconnect,
            OverlayNode::Failed => failed,
        };
        visibility.set_if_neq(if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    if !show {
        return;
    }
    // One dot per beat, then back to one: 正在重连… / …… / ……….
    let dots = 1 + (time.elapsed_secs() * 2.5) as usize % 3;
    let title = format!("连接中断，正在重连{}", "…".repeat(dots));
    let line = format!(
        "第 {} 次，已等 {} 秒（最多 {} 秒）",
        status.attempts, status.down_secs, status.budget_secs
    );
    let attempt = if spectate.0 {
        format!("{line}\n{SPECTATE_JOIN}")
    } else {
        line
    };
    let detail = if spectate.0 {
        SPECTATE_JOIN
    } else {
        "对局仍在服务器上进行"
    };
    for (kind, mut text) in &mut texts {
        let value = match kind {
            OverlayText::Title => title.as_str(),
            OverlayText::Attempt => attempt.as_str(),
            OverlayText::Detail => detail,
        };
        if text.0 != value {
            text.0 = value.to_string();
        }
    }
}

fn style_back_button(
    mut buttons: Query<
        (
            &RelativeCursorPosition,
            &mut BackgroundColor,
            &InheritedVisibility,
        ),
        With<BackHomeButton>,
    >,
) {
    for (cursor, mut background, visible) in &mut buttons {
        let color = if visible.get() && cursor.cursor_over() {
            HOVER_BG
        } else {
            PANEL_BG
        };
        if background.0 != color {
            background.0 = color;
        }
    }
}

fn click_back(
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    status: Res<ConnectionStatus>,
    mut state: ResMut<OverlayState>,
    buttons: Query<(&RelativeCursorPosition, &InheritedVisibility), With<BackHomeButton>>,
    mut nodes: Query<&mut Visibility, With<OverlayNode>>,
    mut depart: ResMut<Depart>,
) {
    if status.status != LinkStatus::Failed || state.dismissed {
        return;
    }
    if !mouse.is_some_and(|mouse| mouse.just_pressed(MouseButton::Left)) {
        return;
    }
    let hit = buttons
        .iter()
        .any(|(cursor, visible)| visible.get() && cursor.cursor_over());
    if !hit {
        return;
    }
    state.dismissed = true;
    for mut visibility in &mut nodes {
        visibility.set_if_neq(Visibility::Hidden);
    }
    depart.request();
}
