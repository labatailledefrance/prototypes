//! Marche réelle d'un colon sous la caméra : A* sur les cases de 2 m
//! franchissables de la carte de P-04, avec un budget d'expansions par tick
//! (déterministe, contrairement à un budget en temps).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::carte::Carte;

/// Porte d'un bâtiment : la case de 1 m accessible la plus proche de son
/// centroïde (spirale, 80 m au plus).
pub fn porte(c: &Carte, p: [f64; 2]) -> Option<[f64; 2]> {
    let (x0, y0) = (p[0].floor(), p[1].floor());
    if c.accessible_monde(p) {
        return Some([x0 + 0.5, y0 + 0.5]);
    }
    for r in 1..=80i32 {
        let mut best: Option<(f64, [f64; 2])> = None;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dy.abs()) != r {
                    continue;
                }
                let q = [x0 + dx as f64 + 0.5, y0 + dy as f64 + 0.5];
                if c.accessible_monde(q) {
                    let d = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, q));
                    }
                }
            }
        }
        if let Some((_, q)) = best {
            return Some(q);
        }
    }
    None
}

/// Cases de 2 m franchissables de tout le bloc (4 cases de 1 m libres et
/// accessibles), calculées une fois : 1 bit par case.
pub struct Passage {
    pub w: i32,
    pub h: i32,
    pub ox: f64,
    pub oy: f64,
    bits: Vec<u64>,
}

impl Passage {
    pub fn new(c: &Carte) -> Passage {
        let g = c.geo;
        let (w, h) = ((g.w * 500) as i32, (g.h * 500) as i32);
        let (ox, oy) = (g.x0 as f64 * 1000.0, g.y0 as f64 * 1000.0);
        let mut bits = vec![0u64; (w as usize * h as usize).div_ceil(64)];
        for y in 0..h {
            for x in 0..w {
                let ok = (0..2).all(|dy| {
                    (0..2).all(|dx| c.accessible_monde([ox + (x * 2 + dx) as f64 + 0.5, oy + (y * 2 + dy) as f64 + 0.5]))
                });
                if ok {
                    let k = (y * w + x) as usize;
                    bits[k >> 6] |= 1 << (k & 63);
                }
            }
        }
        Passage { w, h, ox, oy, bits }
    }
    #[inline(always)]
    pub fn libre(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return false;
        }
        let k = (y * self.w + x) as usize;
        (self.bits[k >> 6] >> (k & 63)) & 1 == 1
    }
}

/// Tampons réutilisés d'un A* (aucune allocation en régime établi).
#[derive(Default)]
pub struct Astar {
    g: Vec<u32>,
    venu: Vec<u32>,
    gen_: Vec<u32>,
    generation: u32,
    tas: BinaryHeap<Reverse<(u32, u32)>>,
    w: usize,
    h: usize,
    pub expansions: u64,
}

const V8: [(i32, i32, u32); 8] = [(1, 0, 5), (-1, 0, 5), (0, 1, 5), (0, -1, 5), (1, 1, 7), (1, -1, 7), (-1, 1, 7), (-1, -1, 7)];

impl Astar {
    /// Chemin à pied entre deux points (mètres Lambert-93), sur les cases de
    /// 2 m entièrement libres et accessibles, dans un rectangle élargi de
    /// 300 m ; au plus `max_exp` expansions. `None` si aucun chemin.
    pub fn chemin(&mut self, c: &Passage, a: [f64; 2], b: [f64; 2], max_exp: u64) -> Option<Vec<[f64; 2]>> {
        let (ox, oy) = (c.ox, c.oy);
        let (wm, hm) = (c.w, c.h);
        let cell = |p: [f64; 2]| (((p[0] - ox) / 2.0) as i32, ((p[1] - oy) / 2.0) as i32);
        let (ca, cb) = (cell(a), cell(b));
        let marge = 150;
        let (bx0, by0) = ((ca.0.min(cb.0) - marge).max(0), (ca.1.min(cb.1) - marge).max(0));
        let (bx1, by1) = ((ca.0.max(cb.0) + marge).min(wm - 1), (ca.1.max(cb.1) + marge).min(hm - 1));
        let (bw, bh) = ((bx1 - bx0 + 1) as usize, (by1 - by0 + 1) as usize);
        let n = bw * bh;
        if self.gen_.len() < n {
            self.g.resize(n, 0);
            self.venu.resize(n, 0);
            self.gen_.resize(n, 0);
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.gen_.fill(0);
            self.generation = 1;
        }
        self.w = bw;
        self.h = bh;
        let libre = |x: i32, y: i32| -> bool { x >= bx0 && y >= by0 && x <= bx1 && y <= by1 && c.libre(x, y) };
        let proche = |p: (i32, i32)| -> Option<(i32, i32)> {
            for r in 0..20i32 {
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx.abs().max(dy.abs()) == r && libre(p.0 + dx, p.1 + dy) {
                            return Some((p.0 + dx, p.1 + dy));
                        }
                    }
                }
            }
            None
        };
        let (ca, cb) = (proche(ca)?, proche(cb)?);
        let idx = |x: i32, y: i32| (y - by0) as usize * bw + (x - bx0) as usize;
        let hcout = |x: i32, y: i32| {
            let (dx, dy) = ((x - cb.0).unsigned_abs(), (y - cb.1).unsigned_abs());
            let (m, n) = (dx.min(dy), dx.max(dy));
            7 * m + 5 * (n - m)
        };
        self.tas.clear();
        let k0 = idx(ca.0, ca.1);
        self.g[k0] = 0;
        self.gen_[k0] = self.generation;
        self.venu[k0] = u32::MAX;
        self.tas.push(Reverse((hcout(ca.0, ca.1), k0 as u32)));
        let cible = idx(cb.0, cb.1);
        let mut trouve = false;
        let debut = self.expansions;
        while let Some(Reverse((fc, k))) = self.tas.pop() {
            if self.expansions - debut >= max_exp {
                break;
            }
            let k = k as usize;
            let (x, y) = (bx0 + (k % bw) as i32, by0 + (k / bw) as i32);
            let gk = self.g[k];
            if fc != gk + hcout(x, y) {
                continue;
            }
            self.expansions += 1;
            if k == cible {
                trouve = true;
                break;
            }
            for (dx, dy, cout) in V8 {
                let (nx, ny) = (x + dx, y + dy);
                if !libre(nx, ny) || (dx != 0 && dy != 0 && (!libre(x + dx, y) || !libre(x, y + dy))) {
                    continue;
                }
                let nk = idx(nx, ny);
                let ng = gk + cout;
                if self.gen_[nk] != self.generation || ng < self.g[nk] {
                    self.gen_[nk] = self.generation;
                    self.g[nk] = ng;
                    self.venu[nk] = k as u32;
                    self.tas.push(Reverse((ng + hcout(nx, ny), nk as u32)));
                }
            }
        }
        if !trouve {
            return None;
        }
        let mut ch = vec![];
        let mut k = cible;
        while k != u32::MAX as usize {
            let (x, y) = (bx0 + (k % bw) as i32, by0 + (k / bw) as i32);
            ch.push([ox + x as f64 * 2.0 + 1.0, oy + y as f64 * 2.0 + 1.0]);
            k = self.venu[k] as usize;
        }
        ch.reverse();
        Some(ch)
    }
}
