//! a small deterministic random number generator (SplitMix64), written here so no dependency can change the algorithm under the seeds.

pub struct Rng(u64);

impl Rng {
    /// seed from text like "7-12-formula-hard", FNV-1a over its bytes.
    pub fn from_text(seed: &str) -> Self {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in seed.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        let mut rng = Rng(h);
        rng.next_u64(); // spread a low entropy seed.
        rng
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// a number in 0.0..1.0.
    pub fn float(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// a whole number from lo to hi, both included.
    pub fn int(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % (hi - lo + 1) as u64) as i64
    }

    /// true with probability p.
    pub fn chance(&mut self, p: f64) -> bool {
        self.float() < p
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next_u64() as usize % items.len()]
    }

    /// an item chosen with the given weights.
    pub fn weighted<'a, T>(&mut self, items: &'a [(T, u32)]) -> &'a T {
        let total: u64 = items.iter().map(|(_, w)| u64::from(*w)).sum();
        let mut left = self.next_u64() % total;
        for (item, w) in items {
            if left < u64::from(*w) {
                return item;
            }
            left -= u64::from(*w);
        }
        &items[items.len() - 1].0
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.next_u64() as usize % (i + 1);
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_always_gives_the_same_numbers() {
        let (mut a, mut b) = (Rng::from_text("7-0"), Rng::from_text("7-0"));
        assert_eq!(
            (0..5).map(|_| a.next_u64()).collect::<Vec<_>>(),
            (0..5).map(|_| b.next_u64()).collect::<Vec<_>>()
        );
        // pinned, checked against an independent python FNV-1a and SplitMix64. if it changes every saved seed gives different cases.
        assert_eq!(
            Rng::from_text("texcopy").next_u64(),
            11_567_265_878_473_622_684
        );
    }

    #[test]
    fn ranges_and_weights_stay_inside_their_bounds() {
        let mut r = Rng::from_text("bounds");
        for _ in 0..1000 {
            assert!((3..=9).contains(&r.int(3, 9)));
            assert!((0.0..1.0).contains(&r.float()));
        }
        let picks: Vec<u8> = (0..2000)
            .map(|_| *r.weighted(&[(1u8, 90), (2u8, 10)]))
            .collect();
        let twos = picks.iter().filter(|&&x| x == 2).count();
        assert!((100..300).contains(&twos), "about 10% twos, got {twos}");
    }
}
