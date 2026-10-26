//! SplitMix64, the same step the arena generator uses. Bit-identical on wasm and native:
//! no `rand`, no platform floats.

use rand::RngCore;

/// SplitMix64. `0` is the state, not the last output.
#[derive(Clone, Debug)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    /// One SplitMix64 step. Same constants as the map generator.
    pub fn step(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn next_u64(&mut self) -> u64 {
        Self::step(&mut self.0)
    }
}

impl RngCore for SplitMix64 {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn next_u64(&mut self) -> u64 {
        SplitMix64::step(&mut self.0)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut offset = 0;
        while offset < dest.len() {
            let bytes = self.next_u64().to_le_bytes();
            let n = (dest.len() - offset).min(8);
            dest[offset..offset + n].copy_from_slice(&bytes[..n]);
            offset += n;
        }
    }
}

/// `match_seed` / `map_seed` for one round of a room.
pub fn mix(room_seed: u64, match_index: u32) -> u64 {
    let mut state = room_seed ^ (match_index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    SplitMix64::step(&mut state)
}

/// Seed for a numbered stream of `match_seed` (0 lobby … 6 combat).
pub fn stream_seed(match_seed: u64, stream: u64) -> u64 {
    let mut state = match_seed ^ stream.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    SplitMix64::step(&mut state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::ArenaMap;

    #[test]
    fn mix_and_map_seed_are_stable() {
        let room = 0x4556_4f43_u64;
        let seed = mix(room, 4);
        assert_eq!(seed, mix(room, 4));
        assert_ne!(seed, mix(room, 5));
        assert_ne!(seed, mix(room ^ 1, 4));
        let a = ArenaMap::generate(seed);
        let b = ArenaMap::generate(seed);
        assert_eq!(a.seed, seed);
        assert_eq!(a.walls, b.walls);
        assert_eq!(a.loot_spots, b.loot_spots);
        assert_eq!(a.props, b.props);
        assert_eq!(a.spawn_points, b.spawn_points);
    }
}
