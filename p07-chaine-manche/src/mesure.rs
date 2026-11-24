//! Chronos et mémoire.

use std::time::Instant;

/// Mémoire résidente maximale du processus (VmHWM), en Mo.
pub fn rss_max_mo() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for l in s.lines() {
        if let Some(v) = l.strip_prefix("VmHWM:") {
            let kb: f64 = v.trim().trim_end_matches("kB").trim().parse().unwrap_or(0.0);
            return kb / 1024.0;
        }
    }
    0.0
}

pub fn rss_mo() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for l in s.lines() {
        if let Some(v) = l.strip_prefix("VmRSS:") {
            let kb: f64 = v.trim().trim_end_matches("kB").trim().parse().unwrap_or(0.0);
            return kb / 1024.0;
        }
    }
    0.0
}

pub struct Etapes {
    pub t0: Instant,
    pub lignes: Vec<(String, f64, f64, f64)>, // étape, s, rss après (Mo), rss max (Mo)
    tc: Instant,
}

impl Etapes {
    pub fn new() -> Self {
        let t = Instant::now();
        Etapes { t0: t, lignes: Vec::new(), tc: t }
    }
    pub fn fin(&mut self, nom: &str) {
        let s = self.tc.elapsed().as_secs_f64();
        self.lignes.push((nom.to_string(), s, rss_mo(), rss_max_mo()));
        eprintln!("[{:8.1} s] {nom} : {s:.2} s, RSS {:.0} Mo (max {:.0} Mo)", self.t0.elapsed().as_secs_f64(), rss_mo(), rss_max_mo());
        self.tc = Instant::now();
    }
}

pub fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

pub fn quantile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let k = ((v.len() - 1) as f64 * q).round() as usize;
    v[k]
}
