//! Géométrie 2D de la génération.
//!
//! **Déterminisme** : tout part des sommets entiers de P-07 (1/16 m). Les
//! calculs exacts (enveloppe convexe, orientation, simplification) sont en
//! entiers. Les autres n'utilisent que `+ − × ÷` et `sqrt`, que la norme IEEE
//! 754 arrondit exactement : même résultat au bit près sur toute machine,
//! tant qu'on n'appelle ni `sin`, ni `atan2`, ni `powf`, ni `hypot` (fonctions
//! de la bibliothèque mathématique du système, voir P-18). Rust ne fusionne
//! jamais `a * b + c` en FMA de lui-même.

pub type P2 = [f64; 2];

pub const Q: f64 = 16.0;

#[inline]
pub fn m(p: [i32; 2]) -> P2 {
    [p[0] as f64 / Q, p[1] as f64 / Q]
}

#[inline]
fn cross_i(o: [i32; 2], a: [i32; 2], b: [i32; 2]) -> i64 {
    (a[0] as i64 - o[0] as i64) * (b[1] as i64 - o[1] as i64) - (a[1] as i64 - o[1] as i64) * (b[0] as i64 - o[0] as i64)
}

/// Aire signée ×2 en unités entières (exacte).
pub fn aire2_i(a: &[[i32; 2]]) -> i64 {
    let n = a.len();
    (0..n).map(|i| a[i][0] as i64 * a[(i + 1) % n][1] as i64 - a[(i + 1) % n][0] as i64 * a[i][1] as i64).sum()
}

pub fn aire_f(a: &[P2]) -> f64 {
    let n = a.len();
    let mut s = 0.0;
    for i in 0..n {
        let (p, q) = (a[i], a[(i + 1) % n]);
        s += p[0] * q[1] - q[0] * p[1];
    }
    s / 2.0
}

/// Enveloppe convexe (chaîne monotone d'Andrew), sens direct, exacte.
pub fn enveloppe(pts: &[[i32; 2]]) -> Vec<[i32; 2]> {
    let mut p: Vec<[i32; 2]> = pts.to_vec();
    p.sort_unstable();
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let mut h: Vec<[i32; 2]> = Vec::with_capacity(2 * p.len());
    for &q in &p {
        while h.len() >= 2 && cross_i(h[h.len() - 2], h[h.len() - 1], q) <= 0 {
            h.pop();
        }
        h.push(q);
    }
    let lo = h.len() + 1;
    for &q in p.iter().rev().skip(1) {
        while h.len() >= lo && cross_i(h[h.len() - 2], h[h.len() - 1], q) <= 0 {
            h.pop();
        }
        h.push(q);
    }
    h.pop();
    h
}

/// Rectangle englobant orienté d'aire minimale (calipers sur les arêtes de
/// l'enveloppe) : centre, axe long `a`, axe court `b` (unitaires, b = a
/// tourné de +90°), demi-longueur `l` ≥ demi-largeur `w`.
#[derive(Clone, Copy, Debug)]
pub struct Obb {
    pub c: P2,
    pub a: P2,
    pub b: P2,
    pub l: f64,
    pub w: f64,
}

impl Obb {
    #[inline]
    pub fn uv(&self, p: P2) -> (f64, f64) {
        let d = [p[0] - self.c[0], p[1] - self.c[1]];
        (d[0] * self.a[0] + d[1] * self.a[1], d[0] * self.b[0] + d[1] * self.b[1])
    }
    pub fn allongement(&self) -> f64 {
        self.l / self.w.max(1e-6)
    }
}

pub fn obb(pts: &[[i32; 2]]) -> Obb {
    let h = enveloppe(pts);
    if h.len() < 3 {
        let c = h.first().map(|p| m(*p)).unwrap_or([0.0, 0.0]);
        return Obb { c, a: [1.0, 0.0], b: [0.0, 1.0], l: 0.5, w: 0.5 };
    }
    let hm: Vec<P2> = h.iter().map(|p| m(*p)).collect();
    let mut best: Option<(f64, Obb)> = None;
    for k in 0..h.len() {
        let (p, q) = (h[k], h[(k + 1) % h.len()]);
        let (dx, dy) = ((q[0] - p[0]) as f64, (q[1] - p[1]) as f64);
        let n = (dx * dx + dy * dy).sqrt();
        if n == 0.0 {
            continue;
        }
        let a = [dx / n, dy / n];
        let b = [-a[1], a[0]];
        let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for s in &hm {
            let u = s[0] * a[0] + s[1] * a[1];
            let v = s[0] * b[0] + s[1] * b[1];
            u0 = u0.min(u);
            u1 = u1.max(u);
            v0 = v0.min(v);
            v1 = v1.max(v);
        }
        let aire = (u1 - u0) * (v1 - v0);
        if best.as_ref().is_none_or(|(x, _)| aire < *x) {
            let (uc, vc) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
            let c = [uc * a[0] + vc * b[0], uc * a[1] + vc * b[1]];
            let (hl, hw) = ((u1 - u0) / 2.0, (v1 - v0) / 2.0);
            let o = if hl >= hw { Obb { c, a, b, l: hl, w: hw } } else { Obb { c, a: b, b: [-b[1], b[0]], l: hw, w: hl } };
            best = Some((aire, o));
        }
    }
    best.unwrap().1
}

/// Demi-plan `α u + β v + γ ≥ 0` dans le repère (u, v) d'un `Obb`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DemiPlan {
    pub al: f64,
    pub be: f64,
    pub ga: f64,
}

impl DemiPlan {
    #[inline]
    pub fn f(&self, o: &Obb, p: P2) -> f64 {
        let (u, v) = o.uv(p);
        self.al * u + self.be * v + self.ga
    }
}

/// Point d'intersection du segment [P, Q] avec la droite f = 0 (fp et fq de
/// signes opposés). Calcul unique, réutilisé par les murs et le toit.
#[inline]
pub fn coupe(p: P2, q: P2, fp: f64, fq: f64) -> P2 {
    let t = fp / (fp - fq);
    [p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]
}

/// Point de coupe **canonique** du segment [P, Q] par la droite du demi-plan :
/// extrémités rangées avant le calcul, pour que le mur et le toit, qui
/// parcourent la même arête dans des sens opposés, obtiennent les mêmes bits.
#[inline]
pub fn coupe_canon(p: P2, q: P2, d: &DemiPlan, o: &Obb) -> P2 {
    let (a, b) = if (p[0], p[1]) <= (q[0], q[1]) { (p, q) } else { (q, p) };
    coupe(a, b, d.f(o, a), d.f(o, b))
}

/// Sutherland-Hodgman : partie d'un anneau dans le demi-plan.
pub fn couper(anneau: &[P2], d: &DemiPlan, o: &Obb) -> Vec<P2> {
    let n = anneau.len();
    let mut out = Vec::with_capacity(n + 4);
    if n == 0 {
        return out;
    }
    let f: Vec<f64> = anneau.iter().map(|p| d.f(o, *p)).collect();
    for i in 0..n {
        let j = (i + 1) % n;
        let (p, q) = (anneau[i], anneau[j]);
        if f[i] >= 0.0 {
            out.push(p);
        }
        if (f[i] >= 0.0) != (f[j] >= 0.0) {
            out.push(coupe(p, q, f[i], f[j]));
        }
    }
    out.dedup();
    while out.len() > 1 && out[0] == out[out.len() - 1] {
        out.pop();
    }
    out
}

/// Triangulation (earcut) d'un polygone : anneau extérieur puis trous.
/// Renvoie les sommets aplatis et les triangles (sens direct).
pub fn trianguler(anneaux: &[Vec<P2>]) -> Option<(Vec<P2>, Vec<u32>)> {
    let mut sommets: Vec<f64> = Vec::new();
    let mut trous = Vec::new();
    let mut pts: Vec<P2> = Vec::new();
    for (k, a) in anneaux.iter().enumerate() {
        if a.len() < 3 {
            if k == 0 {
                return None;
            }
            continue;
        }
        if k > 0 {
            trous.push(pts.len());
        }
        for p in a {
            sommets.push(p[0]);
            sommets.push(p[1]);
            pts.push(*p);
        }
    }
    let t = earcutr::earcut(&sommets, &trous, 2).ok()?;
    let mut idx: Vec<u32> = Vec::with_capacity(t.len());
    for tri in t.chunks_exact(3) {
        let (a, b, c) = (pts[tri[0]], pts[tri[1]], pts[tri[2]]);
        let s = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if s > 0.0 {
            idx.extend_from_slice(&[tri[0] as u32, tri[1] as u32, tri[2] as u32]);
        } else if s < 0.0 {
            idx.extend_from_slice(&[tri[0] as u32, tri[2] as u32, tri[1] as u32]);
        }
    }
    Some((pts, idx))
}

/// Douglas-Peucker sur un anneau fermé, en entiers (distance comparée au
/// carré, en i128) : déterministe. `tol` en unités de 1/16 m.
pub fn simplifier(a: &[[i32; 2]], tol: i64) -> Vec<[i32; 2]> {
    let n = a.len();
    if n <= 4 {
        return a.to_vec();
    }
    // Point le plus éloigné du premier : coupe l'anneau en deux chaînes.
    let d2 = |p: [i32; 2], q: [i32; 2]| {
        let (dx, dy) = ((p[0] - q[0]) as i64, (p[1] - q[1]) as i64);
        dx * dx + dy * dy
    };
    let far = (1..n).max_by_key(|&k| (d2(a[0], a[k]), std::cmp::Reverse(k))).unwrap();
    let mut garde = vec![false; n];
    garde[0] = true;
    garde[far] = true;
    let mut pile = vec![(0usize, far), (far, n)];
    while let Some((i, j)) = pile.pop() {
        let (p, q) = (a[i], a[j % n]);
        let (vx, vy) = ((q[0] - p[0]) as i128, (q[1] - p[1]) as i128);
        let l2 = vx * vx + vy * vy;
        let mut best = (0i128, 0usize);
        for k in i + 1..j {
            let (wx, wy) = ((a[k][0] - p[0]) as i128, (a[k][1] - p[1]) as i128);
            let c = vx * wy - vy * wx;
            // Distance² × l² (ou distance² au point si l = 0).
            let d = if l2 == 0 { (wx * wx + wy * wy) * 1 } else { c * c };
            let lim = if l2 == 0 { (tol as i128) * (tol as i128) } else { (tol as i128) * (tol as i128) * l2 };
            if d > lim && d > best.0 {
                best = (d, k);
            }
        }
        if best.0 > 0 {
            garde[best.1] = true;
            pile.push((i, best.1));
            pile.push((best.1, j));
        }
    }
    let out: Vec<[i32; 2]> = (0..n).filter(|&k| garde[k]).map(|k| a[k]).collect();
    if out.len() < 3 || aire2_i(&out) == 0 { Vec::new() } else { out }
}

/// Point dans un polygone (pair-impair), anneaux en m.
pub fn dedans(p: P2, anneaux: &[Vec<P2>]) -> bool {
    let mut c = false;
    for a in anneaux {
        let n = a.len();
        for i in 0..n {
            let (u, v) = (a[i], a[(i + 1) % n]);
            if (u[1] > p[1]) != (v[1] > p[1]) {
                let x = u[0] + (p[1] - u[1]) * (v[0] - u[0]) / (v[1] - u[1]);
                if p[0] < x {
                    c = !c;
                }
            }
        }
    }
    c
}

/// Distance d'un point à un segment.
pub fn dist_seg(p: P2, a: P2, b: P2) -> f64 {
    let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = vx * vx + vy * vy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * vx + (p[1] - a[1]) * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (dx, dy) = (p[0] - a[0] - t * vx, p[1] - a[1] - t * vy);
    (dx * dx + dy * dy).sqrt()
}
