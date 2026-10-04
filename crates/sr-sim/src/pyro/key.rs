//! Content hash of an exported volume, for caches that must notice any change.
//!
//! Deterministic and independent of the thread count: every brick is hashed
//! on its own, serially in sample order, and the brick hashes are combined
//! serially in brick-key order. All mixing uses the SplitMix64 finaliser, a
//! bijective 64-bit mixer with full avalanche, so the result depends on every
//! sample bit and on where each sample sits.

use super::*;

const SEED: u64 = 0x243f_6a88_85a3_08d3;
const GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;

fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Order-sensitive absorption of one word.
fn absorb(h: u64, word: u64) -> u64 {
    mix(h ^ word).wrapping_add(GOLDEN)
}

fn brick_hash(key: [i32; 3], values: &[f32; 512]) -> u64 {
    let mut h = SEED;
    for k in key {
        h = absorb(h, u64::from(k as u32));
    }
    for pair in values.as_chunks::<2>().0 {
        h = absorb(h, u64::from(pair[0].to_bits()) | (u64::from(pair[1].to_bits()) << 32));
    }
    h
}

/// A 64-bit hash of a volume's grid names, backgrounds, transforms, brick
/// coordinates and every sample bit. Equal volumes hash equal; the value is a
/// cache token for this process and is not a stable file format.
pub fn volume_key(volume: &Volume) -> u64 {
    let mut h = SEED;
    for (name, grid) in volume.grids() {
        h = absorb(h, name.len() as u64);
        for chunk in name.as_bytes().chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            h = absorb(h, u64::from_le_bytes(word));
        }
        h = absorb(h, u64::from(grid.background().to_bits()));
        for column in grid.transform().columns() {
            h = absorb(h, column.to_bits());
        }
        h = absorb(h, grid.brick_count() as u64);
        let bricks: Vec<_> = grid.bricks().collect();
        let hashes: Vec<u64> =
            bricks.par_iter().with_min_len(16).map(|(key, values)| brick_hash(*key, values)).collect();
        for brick in hashes {
            h = absorb(h, brick);
        }
    }
    h
}
