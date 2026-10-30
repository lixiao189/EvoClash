//! Holds client messages until the next fixed tick reads them.
//!
//! lightyear clears every unread `MessageReceiver` in `Last`, once per frame. The server loop
//! wakes every millisecond but `FixedUpdate` runs at 64 Hz, so most frames have no fixed tick:
//! a command read only from `FixedUpdate` is usually cleared before any tick sees it. Each frame,
//! right after lightyear receives, the messages move into an `Inbox` on the link, and the
//! `SimSet::Input` readers drain that instead.

use bevy::prelude::*;
use evoclash_game::items::InventoryMove;
use evoclash_game::protocol::RoomCommand;
use evoclash_game::shop::PlayerCommand;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::*;

pub struct InboxPlugin;

impl Plugin for InboxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                buffer::<PlayerCommand>,
                buffer::<RoomCommand>,
                buffer::<InventoryMove>,
            )
                .after(MessageSystems::Receive),
        );
    }
}

/// Messages of one type received on a link and not yet applied by a fixed tick.
#[derive(Component)]
pub struct Inbox<M>(pub Vec<M>);

impl<M> Default for Inbox<M> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<M> Inbox<M> {
    pub fn take(&mut self) -> Vec<M> {
        std::mem::take(&mut self.0)
    }
}

pub(crate) fn buffer<M: lightyear::prelude::Message>(
    mut commands: Commands,
    mut links: Query<(Entity, &mut MessageReceiver<M>, Option<&mut Inbox<M>>), With<ClientOf>>,
) {
    for (link, mut receiver, inbox) in &mut links {
        if !receiver.has_messages() {
            continue;
        }
        let received: Vec<M> = receiver.receive().collect();
        match inbox {
            Some(mut inbox) => inbox.0.extend(received),
            None => {
                commands.entity(link).insert(Inbox(received));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolution::{LifeLog, read_commands};
    use evoclash_game::evolution::{Evolution, SkillState};
    use evoclash_game::protocol::{MatchState, Phase, Player, Vitals};
    use evoclash_game::shop::{ShopItem, Wallet};

    /// A buy that arrived in a frame with no fixed tick waits in the inbox, and the next fixed
    /// tick applies it to the body that link controls.
    #[test]
    fn buffered_command_reaches_the_next_tick() {
        let mut app = App::new();
        app.world_mut().spawn(MatchState {
            phase: Phase::Playing,
            ..default()
        });
        let link = app
            .world_mut()
            .spawn((ClientOf, Inbox(vec![PlayerCommand::Buy(ShopItem::Bow)])))
            .id();
        let body = app
            .world_mut()
            .spawn((
                Player {
                    client_id: 7,
                    name: "aba".into(),
                    color: [1.0; 3],
                    bot: false,
                },
                ControlledBy {
                    owner: link,
                    lifetime: Lifetime::Persistent,
                },
                Evolution::default(),
                Wallet {
                    scrap: 39,
                    ..default()
                },
                Vitals {
                    alive: false,
                    ..default()
                },
                SkillState::default(),
                LifeLog::default(),
            ))
            .id();
        app.add_systems(Update, read_commands);
        app.update();

        let wallet = app.world().get::<Wallet>(body).unwrap();
        assert_eq!(wallet.cart, vec![ShopItem::Bow]);
        assert_eq!(wallet.scrap, 39 - ShopItem::Bow.price());
        assert!(
            app.world()
                .get::<Inbox<PlayerCommand>>(link)
                .unwrap()
                .0
                .is_empty()
        );
    }
}
