// Xoroshiro128Plus
// muy good and fast
//
pub struct X128P {
    s0: u64,
    s1: u64,
}

impl X128P {
    pub fn new(seed: u64) -> Self {
        // expand a 64-bit seed into 128 bits of state
        fn splitmix64(mut x: u64) -> u64 {
            x = x.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }

        let s0 = splitmix64(seed);
        let s1 = splitmix64(seed.wrapping_add(0x9E3779B97F4A7C15));

        Self { s0, s1 }
    }

    #[inline]
    fn rotl(x: u64, k: u32) -> u64 {
        (x << k) | (x >> (64 - k))
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s0.wrapping_add(self.s1);

        let s1 = self.s1 ^ self.s0;
        self.s0 = Self::rotl(self.s0, 55) ^ s1 ^ (s1 << 14);
        self.s1 = Self::rotl(s1, 36);

        result
    }

    pub fn next_f64(&mut self) -> f64 {
        // Scale into [0, 1)
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    pub fn next_f32(&mut self) -> f32 {
        self.next_f64() as f32
    }

    pub fn next_i64_range(&mut self, lower: i64, upper: i64) -> i64 {
        let r = self.next_u64();
        let mut range: u64 = match upper > lower {
            true => (upper - lower).try_into().unwrap(),
            false => (lower - upper).try_into().unwrap(),
        };
        
        let val = ((r as u128 * range as u128) >> 64) as i64;
        lower + val
    }
}

// architecture-dependent fast seeding
//
#[cfg(target_arch = "x86_64")]
pub fn fast_seed() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

// ARM
#[cfg(target_arch = "aarch64")]
pub fn fast_seed() -> u64 {
    unsafe {
        let value: u64;
        core::arch::asm!("mrs {0}, cntvct_el0", out(reg) value);
        value
    }
}

// generic fallback
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub fn fast_seed() -> u64 {
    use core::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(1);
    let a = CTR.fetch_add(0x9e3779b97f4a7c15, Ordering::Relaxed);
    let b = (&CTR as *const _ as usize as u64).rotate_left(17);

    a ^ b
}
