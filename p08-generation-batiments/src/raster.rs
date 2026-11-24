//! Rastérisation à la règle du **centre de case** (copiée de P-07,
//! `raster.rs`) : une case est couverte si son centre est dans le polygone
//! (pair-impair). Elle ne dépend que des coordonnées globales de la case.
//! Adaptée ici à des anneaux `[f64; 2]` (m, locaux).

/// Grille de cases : `nx × ny` cases de `res` m, origine (x0, y0) au coin
/// sud-ouest, ligne j croissante vers le nord.
#[derive(Clone, Copy)]
pub struct Fenetre {
    pub x0: f64,
    pub y0: f64,
    pub res: f64,
    pub nx: usize,
    pub ny: usize,
}

/// Appelle `f(i, j)` pour chaque case dont le centre est dans le polygone.
/// Table d'arêtes triées par ligne de départ : le coût suit le nombre
/// d'arêtes utiles, même pour une commune de milliers de sommets.
pub fn polygone(w: Fenetre, poly: &[Vec<[f64; 2]>], mut f: impl FnMut(usize, usize)) {
    let (nx, ny) = (w.nx as f64, w.ny as f64);
    // Arêtes ramenées en cases : (ligne de départ, ligne de fin, x à y0, pente).
    let mut aretes: Vec<(i64, i64, f64, f64, f64)> = Vec::new();
    for anneau in poly {
        let k = anneau.len();
        for a in 0..k {
            let p = anneau[a];
            let q = anneau[(a + 1) % k];
            let (ax, ay, bx, by) = ((p[0] - w.x0) / w.res, (p[1] - w.y0) / w.res, (q[0] - w.x0) / w.res, (q[1] - w.y0) / w.res);
            if ay == by {
                continue;
            }
            let (lo, hi) = (ay.min(by), ay.max(by));
            // Lignes j dont le centre yc = j + 0.5 est dans [lo, hi[ (règle demi-ouverte).
            let j0 = ((lo - 0.5).ceil() as i64).max(0);
            let j1 = ((hi - 0.5).ceil() as i64 - 1).min(ny as i64 - 1);
            // Les arêtes hors de la fenêtre en x sont gardées : elles
            // apparient les intersections (règle pair-impair).
            if j1 < j0 {
                continue;
            }
            aretes.push((j0, j1, ax, ay, (bx - ax) / (by - ay)));
        }
    }
    if aretes.is_empty() {
        return;
    }
    aretes.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.partial_cmp(&b.2).unwrap()));
    let jmin = aretes[0].0;
    let jmax = aretes.iter().map(|e| e.1).max().unwrap();
    let mut actives: Vec<usize> = Vec::new();
    let mut prochaine = 0;
    let mut xs: Vec<f64> = Vec::new();
    for j in jmin..=jmax {
        while prochaine < aretes.len() && aretes[prochaine].0 == j {
            actives.push(prochaine);
            prochaine += 1;
        }
        actives.retain(|&e| aretes[e].1 >= j);
        let yc = j as f64 + 0.5;
        xs.clear();
        for &e in &actives {
            let (_, _, ax, ay, pente) = aretes[e];
            xs.push(ax + (yc - ay) * pente);
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for k in (0..xs.len().saturating_sub(1)).step_by(2) {
            let i0 = (xs[k] - 0.5).ceil().max(0.0);
            let i1 = (xs[k + 1] - 0.5).floor().min(nx - 1.0);
            if i1 >= i0 {
                for i in i0 as usize..=i1 as usize {
                    f(i, j as usize);
                }
            }
        }
    }
}

/// Appelle `f(i, j)` pour chaque case dont le centre est à moins de
/// `demi` m de la polyligne.
pub fn ligne(w: Fenetre, l: &[[f64; 2]], demi: f64, mut f: impl FnMut(usize, usize)) {
    let d = demi / w.res;
    let (nx, ny) = (w.nx as f64, w.ny as f64);
    for s in l.windows(2) {
        let (ax, ay) = ((s[0][0] - w.x0) / w.res, (s[0][1] - w.y0) / w.res);
        let (bx, by) = ((s[1][0] - w.x0) / w.res, (s[1][1] - w.y0) / w.res);
        let i0 = (ax.min(bx) - d - 0.5).ceil().max(0.0);
        let i1 = (ax.max(bx) + d - 0.5).floor().min(nx - 1.0);
        let j0 = (ay.min(by) - d - 0.5).ceil().max(0.0);
        let j1 = (ay.max(by) + d - 0.5).floor().min(ny - 1.0);
        if i1 < i0 || j1 < j0 {
            continue;
        }
        let (vx, vy) = (bx - ax, by - ay);
        let l2 = vx * vx + vy * vy;
        for j in j0 as usize..=j1 as usize {
            for i in i0 as usize..=i1 as usize {
                let (px, py) = (i as f64 + 0.5 - ax, j as f64 + 0.5 - ay);
                let t = if l2 > 0.0 { ((px * vx + py * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                let (dx, dy) = (px - t * vx, py - t * vy);
                if dx * dx + dy * dy <= d * d {
                    f(i, j);
                }
            }
        }
    }
}

/// Bitmap de `n × n` bits, case (i, j) au bit j·n + i, mots u64.
#[derive(Clone)]
pub struct Bitmap {
    pub n: usize,
    pub mots: Vec<u64>,
}

impl Bitmap {
    pub fn new(n: usize) -> Bitmap {
        Bitmap { n, mots: vec![0; (n * n).div_ceil(64)] }
    }
    #[inline]
    pub fn get(&self, i: usize, j: usize) -> bool {
        let k = j * self.n + i;
        self.mots[k >> 6] >> (k & 63) & 1 == 1
    }
    #[inline]
    pub fn set(&mut self, i: usize, j: usize) {
        let k = j * self.n + i;
        self.mots[k >> 6] |= 1 << (k & 63);
    }
    #[inline]
    pub fn clear(&mut self, i: usize, j: usize) {
        let k = j * self.n + i;
        self.mots[k >> 6] &= !(1 << (k & 63));
    }
    pub fn compte(&self) -> usize {
        self.mots.iter().map(|m| m.count_ones() as usize).sum()
    }
    pub fn octets(&self) -> Vec<u8> {
        self.mots.iter().flat_map(|m| m.to_le_bytes()).collect()
    }
    pub fn de_octets(n: usize, b: &[u8]) -> Bitmap {
        Bitmap { n, mots: b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect() }
    }
    pub fn vide(&self) -> bool {
        self.mots.iter().all(|m| *m == 0)
    }
}

