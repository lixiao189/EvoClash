//! The death screen, shown while the local player is dead: first the death card (who killed
//! you, how, and how the life went), then a free respec of the evolution tree and the scrap
//! shop, with the deploy buttons along the bottom the whole time.

use bevy::prelude::*;
use evoclash_game::evolution::Evolution;
use evoclash_game::items::tier_color;
use evoclash_game::protocol::Vitals;
use evoclash_game::shop::{
    DEATH_CARD_TICKS, DeathRecap, PlayerCommand, RESPAWN_MAX_TICKS, RESPAWN_MIN_TICKS, ShopItem,
    Wallet, can_deploy, dead_ticks,
};
use lightyear::prelude::{Client, MessageSender};

use crate::buttons::{Act, BlocksPointer, button, send};
use crate::evo_panel::spawn_tree;
use crate::hud::{GOLD, PANEL_BG, path_color};
use crate::input::LocalPlayer;
use crate::render::UiFont;

pub struct DeathScreenPlugin;

impl Plugin for DeathScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_death_screen);
        app.add_systems(Update, (update_death_screen, death_keys));
    }
}

/// Cart rows on screen; more than this can't be bought (one gun, one armor, a few EMPs).
const CART_SLOTS: usize = 8;
const RED: Color = Color::srgb(1.0, 0.4, 0.35);
const DIM: Color = Color::srgba(0.85, 0.85, 0.85, 0.85);

#[derive(Component, Clone, Copy, PartialEq)]
enum DeathUi {
    Root,
    /// The first `DEATH_CARD_TICKS`.
    Card,
    Title,
    KillerPath,
    Summary,
    /// The respec and shop, after the card.
    Prep,
    SmallSummary,
    Scrap,
    ShopName(usize),
    ShopInfo(usize),
    CartSlot(usize),
    CartText(usize),
    Countdown,
    DeployText,
    LastText,
}

fn text(font: &UiFont, size: f32, kind: DeathUi, color: Color) -> impl Bundle {
    (
        kind,
        Text::new(""),
        font.sized(size),
        TextColor(color),
        TextShadow::default(),
    )
}

fn label(font: &UiFont, size: f32, value: &str, color: Color) -> impl Bundle {
    (
        Text::new(value),
        font.sized(size),
        TextColor(color),
        TextShadow::default(),
    )
}

fn panel() -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            padding: UiRect::all(Val::Px(10.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
    )
}

fn spawn_death_screen(mut commands: Commands, font: Res<UiFont>) {
    let font = &*font;
    commands
        .spawn((
            DeathUi::Root,
            BlocksPointer,
            Visibility::Hidden,
            GlobalZIndex(10),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(12.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.02, 0.02, 0.72)),
        ))
        .with_children(|root| {
            root.spawn((
                DeathUi::Card,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Percent(32.0),
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(10.0),
                    ..default()
                },
            ))
            .with_children(|card| {
                card.spawn(text(font, 30.0, DeathUi::Title, RED));
                card.spawn(text(font, 20.0, DeathUi::KillerPath, Color::WHITE));
                card.spawn(text(font, 17.0, DeathUi::Summary, DIM));
            });
            root.spawn((
                DeathUi::Prep,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(8.0),
                    ..default()
                },
            ))
            .with_children(|prep| {
                prep.spawn(text(font, 14.0, DeathUi::SmallSummary, DIM));
                prep.spawn(text(font, 22.0, DeathUi::Scrap, GOLD));
                prep.spawn(Node {
                    column_gap: Val::Px(16.0),
                    align_items: AlignItems::FlexStart,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn(panel()).with_children(|tree| {
                        spawn_tree(tree, font, true);
                        tree.spawn(button(
                            Act::Send(PlayerCommand::ResetPerks),
                            Node {
                                align_self: AlignSelf::FlexStart,
                                ..default()
                            },
                        ))
                        .with_child(label(
                            font,
                            14.0,
                            "重置",
                            Color::WHITE,
                        ));
                    });
                    row.spawn(panel())
                        .with_children(|shop| spawn_shop(shop, font));
                });
            });
            root.spawn(text(font, 18.0, DeathUi::Countdown, Color::WHITE));
            root.spawn(Node {
                column_gap: Val::Px(12.0),
                ..default()
            })
            .with_children(|row| {
                let big = Node {
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                    ..default()
                };
                row.spawn(button(Act::Send(PlayerCommand::Deploy), big.clone()))
                    .with_child(text(font, 18.0, DeathUi::DeployText, Color::WHITE));
                row.spawn(button(Act::Send(PlayerCommand::DeployLast), big))
                    .with_child(text(font, 18.0, DeathUi::LastText, Color::WHITE));
            });
        });
}

fn spawn_shop(shop: &mut ChildSpawnerCommands, font: &UiFont) {
    shop.spawn(label(font, 16.0, "商店", GOLD));
    for (i, item) in ShopItem::STOCK.into_iter().enumerate() {
        shop.spawn(button(
            Act::Send(PlayerCommand::Buy(item)),
            Node {
                width: Val::Px(240.0),
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            },
        ))
        .with_children(|b| {
            b.spawn(text(font, 15.0, DeathUi::ShopName(i), Color::WHITE));
            b.spawn(text(font, 14.0, DeathUi::ShopInfo(i), GOLD));
        });
    }
    shop.spawn((
        label(font, 14.0, "已购（点击退回）", DIM),
        Node {
            margin: UiRect::top(Val::Px(6.0)),
            ..default()
        },
    ));
    for i in 0..CART_SLOTS {
        shop.spawn(button(
            Act::Unbuy(i),
            Node {
                width: Val::Px(240.0),
                ..default()
            },
        ))
        .insert(DeathUi::CartSlot(i))
        .with_child(text(font, 14.0, DeathUi::CartText(i), Color::WHITE));
    }
}

fn names(items: &[ShopItem]) -> String {
    items
        .iter()
        .map(|i| i.name())
        .collect::<Vec<_>>()
        .join(" + ")
}

fn title(recap: Option<&DeathRecap>) -> String {
    recap.map_or("你倒下了".to_string(), |r| {
        format!("被 {} 用 {} 击杀", r.killer, r.how)
    })
}

fn summary(recap: Option<&DeathRecap>) -> String {
    recap.map_or(String::new(), |r| {
        format!(
            "存活 {}:{:02} · 废料 +{} · 击杀 {}",
            r.alive_secs / 60,
            r.alive_secs % 60,
            r.scrap,
            r.kills
        )
    })
}

fn shown(on: bool) -> Visibility {
    if on {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

fn update_death_screen(
    me: Query<(&Vitals, &Wallet, &Evolution), With<LocalPlayer>>,
    mut ui: Query<(
        &DeathUi,
        Option<&mut Text>,
        Option<&mut TextColor>,
        &mut Visibility,
    )>,
) {
    let me = me.single().ok().filter(|(v, ..)| !v.alive);
    for (kind, text, color, mut vis) in &mut ui {
        if *kind == DeathUi::Root {
            vis.set_if_neq(shown(me.is_some()));
            continue;
        }
        let Some((vitals, wallet, evo)) = me else {
            continue;
        };
        let recap = wallet.recap.as_ref();
        let on_card = dead_ticks(vitals.respawn_in) < DEATH_CARD_TICKS;
        let set = |value: String, c: Option<Color>| {
            if let Some(mut text) = text
                && text.0 != value
            {
                text.0 = value;
            }
            if let (Some(mut color), Some(c)) = (color, c)
                && color.0 != c
            {
                color.0 = c;
            }
        };
        match *kind {
            DeathUi::Root => {}
            DeathUi::Card => {
                vis.set_if_neq(shown(on_card));
            }
            DeathUi::Prep => {
                vis.set_if_neq(shown(!on_card));
            }
            DeathUi::Title => set(title(recap), None),
            DeathUi::KillerPath => {
                match recap.and_then(|r| r.killer_path.zip(Some(r.killer_level))) {
                    Some((path, level)) if level > 0 => {
                        set(format!("{} Lv{level}", path.name()), Some(path_color(path)));
                    }
                    _ => set(String::new(), None),
                }
            }
            DeathUi::Summary => set(summary(recap), None),
            DeathUi::SmallSummary => {
                let line = summary(recap);
                let value = if line.is_empty() {
                    title(recap)
                } else {
                    format!("{} · {line}", title(recap))
                };
                set(value, None);
            }
            DeathUi::Scrap => set(format!("废料 {}", wallet.scrap), None),
            DeathUi::ShopName(i) => {
                let item = ShopItem::STOCK[i];
                let c = tier_color(item.tier()).mix(&Color::WHITE, 0.3);
                let c = if wallet.can_buy(item, evo) {
                    c
                } else {
                    c.with_alpha(0.4)
                };
                set(format!("{}  T{}", item.name(), item.tier()), Some(c));
            }
            DeathUi::ShopInfo(i) => {
                let item = ShopItem::STOCK[i];
                let held = wallet.cart.iter().filter(|c| **c == item).count();
                let net = wallet.net_price(item);
                let mut value = if net != item.price() as i64 {
                    format!("{} 废料（补差 {net}）", item.price())
                } else {
                    format!("{} 废料", item.price())
                };
                if held > 0 {
                    value.push_str(&format!("  ×{held}"));
                }
                let c = if wallet.can_buy(item, evo) {
                    GOLD
                } else {
                    GOLD.with_alpha(0.4)
                };
                set(value, Some(c));
            }
            DeathUi::CartSlot(i) => {
                vis.set_if_neq(shown(i < wallet.cart.len()));
            }
            DeathUi::CartText(i) => {
                if let Some(item) = wallet.cart.get(i) {
                    set(
                        format!("{}  ×", item.name()),
                        Some(tier_color(item.tier()).mix(&Color::WHITE, 0.3)),
                    );
                }
            }
            DeathUi::Countdown => {
                let secs = vitals.respawn_in.div_ceil(64);
                let early = vitals
                    .respawn_in
                    .saturating_sub(RESPAWN_MAX_TICKS - RESPAWN_MIN_TICKS)
                    .div_ceil(64);
                let value = if early > 0 {
                    format!("{secs} 秒后自动出发 · {early} 秒后可提前出发")
                } else {
                    format!("{secs} 秒后自动出发")
                };
                set(value, None);
            }
            DeathUi::DeployText => {
                let c = if can_deploy(vitals.respawn_in) {
                    Color::WHITE
                } else {
                    Color::srgba(1.0, 1.0, 1.0, 0.4)
                };
                set("出发 [空格]".to_string(), Some(c));
            }
            DeathUi::LastText => {
                let value = if wallet.last_cart.is_empty() {
                    "沿用上次配置 [R]".to_string()
                } else {
                    format!("沿用上次配置：{} [R]", names(&wallet.last_cart))
                };
                set(value, None);
            }
        }
    }
}

fn death_keys(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    me: Query<&Vitals, With<LocalPlayer>>,
    mut senders: Query<&mut MessageSender<PlayerCommand>, With<Client>>,
) {
    let (Some(keys), Ok(vitals)) = (keys, me.single()) else {
        return;
    };
    if vitals.alive {
        return;
    }
    if keys.just_pressed(KeyCode::Space) && can_deploy(vitals.respawn_in) {
        send(&mut senders, PlayerCommand::Deploy);
    }
    if keys.just_pressed(KeyCode::KeyR) {
        send(&mut senders, PlayerCommand::DeployLast);
    }
}
