//! The evolution tree: three path columns of four perks. `Tab` toggles it while alive as a small
//! panel (the game keeps running; click a perk to spend a point); the death screen shows the
//! same tree with +/- buttons for a free respec (`spawn_tree(.., true)`).

use bevy::prelude::*;
use evoclash_game::evolution::{EvoPath, Evolution, MAX_RANK, Perk, SKILL_A_RANK, SKILL_B_RANK};
use evoclash_game::protocol::Vitals;
use evoclash_game::shop::PlayerCommand;

use crate::buttons::{Act, BlocksPointer, button};
use crate::hud::{GOLD, LOCKED, PANEL_BG, path_color};
use crate::input::LocalPlayer;
use crate::menu::MenuOpen;
use crate::perf::{EvoSelectSent, SlowWarn, take_select_rtt};
use crate::render::UiFont;

pub struct EvoPanelPlugin;

impl Plugin for EvoPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EvoPanelOpen>();
        app.init_resource::<EvoSelectSent>();
        app.add_systems(Startup, spawn_panel);
        app.add_systems(Update, (toggle_panel, update_tree));
    }
}

/// Whether the alive-time panel is open (the HUD hides its "Tab 加点" hint then).
#[derive(Resource, Default)]
pub struct EvoPanelOpen(pub bool);

#[derive(Component)]
struct EvoPanel;

/// Parts of a tree; every tree on screen is updated from the local player's `Evolution`.
#[derive(Component, Clone, Copy)]
enum TreeUi {
    Header,
    PathHead(EvoPath),
    Card(Perk),
    PerkName(Perk),
}

const DESC: Color = Color::srgba(0.85, 0.85, 0.85, 0.85);
const CARD_WIDTH: f32 = 176.0;

fn text(font: &UiFont, size: f32, value: impl Into<String>, color: Color) -> impl Bundle {
    (
        Text::new(value),
        font.sized(size),
        TextColor(color),
        TextShadow::default(),
    )
}

fn card_node() -> Node {
    Node {
        width: Val::Px(CARD_WIDTH),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(1.0),
        ..default()
    }
}

/// Spawn a tree under `parent`. With `respec`, each perk gets + and - buttons (death screen);
/// without, the perk card itself is the spend button.
pub fn spawn_tree(parent: &mut ChildSpawnerCommands, font: &UiFont, respec: bool) {
    parent.spawn((TreeUi::Header, text(font, 14.0, "", GOLD)));
    parent
        .spawn(Node {
            column_gap: Val::Px(10.0),
            ..default()
        })
        .with_children(|cols| {
            for path in EvoPath::ALL {
                cols.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|col| {
                    col.spawn((
                        TreeUi::PathHead(path),
                        text(font, 16.0, "", path_color(path)),
                    ));
                    for perk in path.perks() {
                        spawn_perk(col, font, perk, respec);
                    }
                });
            }
        });
}

fn spawn_perk(col: &mut ChildSpawnerCommands, font: &UiFont, perk: Perk, respec: bool) {
    let fill = |card: &mut ChildSpawnerCommands| {
        card.spawn((TreeUi::PerkName(perk), text(font, 14.0, "", Color::WHITE)));
        card.spawn(text(font, 11.0, perk.desc(), DESC));
    };
    if !respec {
        col.spawn((
            TreeUi::Card(perk),
            button(Act::Send(PlayerCommand::Spend(perk)), card_node()),
        ))
        .with_children(fill);
        return;
    }
    col.spawn(Node {
        column_gap: Val::Px(4.0),
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|row| {
        row.spawn((
            TreeUi::Card(perk),
            Node {
                padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..card_node()
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(LOCKED),
        ))
        .with_children(fill);
        row.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(3.0),
            ..default()
        })
        .with_children(|pm| {
            for (label, cmd) in [
                ("+", PlayerCommand::Spend(perk)),
                ("-", PlayerCommand::Refund(perk)),
            ] {
                pm.spawn(button(
                    Act::Send(cmd),
                    Node {
                        width: Val::Px(26.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                ))
                .with_child(text(font, 15.0, label, Color::WHITE));
            }
        });
    });
}

fn spawn_panel(mut commands: Commands, font: Res<UiFont>) {
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(128.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                EvoPanel,
                BlocksPointer,
                Visibility::Hidden,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
                BorderColor::all(GOLD.with_alpha(0.5)),
            ))
            .with_children(|panel| spawn_tree(panel, &font, false));
        });
}

fn toggle_panel(
    menu: Res<MenuOpen>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    me: Query<&Vitals, With<LocalPlayer>>,
    mut open: ResMut<EvoPanelOpen>,
    mut panel: Query<&mut Visibility, With<EvoPanel>>,
) {
    let alive = me.single().is_ok_and(|v| v.alive);
    if keys.is_some_and(|k| k.just_pressed(KeyCode::Tab)) && alive && !menu.0 {
        open.0 = !open.0;
    }
    if !alive && open.0 {
        open.0 = false;
    }
    for mut vis in &mut panel {
        vis.set_if_neq(if open.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

/// "R 技能名 已解锁" or "R 技能名 需 2 点".
fn gate(key: &str, name: &str, need: u8, rank: u8) -> String {
    if rank >= need {
        format!("{key} {name} 已解锁")
    } else {
        format!("{key} {name} 需 {need} 点")
    }
}

fn update_tree(
    mut select_sent: ResMut<EvoSelectSent>,
    me: Query<&Evolution, (With<LocalPlayer>, Changed<Evolution>)>,
    mut ui: Query<(
        &TreeUi,
        Option<&mut Text>,
        Option<&mut TextColor>,
        Option<&mut BorderColor>,
    )>,
) {
    let _probe = SlowWarn::new("update_tree");
    // The trees are spawned at startup, before the player (whose arrival counts as a change).
    let Ok(evo) = me.single() else {
        return;
    };
    // Click-to-replication wait, not the cost of this system. Cleared even when it was fast.
    take_select_rtt(&mut select_sent);
    for (kind, text, color, border) in &mut ui {
        let set = |value: String, c: Color| {
            if let Some(mut text) = text {
                text.set_if_neq(Text::new(value));
            }
            if let Some(mut color) = color {
                color.set_if_neq(TextColor(c));
            }
        };
        match *kind {
            TreeUi::Header => {
                let skills = match evo.lead_path() {
                    Some(path) => {
                        let rank = evo.path_rank(path);
                        let (a, b) = path.skills();
                        format!(
                            "领先：{} {rank} 点 · {} · {}",
                            path.name(),
                            gate("R", a.name(), SKILL_A_RANK, rank),
                            gate("T", b.name(), SKILL_B_RANK, rank),
                        )
                    }
                    None => {
                        format!("领先路线 {SKILL_A_RANK} 点解锁技能 R，{SKILL_B_RANK} 点解锁技能 T")
                    }
                };
                set(format!("进化点 {}    {skills}", evo.points), GOLD);
            }
            TreeUi::PathHead(path) => {
                set(
                    format!("{}  {} 点", path.name(), evo.path_rank(path)),
                    path_color(path),
                );
            }
            TreeUi::Card(perk) => {
                if let Some(mut border) = border {
                    let c = if evo.rank(perk) > 0 {
                        path_color(perk.evo_path())
                    } else {
                        LOCKED
                    };
                    border.set_if_neq(BorderColor::all(c));
                }
            }
            TreeUi::PerkName(perk) => {
                let rank = evo.rank(perk);
                let c = if rank > 0 {
                    path_color(perk.evo_path()).mix(&Color::WHITE, 0.4)
                } else {
                    Color::WHITE
                };
                set(format!("{}  {rank}/{MAX_RANK}", perk.name()), c);
            }
        }
    }
}
