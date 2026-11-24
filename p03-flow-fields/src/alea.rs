//! Générateur pseudo-aléatoire déterministe (SplitMix64), sans dépendance.

pub struct Alea(u64);

/// Hachage de deux entiers : même entrée, même sortie, quel que soit le
/// thread. Sert aux décisions « aléatoires » des systèmes parallèles.
#[inline(always)]
pub fn hacher(a: u64, b: u64) -> u64 {
    let mut z = a
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(b)
        .wrapping_add(0x6A09_E667_F3BC_C909);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Flottant dans [0, 1) à partir d'un hachage.
#[inline(always)]
pub fn unite(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

impl Alea {
    pub fn new(graine: u64) -> Self {
        Alea(graine)
    }
    pub fn u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1);
        hacher(self.0, 0)
    }
    pub fn f32(&mut self) -> f32 {
        unite(self.u64())
    }
    /// Entier dans [a, b].
    pub fn entre(&mut self, a: usize, b: usize) -> usize {
        a + (self.u64() % (b - a + 1) as u64) as usize
    }
}
