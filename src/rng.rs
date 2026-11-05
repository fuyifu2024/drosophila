//! xoshiro128++, seeded through splitmix64.
//!
//! The simulation is stochastic — every neuron gets a chance of a random kick
//! each millisecond — so the generator sits in the same inner loop as the
//! membrane update and has to be cheap. It also has to be reproducible: a
//! 139,000-neuron circuit is far too big to compare spike-for-spike against a
//! reference, so the only way to test it is to run the same seeds twice and
//! assert statistics, which means the seeds must actually be deterministic.

#[derive(Clone)]
pub struct Rng {
    s: [u32; 4],
}

impl Rng {
    pub fn from_seed(seed: u64) -> Self {
        // splitmix64 to stir the seed; seeding xoshiro from a bare counter
        // leaves the first few outputs correlated across nearby seeds.
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        let mut s = [0u32; 4];
        for slot in &mut s {
            *slot = next() as u32;
        }
        // xoshiro is undefined for an all-zero state.
        if s == [0, 0, 0, 0] {
            s[0] = 1;
        }
        Rng { s }
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let result = self.s[0].wrapping_add(self.s[3]).rotate_left(7).wrapping_add(self.s[0]);
        let t = self.s[1] << 9;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(11);
        result
    }

    /// Uniform in [0, 1). 24 bits, which is all an f32 can hold anyway.
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// Uniform in [lo, hi).
    #[inline]
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_stream() {
        let mut a = Rng::from_seed(0x_F1_1E);
        let mut b = Rng::from_seed(0x_F1_1E);
        for _ in 0..1000 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn different_seeds_diverge_immediately() {
        let mut a = Rng::from_seed(1);
        let mut b = Rng::from_seed(2);
        let differ = (0..64).filter(|_| a.next_u32() != b.next_u32()).count();
        assert!(differ > 60, "only {differ}/64 differed");
    }

    #[test]
    fn floats_stay_in_range_and_are_roughly_uniform() {
        let mut r = Rng::from_seed(7);
        let mut buckets = [0usize; 10];
        for _ in 0..100_000 {
            let x = r.next_f32();
            assert!((0.0..1.0).contains(&x), "{x} out of range");
            buckets[(x * 10.0) as usize] += 1;
        }
        // Each bucket should get ~10% of 100k; allow generous slack.
        for (i, &n) in buckets.iter().enumerate() {
            assert!((8000..12_000).contains(&n), "bucket {i} got {n}");
        }
    }
}
