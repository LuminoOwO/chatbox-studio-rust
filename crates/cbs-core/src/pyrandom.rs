//! Python's `random.Random(seed).randrange(n)` (Mersenne Twister), so rotating messages
//! with "random order" pick the same message in the same time slot as the Python version.

const N: usize = 624;
const M: usize = 397;

pub struct PyRandom {
    mt: [u32; N],
    idx: usize,
}

impl PyRandom {
    fn init_genrand(s: u32) -> [u32; N] {
        let mut mt = [0u32; N];
        mt[0] = s;
        for i in 1..N {
            mt[i] = 1812433253u32
                .wrapping_mul(mt[i - 1] ^ (mt[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        mt
    }

    /// `random.Random(seed)` for an integer seed.
    pub fn new(seed: i64) -> PyRandom {
        let mut a = seed.unsigned_abs();
        let mut key: Vec<u32> = Vec::new();
        while a > 0 {
            key.push(a as u32);
            a >>= 32;
        }
        if key.is_empty() {
            key.push(0);
        }
        let mut mt = Self::init_genrand(19650218);
        let (mut i, mut j) = (1usize, 0usize);
        let mut k = N.max(key.len());
        while k > 0 {
            mt[i] = (mt[i] ^ (mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1664525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= N {
                mt[0] = mt[N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
            k -= 1;
        }
        k = N - 1;
        while k > 0 {
            mt[i] = (mt[i] ^ (mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1566083941))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= N {
                mt[0] = mt[N - 1];
                i = 1;
            }
            k -= 1;
        }
        mt[0] = 0x8000_0000;
        PyRandom { mt, idx: N }
    }

    fn next_u32(&mut self) -> u32 {
        if self.idx >= N {
            for kk in 0..N {
                let y = (self.mt[kk] & 0x8000_0000) | (self.mt[(kk + 1) % N] & 0x7fff_ffff);
                let mag = if y & 1 == 1 { 0x9908_b0df } else { 0 };
                self.mt[kk] = self.mt[(kk + M) % N] ^ (y >> 1) ^ mag;
            }
            self.idx = 0;
        }
        let mut y = self.mt[self.idx];
        self.idx += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    /// `randrange(n)` for 0 < n < 2**32.
    pub fn randrange(&mut self, n: u32) -> u32 {
        if n <= 1 {
            // Python still draws one number for n == 1; the result is always 0
            if n == 1 {
                self.next_u32();
            }
            return 0;
        }
        let k = 32 - n.leading_zeros();
        loop {
            let r = self.next_u32() >> (32 - k);
            if r < n {
                return r;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_python() {
        let cases = [(0i64, 10u32, 6u32), (1, 3, 0), (12345678, 7, 5), ((1i64 << 40) + 5, 5, 4),
                     (-3, 4, 1), (178000000, 2, 0)];
        for (seed, n, want) in cases {
            assert_eq!(PyRandom::new(seed).randrange(n), want, "seed {seed}");
        }
    }
}
