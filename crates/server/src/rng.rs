//! Seeded streams for one room. Lobby rolls use stream 0 only. Playing re-inits streams 1-6
//! from `match_seed`, so practice cannot change the map or the opening loot.

use std::sync::Mutex;

use bevy::prelude::*;
use evoclash_game::rng::{SplitMix64, stream_seed};

pub const STREAM_LOBBY: u64 = 0;
pub const STREAM_MAP: u64 = 1;
pub const STREAM_LOOT: u64 = 2;
pub const STREAM_BOTS: u64 = 3;
pub const STREAM_MONSTERS: u64 = 4;
pub const STREAM_BOSS: u64 = 5;
pub const STREAM_COMBAT: u64 = 6;

#[derive(Clone, Copy)]
pub enum Stream {
    /// Map layout comes from `map_seed` itself; the stream is still re-seeded with the others.
    #[allow(dead_code)]
    Map,
    Loot,
    Bots,
    Monsters,
    Boss,
    Combat,
}

struct Streams {
    lobby: SplitMix64,
    map: SplitMix64,
    loot: SplitMix64,
    bots: SplitMix64,
    monsters: SplitMix64,
    boss: SplitMix64,
    combat: SplitMix64,
}

impl Streams {
    fn from_match_seed(match_seed: u64) -> Self {
        Self {
            lobby: SplitMix64(stream_seed(match_seed, STREAM_LOBBY)),
            map: SplitMix64(stream_seed(match_seed, STREAM_MAP)),
            loot: SplitMix64(stream_seed(match_seed, STREAM_LOOT)),
            bots: SplitMix64(stream_seed(match_seed, STREAM_BOTS)),
            monsters: SplitMix64(stream_seed(match_seed, STREAM_MONSTERS)),
            boss: SplitMix64(stream_seed(match_seed, STREAM_BOSS)),
            combat: SplitMix64(stream_seed(match_seed, STREAM_COMBAT)),
        }
    }

    fn slot(&mut self, stream: Stream) -> &mut SplitMix64 {
        match stream {
            Stream::Map => &mut self.map,
            Stream::Loot => &mut self.loot,
            Stream::Bots => &mut self.bots,
            Stream::Monsters => &mut self.monsters,
            Stream::Boss => &mut self.boss,
            Stream::Combat => &mut self.combat,
        }
    }
}

/// Shared by every system. Each Playing stream is independent, so system order across streams
/// does not change a stream's sequence. Outside Playing every roll uses the lobby stream.
#[derive(Resource)]
pub struct SimRng(Mutex<Streams>);

impl SimRng {
    pub fn from_match_seed(match_seed: u64) -> Self {
        Self(Mutex::new(Streams::from_match_seed(match_seed)))
    }

    /// New lobby: every stream, including lobby, restarts from this round's seed.
    pub fn reset_all(&self, match_seed: u64) {
        *self.0.lock().expect("sim rng") = Streams::from_match_seed(match_seed);
    }

    /// Playing start: streams 1-6 only. Stream 0 keeps whatever the lobby already drew.
    pub fn reinit_match(&self, match_seed: u64) {
        let fresh = Streams::from_match_seed(match_seed);
        let mut streams = self.0.lock().expect("sim rng");
        streams.map = fresh.map;
        streams.loot = fresh.loot;
        streams.bots = fresh.bots;
        streams.monsters = fresh.monsters;
        streams.boss = fresh.boss;
        streams.combat = fresh.combat;
    }

    pub fn with<R>(
        &self,
        playing: bool,
        stream: Stream,
        f: impl FnOnce(&mut SplitMix64) -> R,
    ) -> R {
        let mut streams = self.0.lock().expect("sim rng");
        let rng = if playing {
            streams.slot(stream)
        } else {
            &mut streams.lobby
        };
        f(rng)
    }
}

#[cfg(test)]
mod tests {
    use evoclash_game::items::{Item, roll_loot};
    use evoclash_game::rng::mix;
    use rand::Rng;

    use super::*;

    #[test]
    fn lobby_rolls_do_not_change_the_playing_map_or_first_loot() {
        let room_seed = 0xC0FFEE_u64;
        let match_index = 2u32;
        let match_seed = mix(room_seed, match_index);
        let map_seed = match_seed;
        let rng = SimRng::from_match_seed(match_seed);
        rng.with(false, Stream::Loot, |lobby| {
            for tier in 0..5 {
                let _ = roll_loot(tier, lobby);
            }
            let _: u64 = lobby.random();
        });
        assert_eq!(map_seed, mix(room_seed, match_index), "map seed is pure");
        rng.reinit_match(match_seed);
        let first = rng.with(true, Stream::Loot, |loot| roll_loot(1, loot));
        let fresh = SimRng::from_match_seed(match_seed);
        fresh.reinit_match(match_seed);
        let expected = fresh.with(true, Stream::Loot, |loot| roll_loot(1, loot));
        assert_eq!(first, expected);
        assert!(matches!(
            first,
            Item::Weapon(_) | Item::Armor { .. } | Item::Ammo { .. } | Item::Medkit | Item::Emp
        ));
    }
}
