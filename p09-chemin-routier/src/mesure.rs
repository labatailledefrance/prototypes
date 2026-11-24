//! Mesures : mémoire du processus, percentiles, générateur pseudo-aléatoire.

pub fn rss_mo() -> (f64, f64) {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let champ = |k: &str| {
        s.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 1024.0
    };
    (champ("VmRSS:"), champ("VmHWM:"))
}

/// Percentile (0..=100) d'une liste, par rang le plus proche.
pub fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut w = v.to_vec();
    w.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let k = ((p / 100.0) * (w.len() as f64 - 1.0)).round() as usize;
    w[k]
}

pub fn moyenne(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

/// SplitMix64 : graine fixe, même suite partout.
pub struct Alea(pub u64);

impl Alea {
    pub fn suivant(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    pub fn sous(&mut self, n: usize) -> usize {
        (self.suivant() % n as u64) as usize
    }
}

pub fn zstd_octets(b: &[u8]) -> usize {
    zstd::bulk::compress(b, 19).unwrap().len()
}

pub fn u32_octets(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Chronomètre en millisecondes.
pub fn ms<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = std::time::Instant::now();
    let r = f();
    (r, t.elapsed().as_secs_f64() * 1000.0)
}
