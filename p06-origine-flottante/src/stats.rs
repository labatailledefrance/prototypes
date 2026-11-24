//! Statistiques et sortie CSV.

pub fn quantile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((v.len() - 1) as f64 * q).round() as usize;
    v[i]
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Resume {
    pub n: usize,
    pub mediane: f64,
    pub p99: f64,
    pub max: f64,
}

pub fn resumer(v: &[f64]) -> Resume {
    let mut w: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if w.is_empty() {
        return Resume { n: 0, mediane: f64::NAN, p99: f64::NAN, max: f64::NAN };
    }
    let n = w.len();
    let mediane = quantile(&mut w, 0.5);
    let p99 = quantile(&mut w, 0.99);
    let max = *w.last().unwrap();
    Resume { n, mediane, p99, max }
}

/// Formatage court pour les CSV.
pub fn f(x: f64) -> String {
    if !x.is_finite() {
        return "nan".into();
    }
    let a = x.abs();
    if a == 0.0 {
        "0".into()
    } else if a >= 100.0 {
        format!("{x:.1}")
    } else if a >= 1.0 {
        format!("{x:.3}")
    } else if a >= 0.001 {
        format!("{x:.5}")
    } else {
        format!("{x:.3e}")
    }
}

/// Hachage FNV-1a 64 bits, pour les sommes de contrôle.
#[derive(Clone, Copy)]
pub struct Fnv(pub u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    pub fn u64(&mut self, x: u64) {
        for b in x.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    pub fn u32(&mut self, x: u32) {
        self.u64(x as u64)
    }
}

/// Mélangeur (splitmix64) pour les tirages déterministes.
pub fn melange(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}
