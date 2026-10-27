//! Boot/loading cover: hides the HUD skeleton until the local player exists.
//!
//! The DOM `#boot` block in `index.html` covers the gap before wasm starts; this Bevy overlay
//! takes over once the app is running and hides the DOM block. It stays up while the files in
//! [`PendingAssets`] load, shows the failure path (with a reload button) if one is missing, and
//! disappears for good once the session has spawned the [`LocalPlayer`] or a [`Spectator`].

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use evoclash_game::protocol::Spectator;

use crate::assets::{Loading, PendingAssets};
use crate::buttons::BlocksPointer;
use crate::connect_ui::{ConnectionStatus, LinkStatus};
use crate::hud::{GOLD, PANEL_BG};
use crate::input::LocalPlayer;
use crate::render::UiFont;
use crate::session::{JoinNotice, JoinOffers, SpectateJoin};
use crate::spectate_ui::SPECTATE_JOIN;

pub struct LoadingUiPlugin;

impl Plugin for LoadingUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CoverDismissed>();
        app.add_systems(Startup, spawn_cover);
        app.add_systems(
            Update,
            (
                hide_dom_boot_once,
                update_cover,
                animate_dots,
                style_reload,
                click_reload,
            )
                .chain(),
        );
    }
}

/// Set once the local player exists; the cover must never come back afterwards.
#[derive(Resource, Default)]
struct CoverDismissed(bool);

/// The full-screen cover root. Hidden (not despawned) so it can never reappear.
#[derive(Component)]
struct Cover;

#[derive(Component)]
enum CoverText {
    Status,
    Count,
    /// The asset path that failed to load.
    Path,
}

/// The animated `…` after the status while files are loading.
#[derive(Component)]
struct Dots;

#[derive(Component)]
#[require(RelativeCursorPosition)]
struct ReloadButton;

/// Error red, used for the failure text, the path and the reload button's border.
const ERROR: Color = Color::srgb(1.0, 0.38, 0.32);

fn line(font: &UiFont, size: f32, value: &str, color: Color) -> impl Bundle {
    (
        Text::new(value),
        font.sized(size),
        TextColor(color),
        TextLayout::justify(Justify::Center),
        TextShadow::default(),
    )
}

fn spawn_cover(mut commands: Commands, font: Res<UiFont>) {
    commands
        .spawn((
            Cover,
            BlocksPointer,
            GlobalZIndex(100),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.106, 0.09, 0.07, 0.98)),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::axes(Val::Px(36.0), Val::Px(28.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    max_width: Val::Px(560.0),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
                BorderColor::all(GOLD.with_alpha(0.5)),
            ))
            .with_children(|card| {
                card.spawn(line(&font, 52.0, "EVOCLASH", GOLD));
                card.spawn(Node {
                    align_items: AlignItems::Baseline,
                    column_gap: Val::Px(2.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        CoverText::Status,
                        line(&font, 18.0, "正在加载资源", Color::WHITE),
                    ));
                    row.spawn((Dots, line(&font, 18.0, "", GOLD)));
                });
                card.spawn((
                    CoverText::Count,
                    line(&font, 14.0, "", Color::srgba(1.0, 1.0, 1.0, 0.6)),
                ));
                card.spawn((CoverText::Path, line(&font, 14.0, "", ERROR)));
                card.spawn((
                    ReloadButton,
                    Node {
                        display: Display::None,
                        margin: UiRect::top(Val::Px(6.0)),
                        padding: UiRect::axes(Val::Px(18.0), Val::Px(7.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(PANEL_BG),
                    BorderColor::all(ERROR.with_alpha(0.8)),
                    children![(
                        Text::new("刷新页面"),
                        font.sized(16.0),
                        TextColor(Color::WHITE),
                        TextShadow::default(),
                    )],
                ));
            });
        });
}

/// `#boot` only covers the gap before the first frame; the Bevy cover is on screen by then.
fn hide_dom_boot_once(mut done: Local<bool>) {
    if *done {
        return;
    }
    *done = true;
    hide_dom_boot();
}

fn update_cover(
    pending: Res<PendingAssets>,
    me: Query<Entity, With<LocalPlayer>>,
    specs: Query<(), With<Spectator>>,
    notice: Res<JoinNotice>,
    offers: Res<JoinOffers>,
    link: Res<ConnectionStatus>,
    spectate: Res<SpectateJoin>,
    mut dismissed: ResMut<CoverDismissed>,
    mut cover: Query<&mut Visibility, With<Cover>>,
    mut texts: Query<(&CoverText, &mut Text, &mut TextColor)>,
    mut reload: Query<&mut Node, With<ReloadButton>>,
) {
    if dismissed.0 {
        return;
    }
    let failed = matches!(&pending.state, Loading::Failed(_));
    // The join/connection UI also lives outside `LocalPlayer` (a failed join, a dropped link
    // before the first spawn). Never sit on top of those buttons.
    let join_ui = !notice.0.is_empty() || !offers.buttons.is_empty();
    let reconnecting = matches!(link.status, LinkStatus::Reconnecting | LinkStatus::Failed);
    // Never hide while a file is missing: the error state has to stay readable.
    let has_player = !me.is_empty() || !specs.is_empty();
    if !failed && (has_player || join_ui || reconnecting) {
        // Only a spawned player makes the cover permanent. Join/reconnect UI can clear on a
        // retry, and the cover should come back for that attempt.
        if has_player {
            dismissed.0 = true;
        }
        if let Ok(mut visibility) = cover.single_mut() {
            visibility.set_if_neq(Visibility::Hidden);
        }
        return;
    }
    if let Ok(mut visibility) = cover.single_mut() {
        visibility.set_if_neq(Visibility::Visible);
    }

    let mut status = "正在加载资源";
    let mut status_color = Color::srgb(0.95, 0.92, 0.85);
    let mut count = String::new();
    let mut path = String::new();
    match &pending.state {
        Loading::Waiting => count = format!("资源 {} 项", pending.total()),
        Loading::Ready => {
            status = if spectate.0 {
                SPECTATE_JOIN
            } else {
                "正在连接对局…"
            };
            status_color = GOLD;
        }
        Loading::Failed(failed_path) => {
            status = "资源加载失败";
            status_color = ERROR;
            path = failed_path.clone();
        }
    }
    for (kind, mut text, mut color) in &mut texts {
        let (value, tint) = match kind {
            CoverText::Status => (status, status_color),
            CoverText::Count => (count.as_str(), Color::srgba(1.0, 1.0, 1.0, 0.6)),
            CoverText::Path => (path.as_str(), ERROR),
        };
        if text.0 != value {
            text.0 = value.to_string();
        }
        if color.0 != tint {
            color.0 = tint;
        }
    }
    for mut node in &mut reload {
        let display = if failed { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
}

fn animate_dots(
    time: Res<Time>,
    pending: Res<PendingAssets>,
    mut dots: Query<&mut Text, With<Dots>>,
) {
    let value = if pending.state == Loading::Waiting {
        ".".repeat((time.elapsed_secs() * 2.5) as usize % 3 + 1)
    } else {
        String::new()
    };
    for mut text in &mut dots {
        if text.0 != value {
            text.0 = value.clone();
        }
    }
}

fn style_reload(
    pending: Res<PendingAssets>,
    mut buttons: Query<
        (
            &RelativeCursorPosition,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<ReloadButton>,
    >,
) {
    let failed = matches!(&pending.state, Loading::Failed(_));
    for (cursor, mut background, mut border) in &mut buttons {
        let over = failed && cursor.cursor_over();
        background.0 = if over {
            GOLD.with_alpha(0.22)
        } else {
            PANEL_BG
        };
        border.set_if_neq(BorderColor::all(if over {
            GOLD
        } else {
            ERROR.with_alpha(0.8)
        }));
    }
}

fn click_reload(
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    pending: Res<PendingAssets>,
    buttons: Query<(&RelativeCursorPosition, &InheritedVisibility), With<ReloadButton>>,
) {
    if !matches!(&pending.state, Loading::Failed(_)) {
        return;
    }
    let Some(mouse) = mouse else {
        return;
    };
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let hit = buttons
        .iter()
        .any(|(cursor, visible)| visible.get() && cursor.cursor_over());
    if hit {
        reload_page();
    }
}

#[cfg(target_family = "wasm")]
fn hide_dom_boot() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    if let Some(boot) = document.get_element_by_id("boot") {
        let _ = boot.set_attribute("style", "display: none");
    }
}

#[cfg(not(target_family = "wasm"))]
fn hide_dom_boot() {}

#[cfg(target_family = "wasm")]
fn reload_page() {
    if let Some(window) = web_sys::window() {
        let _ = window.location().reload();
    }
}

#[cfg(not(target_family = "wasm"))]
fn reload_page() {}
