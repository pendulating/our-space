//! A minimal RNG abstraction so the render-agnostic core can drive random walks
//! and agent variety without pulling in a heavy `rand` dependency (which bloats
//! the WASM build). The app supplies a source; `WyRand` is a cheap default.

/// Anything that can produce `u32`s. Default helpers cover the common draws.
pub trait RngLike {
    fn next_u32(&mut self) -> u32;

    /// Uniform in `[0, 1)`.
    fn next_f64(&mut self) -> f64 {
        (self.next_u32() as f64) / (u32::MAX as f64 + 1.0)
    }

    /// Uniform integer in `[0, n)` (returns 0 when `n == 0`).
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u32() as usize) % n
        }
    }
}

/// Small, fast, deterministic PRNG (wyrand). Not cryptographic — just cheap and
/// WASM-friendly for picking edges, routes, and Bernoulli rolls.
#[derive(Debug, Clone)]
pub struct WyRand(u64);

impl WyRand {
    pub fn new(seed: u64) -> Self {
        WyRand(seed ^ 0x2545_f491_4f6c_dd1d)
    }
}

impl RngLike for WyRand {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0xa076_1d64_78bd_642f);
        let s = self.0;
        let t = (s as u128).wrapping_mul((s ^ 0xe703_7ed1_a0b4_28db) as u128);
        ((t >> 64) ^ t) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_is_in_range_and_varies() {
        let mut r = WyRand::new(42);
        let mut seen = [false; 5];
        for _ in 0..500 {
            let v = r.below(5);
            assert!(v < 5);
            seen[v] = true;
        }
        assert!(seen.iter().all(|&s| s), "all buckets should be hit");
    }

    #[test]
    fn f64_in_unit_interval() {
        let mut r = WyRand::new(7);
        for _ in 0..1000 {
            let x = r.next_f64();
            assert!((0.0..1.0).contains(&x));
        }
    }
}
