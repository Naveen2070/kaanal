//! The per-hop random generator (R1.34, D8, re-pinned 2026-10-25).

use std::convert::Infallible;

use rand::SeedableRng;
use rand::rand_core::TryRng;

/// `SplitMix64`: a 64-bit state, one add and a three-step mix per draw.
///
/// Every hop of every request seeds its own generator from `stable_hash(path, node, tag)`
/// and draws one to three values, so the cost that matters is seeding. `ChaCha8Rng` spent
/// most of a request there (key expansion plus a 256-byte block per hop); this seeds with
/// one store. The state is the seed itself, so two hops overlap only if their 64-bit
/// hashes differ by a few draws, which FNV-1a over distinct (path, node, tag) does not do
/// in practice. Not for cryptography; it only drives simulated latencies and chaos rolls.
#[derive(Clone, Debug)]
pub struct HopRng(u64);

// rand_core 0.10: a generator that cannot fail implements `TryRng` with `Infallible`, and the
// blanket impl makes it a `Rng`. The stream is the same SplitMix64 as before.
impl TryRng for HopRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        // the high half is the better-mixed one
        Ok((self.step() >> 32) as u32)
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        Ok(self.step())
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Infallible> {
        for chunk in dest.chunks_mut(8) {
            let b = self.step().to_le_bytes();
            chunk.copy_from_slice(&b[..chunk.len()]);
        }
        Ok(())
    }
}

impl HopRng {
    fn step(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

impl SeedableRng for HopRng {
    type Seed = [u8; 8];

    fn from_seed(seed: Self::Seed) -> Self {
        Self(u64::from_le_bytes(seed))
    }

    fn seed_from_u64(state: u64) -> Self {
        Self(state)
    }
}
