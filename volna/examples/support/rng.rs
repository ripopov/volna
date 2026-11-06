//! Deterministic xorshift values for simulated recordings.

pub(super) struct Rng(pub(super) u64);
impl Rng {
    pub(super) fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub(super) fn chance(&mut self, one_in: u64) -> bool {
        self.next().is_multiple_of(one_in)
    }
}
