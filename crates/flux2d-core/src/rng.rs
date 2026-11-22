use glam::Vec2;

/// A small, fast random number generator (xorshift64*). The same seed gives the same numbers
/// on every machine, which keeps benchmarks and replays repeatable. Not for cryptography.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    /// A generator from `seed` (any value; 0 is replaced by a fixed constant).
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..1`.
    #[inline]
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `lo..hi`.
    #[inline]
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }

    /// Uniform integer in `0..n` (`n` > 0).
    #[inline]
    pub fn below(&mut self, n: u32) -> u32 {
        (((self.next_u64() >> 32) * u64::from(n)) >> 32) as u32
    }

    /// True with probability `p`.
    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }

    /// A unit vector in a random direction.
    #[inline]
    pub fn direction(&mut self) -> Vec2 {
        Vec2::from_angle(self.range(0.0, std::f32::consts::TAU))
    }

    /// A new generator seeded from this one, for handing to another thread or system.
    pub fn fork(&mut self) -> Rng {
        Rng::new(self.next_u64() | 1)
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_and_repeatability() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            let x = a.range(-3.0, 5.0);
            assert_eq!(x, b.range(-3.0, 5.0));
            assert!((-3.0..5.0).contains(&x));
            assert!(a.below(7) < 7);
            b.below(7);
            assert!((a.direction().length() - 1.0).abs() < 1e-4);
            b.direction();
        }
    }
}
