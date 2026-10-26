//! The backpack panel (B): hotbar, bag grid and ammo pouch. Click a weapon to lift it, click
//! another slot to swap them (`InventoryMove`); right-click or Esc puts it back.

use bevy::prelude::*;
use bevy::ui::UiGlobalTransform;
use bevy::window::PrimaryWindow;
use evoclash_game::evolution::Evolution;
use evoclash_game::items::{
    AmmoKind, BAG_SLOTS, HOTBAR_SLOTS, Item, MELEE_SLOT, SLOT_COUNT, THROW_SLOT, WeaponKind,
    tier_color,
};
use evoclash_game::protocol::*;
use lightyear::prelude::*;

use crate::input::{BackpackOpen, LocalPlayer, Selection};
use crate::render::UiFont;
use crate::textures::Textures;

const PANEL_BG: Color = Color::srgba(0.05, 0.05, 0.06, 0.92);
const SLOT_BG: Color = Color::srgba(0.16, 0.16, 0.18, 0.95);
const EDGE: Color = Color::srgba(0.45, 0.45, 0.5, 0.9);
const LIFT: Color = Color::srgb(1.0, 0.85, 0.35);
const HOVER: Color = Color::srgb(0.8, 0.9, 1.0);
const SLOT_PX: f32 = 56.0;

pub struct BackpackPlugin;

impl Plugin for BackpackPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Lifted>().init_resource::<Hover>();
        app.add_systems(Startup, spawn_panel);
        app.add_systems(
            Update,
            (toggle, hover_and_click, paint_slots, paint_pouch).chain(),
        );
    }
}

/// The slot picked up and waiting for a destination.
#[derive(Resource, Default)]
struct Lifted(Option<u8>);

/// The slot under the cursor. UI picking isn't compiled in, so slots are hit-tested by hand.
#[derive(Resource, Default)]
struct Hover(Option<u8>);

#[derive(Component)]
struct PanelRoot;

/// A clickable slot, by slot id (`Loadout::slot`).
#[derive(Component)]
struct SlotUi(u8);

#[derive(Component)]
struct SlotIcon(u8);

#[derive(Component)]
struct SlotCount(u8);

#[derive(Component)]
struct Tip;

#[derive(Component)]
struct PouchIcon(AmmoKind);

#[derive(Component)]
struct PouchText(AmmoKind);

/// Slots the player can lift: gun slots and the bag, never melee or throwables.
fn movable(id: u8) -> bool {
    id < SLOT_COUNT && id != MELEE_SLOT && id != THROW_SLOT
}

/// Shots a gun has left in the pouch.
pub fn shots_left(loadout: &Loadout, weapon: WeaponKind) -> u16 {
    match (weapon.ammo(), weapon.ammo_cost()) {
        (Some(kind), cost) if cost > 0 => loadout.rounds(kind) / cost,
        _ => 0,
    }
}

/// The picture for a hotbar or bag slot, if it holds anything.
pub fn slot_image(textures: &Textures, loadout: &Loadout, id: u8) -> Option<Handle<Image>> {
    if id == THROW_SLOT {
        return Some(textures.item(Item::Emp));
    }
    loadout.slot(id).map(|w| textures.item(Item::Weapon(w)))
}

fn spawn_panel(mut commands: Commands, font: Res<UiFont>) {
    commands
        .spawn((
            PanelRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: Display::None,
                ..default()
            },
            GlobalZIndex(20),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(14.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
                BorderColor::all(EDGE),
            ))
            .with_children(|panel| {
                label(panel, &font, 18.0, "背包");
                label(panel, &font, 13.0, "快捷栏");
                slot_row(panel, &font, 0..HOTBAR_SLOTS);
                label(panel, &font, 13.0, "背包格");
                for row in 0..2u8 {
                    let from = HOTBAR_SLOTS + row * 3;
                    slot_row(panel, &font, from..from + (BAG_SLOTS / 2) as u8);
                }
                label(panel, &font, 13.0, "弹药袋");
                for kind in AmmoKind::ALL {
                    panel
                        .spawn(Node {
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn((
                                PouchIcon(kind),
                                ImageNode::default(),
                                Node {
                                    width: Val::Px(28.0),
                                    ..default()
                                },
                            ));
                            row.spawn((
                                PouchText(kind),
                                Text::new(""),
                                font.sized(14.0),
                                TextColor(Color::WHITE),
                            ));
                        });
                }
                panel.spawn((
                    Tip,
                    Text::new(""),
                    font.sized(14.0),
                    TextColor(LIFT),
                    Node {
                        min_height: Val::Px(18.0),
                        ..default()
                    },
                ));
            });
        });
}

fn label(parent: &mut ChildSpawnerCommands, font: &UiFont, size: f32, text: &str) {
    parent.spawn((
        Text::new(text),
        font.sized(size),
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
    ));
}

fn slot_row(parent: &mut ChildSpawnerCommands, font: &UiFont, ids: std::ops::Range<u8>) {
    parent
        .spawn(Node {
            column_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|row| {
            for id in ids {
                row.spawn((
                    SlotUi(id),
                    Node {
                        width: Val::Px(SLOT_PX),
                        height: Val::Px(SLOT_PX),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(SLOT_BG),
                    BorderColor::all(EDGE),
                ))
                .with_children(|slot| {
                    slot.spawn((
                        SlotIcon(id),
                        ImageNode::default(),
                        Node {
                            width: Val::Px(44.0),
                            ..default()
                        },
                    ));
                    slot.spawn((
                        Text::new(if id < HOTBAR_SLOTS {
                            (id + 1).to_string()
                        } else {
                            String::new()
                        }),
                        font.sized(12.0),
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.7)),
                        Node {
                            position_type: PositionType::Absolute,
                            top: Val::Px(1.0),
                            left: Val::Px(3.0),
                            ..default()
                        },
                    ));
                    slot.spawn((
                        SlotCount(id),
                        Text::new(""),
                        font.sized(12.0),
                        TextColor(Color::WHITE),
                        Node {
                            position_type: PositionType::Absolute,
                            bottom: Val::Px(1.0),
                            right: Val::Px(3.0),
                            ..default()
                        },
                    ));
                });
            }
        });
}

fn toggle(
    open: Res<BackpackOpen>,
    mut lifted: ResMut<Lifted>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut root: Query<&mut Node, With<PanelRoot>>,
) {
    let display = if open.0 { Display::Flex } else { Display::None };
    for mut node in &mut root {
        if node.display != display {
            node.display = display;
        }
    }
    if (!open.0 || mouse.just_pressed(MouseButton::Right)) && lifted.0.is_some() {
        lifted.0 = None;
    }
}

fn hover_and_click(
    open: Res<BackpackOpen>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Query<&Window, With<PrimaryWindow>>,
    slots: Query<(&SlotUi, &ComputedNode, &UiGlobalTransform)>,
    me: Query<&Loadout, With<LocalPlayer>>,
    mut hover: ResMut<Hover>,
    mut lifted: ResMut<Lifted>,
    mut senders: Query<&mut MessageSender<InventoryMove>, With<Client>>,
) {
    let cursor = window
        .single()
        .ok()
        .and_then(|w| w.physical_cursor_position())
        .filter(|_| open.0);
    let over = cursor.and_then(|point| {
        slots
            .iter()
            .find(|(_, node, transform)| node.contains_point(**transform, point))
            .map(|(SlotUi(id), _, _)| *id)
    });
    if hover.0 != over {
        hover.0 = over;
    }
    let (Some(id), true, Ok(loadout)) = (over, mouse.just_pressed(MouseButton::Left), me.single())
    else {
        return;
    };
    match lifted.0 {
        None => {
            if movable(id) && loadout.slot(id).is_some() {
                lifted.0 = Some(id);
            }
        }
        Some(from) if movable(id) => {
            lifted.0 = None;
            if from != id {
                for mut sender in &mut senders {
                    sender.send::<ReliableChannel>(InventoryMove { from, to: id });
                }
            }
        }
        Some(_) => {}
    }
}

fn paint_slots(
    open: Res<BackpackOpen>,
    textures: Option<Res<Textures>>,
    selection: Res<Selection>,
    lifted: Res<Lifted>,
    me: Query<&Loadout, With<LocalPlayer>>,
    hover: Res<Hover>,
    mut slots: Query<(&SlotUi, &mut BorderColor, &mut BackgroundColor)>,
    mut icons: Query<(&SlotIcon, &mut ImageNode, &mut Visibility)>,
    mut counts: Query<(&SlotCount, &mut Text), Without<Tip>>,
    mut tip: Query<&mut Text, With<Tip>>,
) {
    let (Some(textures), Ok(loadout), true) = (textures, me.single(), open.0) else {
        return;
    };
    let shown = selection.shown(loadout);
    let mut hint = String::new();
    for (SlotUi(id), mut border, mut bg) in &mut slots {
        let hovered = hover.0 == Some(*id);
        let color = if lifted.0 == Some(*id) {
            LIFT
        } else if hovered {
            HOVER
        } else if *id == shown {
            tier_color(1).mix(&Color::WHITE, 0.5)
        } else {
            EDGE
        };
        border.set_if_neq(BorderColor::all(color));
        let fill = match (lifted.0, hovered) {
            (Some(_), true) if !movable(*id) => Color::srgba(0.3, 0.1, 0.1, 0.95),
            _ => SLOT_BG,
        };
        bg.set_if_neq(BackgroundColor(fill));
        if hovered {
            hint = match (*id, loadout.slot(*id)) {
                (THROW_SLOT, _) => "电磁脉冲雷".to_string(),
                (_, Some(w)) => {
                    let spec = w.spec();
                    format!("{}  T{}", spec.name(), spec.tier())
                }
                _ => String::new(),
            };
        }
    }
    for (SlotIcon(id), mut image, mut visibility) in &mut icons {
        match slot_image(&textures, loadout, *id) {
            Some(handle) => {
                if image.image != handle {
                    image.image = handle;
                }
                visibility.set_if_neq(Visibility::Inherited);
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }
    for (SlotCount(id), mut text) in &mut counts {
        let value = match (*id, loadout.slot(*id)) {
            (THROW_SLOT, _) => format!("×{}", loadout.emps),
            (_, Some(w)) if w.ranged().is_some() => format!("{}", shots_left(loadout, w)),
            _ => String::new(),
        };
        if text.0 != value {
            text.0 = value;
        }
    }
    for mut text in &mut tip {
        if text.0 != hint {
            text.0 = hint.clone();
        }
    }
}

fn paint_pouch(
    open: Res<BackpackOpen>,
    textures: Option<Res<Textures>>,
    me: Query<(&Loadout, Option<&Evolution>), With<LocalPlayer>>,
    mut icons: Query<(&PouchIcon, &mut ImageNode)>,
    mut texts: Query<(&PouchText, &mut Text)>,
) {
    let (Some(textures), Ok((loadout, evo)), true) = (textures, me.single(), open.0) else {
        return;
    };
    let default_evo = Evolution::default();
    let evo = evo.unwrap_or(&default_evo);
    for (PouchIcon(kind), mut image) in &mut icons {
        let handle = textures.item(Item::Ammo {
            kind: *kind,
            rounds: 0,
        });
        if image.image != handle {
            image.image = handle;
        }
    }
    for (PouchText(kind), mut text) in &mut texts {
        let value = format!(
            "{}/{}  {}",
            loadout.rounds(*kind),
            evo.ammo_cap(*kind),
            kind.name()
        );
        if text.0 != value {
            text.0 = value;
        }
    }
}
