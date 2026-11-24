//! Statistiques de temps et mémoire résidente.

use std::time::Duration;

pub struct Stats {
    pub mediane_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub moyenne_ms: f64,
}

pub fn stats(mut d: Vec<Duration>) -> Stats {
    d.sort_unstable();
    let ms = |x: Duration| x.as_secs_f64() * 1e3;
    let n = d.len();
    let rang = |q: f64| ((q * n as f64).ceil() as usize).clamp(1, n) - 1;
    Stats {
        mediane_ms: ms(d[rang(0.5)]),
        p99_ms: ms(d[rang(0.99)]),
        max_ms: ms(d[n - 1]),
        moyenne_ms: d.iter().map(|x| ms(*x)).sum::<f64>() / n as f64,
    }
}

/// Mémoire résidente du processus en Mo (Linux : /proc/self/status).
pub fn rss_mo() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let ligne = s.lines().find(|l| l.starts_with("VmRSS:"))?;
    let ko: f64 = ligne.split_whitespace().nth(1)?.parse().ok()?;
    Some(ko / 1024.0)
}
