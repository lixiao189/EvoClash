//! In-game menu overlay (Esc): resume, controls help, leave the match.
//!
//! The game does not pause (it is multiplayer); the overlay only blocks pointer input. The
//! session's input stack owns the Esc key and toggles [`MenuOpen`], closing the backpack and
//! the evolution panel first.
//!
//! UI picking is not compiled into the client, so buttons are hit-tested by hand with
//! `RelativeCursorPosition`, like the HUD's results bar.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use evoclash_game::protocol::{CommandChannel, MatchState, Phase, RoomCommand};
use lightyear::prelude::{Client, MessageSender};

use crate::buttons::BlocksPointer;
use crate::connect_ui::{ConnectionStatus, LinkStatus};
use crate::hud::{GOLD, PANEL_BG};
use crate::render::UiFont;
use crate::session::Depart;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuOpen>()
            .init_resource::<MenuPage>()
            .add_systems(Startup, spawn_menu)
            .add_systems(
                Update,
                (
                    close_in_transition,
                    click_buttons,
                    sync_pages,
                    style_buttons,
                    update_subtitle,
                )
                    .chain(),
            );
    }
}

/// Whether the Esc menu is open.
#[derive(Resource, Default)]
pub struct MenuOpen(pub bool);

/// Which page the overlay shows. Reset to `Main` whenever the menu closes.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
enum MenuPage {
    #[default]
    Main,
    Help,
    ConfirmLeave,
}

#[derive(Component)]
struct MenuRoot;

/// A page container; exactly one is visible while the menu is open.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
struct MenuPageRoot(MenuPage);

/// "对局仍在继续" / "等待大厅" under the title.
#[derive(Component)]
struct Subtitle;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    Resume,
    Help,
    Back,
    Leave,
    ConfirmLeave,
    Cancel,
}

/// A button and the page it belongs to. The page guards against a hidden page's stale
/// `RelativeCursorPosition` (the focus pass skips hidden nodes without clearing it).
#[derive(Component)]
#[require(RelativeCursorPosition)]
struct MenuButton {
    action: MenuAction,
    page: MenuPage,
}

const SCRIM: Color = Color::srgba(0.0, 0.0, 0.0, 0.65);
const BUTTON_BG: Color = Color::srgba(0.12, 0.12, 0.12, 0.85);
const HOVER_BG: Color = Color::srgba(0.3, 0.28, 0.2, 0.95);
const EDGE: Color = Color::srgba(1.0, 1.0, 1.0, 0.25);
const DIM: Color = Color::srgba(0.85, 0.85, 0.85, 0.85);
const WIDTH: f32 = 360.0;

fn text(font: &UiFont, size: f32, value: &str, color: Color) -> impl Bundle {
    (
        Text::new(value),
        font.sized(size),
        TextColor(color),
        TextShadow::default(),
    )
}

fn centered_text(font: &UiFont, size: f32, value: &str, color: Color) -> impl Bundle {
    (
        text(font, size, value, color),
        TextLayout::justify(Justify::Center),
        Node {
            align_self: AlignSelf::Center,
            ..default()
        },
    )
}

fn menu_button(action: MenuAction, page: MenuPage, font: &UiFont, label: &str) -> impl Bundle {
    (
        MenuButton { action, page },
        BlocksPointer,
        Node {
            padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
            justify_content: JustifyContent::Center,
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(BUTTON_BG),
        BorderColor::all(EDGE),
        children![(
            Text::new(label),
            font.sized(17.0),
            TextColor(Color::WHITE),
            TextShadow::default(),
        )],
    )
}

fn page_node() -> Node {
    Node {
        width: Val::Percent(100.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(8.0),
        ..default()
    }
}

fn spawn_menu(mut commands: Commands, font: Res<UiFont>) {
    let font = &*font;
    commands
        .spawn((
            MenuRoot,
            BlocksPointer,
            Visibility::Hidden,
            GlobalZIndex(30),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(SCRIM),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Px(WIDTH),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    padding: UiRect::axes(Val::Px(18.0), Val::Px(16.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
                BorderColor::all(GOLD.with_alpha(0.5)),
            ))
            .with_children(|panel| {
                spawn_main(panel, font);
                spawn_help(panel, font);
                spawn_confirm(panel, font);
            });
        });
}

fn spawn_main(panel: &mut ChildSpawnerCommands, font: &UiFont) {
    panel
        .spawn((MenuPageRoot(MenuPage::Main), page_node()))
        .with_children(|page| {
            page.spawn(centered_text(font, 26.0, "菜单", GOLD));
            page.spawn((Subtitle, centered_text(font, 14.0, "", DIM)));
            page.spawn(menu_button(
                MenuAction::Resume,
                MenuPage::Main,
                font,
                "继续游戏",
            ));
            page.spawn(menu_button(
                MenuAction::Help,
                MenuPage::Main,
                font,
                "操作说明",
            ));
            page.spawn(menu_button(
                MenuAction::Leave,
                MenuPage::Main,
                font,
                "离开对局",
            ));
            page.spawn(centered_text(font, 12.0, "Esc 继续游戏", DIM));
        });
}

fn spawn_help(panel: &mut ChildSpawnerCommands, font: &UiFont) {
    panel
        .spawn((MenuPageRoot(MenuPage::Help), page_node()))
        .with_children(|page| {
            page.spawn(centered_text(font, 22.0, "操作说明", GOLD));
            for line in [
                "WASD 移动 · 空格/Shift 冲刺",
                "鼠标瞄准 · 左键/J 射击",
                "E 交互/长按 · Q/右键 电磁雷",
                "1-4 或滚轮 换武器 · B 背包",
                "Tab 进化 · R/T 技能",
                "Esc 打开或关闭本菜单",
            ] {
                page.spawn(text(font, 14.0, line, DIM));
            }
            page.spawn(menu_button(MenuAction::Back, MenuPage::Help, font, "返回"));
        });
}

fn spawn_confirm(panel: &mut ChildSpawnerCommands, font: &UiFont) {
    panel
        .spawn((MenuPageRoot(MenuPage::ConfirmLeave), page_node()))
        .with_children(|page| {
            page.spawn(centered_text(font, 22.0, "确认离开本局？", GOLD));
            page.spawn(centered_text(
                font,
                14.0,
                "会回到名字页面，本局进度不会保留。",
                DIM,
            ));
            page.spawn(menu_button(
                MenuAction::ConfirmLeave,
                MenuPage::ConfirmLeave,
                font,
                "确认离开",
            ));
            page.spawn(menu_button(
                MenuAction::Cancel,
                MenuPage::ConfirmLeave,
                font,
                "取消",
            ));
        });
}

/// The server never pauses, so the overlay is only valid outside the between-round shades.
/// A reconnect overlay takes over the screen too; close the menu under it.
fn close_in_transition(
    state: Query<&MatchState>,
    link: Res<ConnectionStatus>,
    mut open: ResMut<MenuOpen>,
) {
    let transitioning = state
        .iter()
        .any(|s| matches!(s.phase, Phase::Starting | Phase::Recovering));
    let reconnecting = matches!(link.status, LinkStatus::Reconnecting | LinkStatus::Failed);
    if (transitioning || reconnecting) && open.0 {
        open.0 = false;
    }
}

fn sync_pages(
    open: Res<MenuOpen>,
    mut page: ResMut<MenuPage>,
    mut roots: Query<&mut Visibility, With<MenuRoot>>,
    mut pages: Query<(&MenuPageRoot, &mut Visibility), Without<MenuRoot>>,
) {
    if !open.0 {
        *page = MenuPage::Main;
    }
    for mut vis in &mut roots {
        vis.set_if_neq(shown(open.0));
    }
    for (root, mut vis) in &mut pages {
        vis.set_if_neq(shown(open.0 && root.0 == *page));
    }
}

fn style_buttons(
    open: Res<MenuOpen>,
    page: Res<MenuPage>,
    mut buttons: Query<(
        &MenuButton,
        &RelativeCursorPosition,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
) {
    for (button, cursor, mut bg, mut border) in &mut buttons {
        let hovered = open.0 && button.page == *page && cursor.cursor_over();
        bg.set_if_neq(BackgroundColor(if hovered { HOVER_BG } else { BUTTON_BG }));
        border.set_if_neq(BorderColor::all(if hovered { GOLD } else { EDGE }));
    }
}

fn click_buttons(
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    mut open: ResMut<MenuOpen>,
    mut page: ResMut<MenuPage>,
    buttons: Query<(&MenuButton, &RelativeCursorPosition)>,
    mut senders: Query<&mut MessageSender<RoomCommand>, With<Client>>,
    mut depart: ResMut<Depart>,
) {
    if !open.0 || !mouse.is_some_and(|m| m.just_pressed(MouseButton::Left)) {
        return;
    }
    let Some((button, _)) = buttons
        .iter()
        .find(|(button, cursor)| button.page == *page && cursor.cursor_over())
    else {
        return;
    };
    match button.action {
        MenuAction::Resume => open.0 = false,
        MenuAction::Help => *page = MenuPage::Help,
        MenuAction::Back | MenuAction::Cancel => *page = MenuPage::Main,
        MenuAction::Leave => *page = MenuPage::ConfirmLeave,
        MenuAction::ConfirmLeave => {
            // `Depart` keeps the link open long enough for this flush; with no link it still
            // returns us to the name form.
            for mut sender in &mut senders {
                sender.send::<CommandChannel>(RoomCommand::Leave);
            }
            depart.request();
            open.0 = false;
        }
    }
}

fn update_subtitle(state: Query<&MatchState>, mut texts: Query<&mut Text, With<Subtitle>>) {
    let phase = state.iter().next().map(|s| s.phase).unwrap_or_default();
    let value = match phase {
        Phase::Lobby => "等待大厅",
        Phase::Results => "对局已结束",
        _ => "对局仍在继续",
    };
    for mut text in &mut texts {
        if text.0 != value {
            text.0 = value.to_string();
        }
    }
}

fn shown(on: bool) -> Visibility {
    if on {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}
