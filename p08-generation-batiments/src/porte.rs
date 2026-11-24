//! Porte unique d'un bâtiment (B-02), placée de façon déterministe.
//!
//! **Règle** : la porte est sur un mur extérieur, devant une case de 2 m de
//! la **zone accessible** (franchissable, hors des cases exclues de P-07),
//! celle des cases au pied des murs qui est la **plus proche de la rue à
//! pied**. La distance à la rue est un parcours en largeur sur les cases
//! accessibles, depuis les cases des routes au sol (P-07, section `reseau`).
//! Candidats : tous les 0,5 m le long de chaque arête extérieure d'au moins
//! 1,6 m, à 1, puis 2, puis 3 m du mur. Égalités : distance, puis éloignement
//! du mur, puis arête, puis abscisse (ordre fixe).
//!
//! **Règle provisoire de P-05**, pour comparaison : la case accessible la
//! plus proche du centroïde.
//!
//! Analogie : choisir la porte, c'est placer le point d'entrée d'un service
//! (son « ingress ») du côté de la route publique, pas sur le réseau interne.

use crate::batiment::{Porte, anneaux_m};
use crate::donnees::{Bat, Contexte};
use crate::geom::{self, P2};
use crate::raster::{self, Fenetre};
use std::collections::VecDeque;

pub const MARGE: i32 = 100;
pub const N: usize = 500 + 2 * MARGE as usize;
pub const LOIN: u16 = u16::MAX;

/// Grille d'accès autour de la tile (500 cases + 2 × 100 de marge).
pub struct Grille {
    pub acc: Vec<bool>,
    pub route: Vec<bool>,
    /// Distance à la rue (cases de 2 m, à pied), LOIN si aucune.
    pub dist: Vec<u16>,
}

impl Grille {
    #[inline]
    pub fn k(gi: i32, gj: i32) -> Option<usize> {
        let (i, j) = (gi + MARGE, gj + MARGE);
        (i >= 0 && j >= 0 && (i as usize) < N && (j as usize) < N).then(|| j as usize * N + i as usize)
    }
}

/// Natures de route qui comptent comme « rue » : du type autoroutier au
/// chemin (P-07, `NATURES_ROUTE` 0 à 6), au sol.
fn est_rue(nature: u8, pos: i8) -> bool {
    nature <= 6 && pos == 0
}

pub fn grille(ctx: &Contexte) -> Grille {
    let mut acc = vec![false; N * N];
    for j in 0..N as i32 {
        for i in 0..N as i32 {
            acc[j as usize * N + i as usize] = ctx.accessible(i - MARGE, j - MARGE);
        }
    }
    let mut route = vec![false; N * N];
    let w = Fenetre { x0: -500.0 - 2.0 * MARGE as f64, y0: -500.0 - 2.0 * MARGE as f64, res: 2.0, nx: N, ny: N };
    for dy in -1..=1i32 {
        for dx in -1..=1i32 {
            let Some(t) = &ctx.tiles[((dy + 1) * 3 + dx + 1) as usize] else { continue };
            let (ox, oy) = (dx as f64 * 1000.0, dy as f64 * 1000.0);
            for r in &t.routes {
                if !est_rue(r.nature, r.pos_sol) {
                    continue;
                }
                let l: Vec<P2> = r.pts.iter().map(|p| [p[0] as f64 + ox, p[1] as f64 + oy]).collect();
                raster::ligne(w, &l, 1.5, |i, j| route[j * N + i] = true);
            }
        }
    }
    let mut dist = vec![LOIN; N * N];
    let mut f = VecDeque::new();
    for k in 0..N * N {
        if route[k] && acc[k] {
            dist[k] = 0;
            f.push_back(k);
        }
    }
    while let Some(k) = f.pop_front() {
        let (i, j) = (k % N, k / N);
        let d = dist[k] + 1;
        let mut v = |kk: usize| {
            if acc[kk] && dist[kk] == LOIN {
                dist[kk] = d;
                f.push_back(kk);
            }
        };
        if i > 0 {
            v(k - 1);
        }
        if i + 1 < N {
            v(k + 1);
        }
        if j > 0 {
            v(k - N);
        }
        if j + 1 < N {
            v(k + N);
        }
    }
    Grille { acc, route, dist }
}

#[inline]
pub fn case_de(q: P2) -> (i32, i32) {
    (((q[0] + 500.0) / 2.0).floor() as i32, ((q[1] + 500.0) / 2.0).floor() as i32)
}

#[inline]
pub fn centre_case(c: (i32, i32)) -> P2 {
    [c.0 as f64 * 2.0 - 499.0, c.1 as f64 * 2.0 - 499.0]
}

#[derive(Clone, Copy, Debug)]
pub enum Methode {
    /// Case au pied du mur, à 1, 2 ou 3 m.
    Mur(u8),
    /// Aucune case accessible au pied des murs : règle de P-05.
    Centroide,
    /// Ni l'une ni l'autre dans la fenêtre.
    Aucune,
}

#[derive(Clone, Copy, Debug)]
pub struct Choix {
    pub porte: Option<Porte>,
    pub case: Option<(i32, i32)>,
    pub methode: Methode,
    /// Distance à la rue (m) depuis la case de la porte, si connue.
    pub rue_m: Option<f64>,
    /// Règle de P-05 : case, distance au mur (m), distance à la rue (m).
    pub p05_case: Option<(i32, i32)>,
    pub p05_mur_m: f64,
    pub p05_rue_m: Option<f64>,
    /// Distance entre la porte de P-08 et la case de P-05 (m).
    pub ecart_m: f64,
}

fn rue(g: &Grille, c: (i32, i32)) -> Option<f64> {
    Grille::k(c.0, c.1).and_then(|k| (g.dist[k] != LOIN).then(|| g.dist[k] as f64 * 2.0))
}

/// Centroïde de l'anneau extérieur du plus grand polygone.
pub fn centroide(b: &Bat) -> P2 {
    let poly = b.polys.iter().max_by_key(|p| geom::aire2_i(&p[0]).abs()).unwrap();
    let a: Vec<P2> = poly[0].iter().map(|q| geom::m(*q)).collect();
    let (mut cx, mut cy, mut s) = (0.0, 0.0, 0.0);
    for i in 0..a.len() {
        let (p, q) = (a[i], a[(i + 1) % a.len()]);
        let c = p[0] * q[1] - q[0] * p[1];
        cx += (p[0] + q[0]) * c;
        cy += (p[1] + q[1]) * c;
        s += c;
    }
    if s.abs() < 1e-9 { a[0] } else { [cx / (3.0 * s), cy / (3.0 * s)] }
}

/// Case accessible la plus proche d'un point (règle de P-05), dans la grille.
pub fn plus_proche(g: &Grille, p: P2) -> Option<(i32, i32)> {
    let c = case_de(p);
    let mut best: Option<(f64, (i32, i32))> = None;
    for r in 0..N as i32 {
        // Anneau de Tchebychev r ; on s'arrête quand plus rien ne peut battre.
        if let Some((d, _)) = best
            && (r as f64 - 1.0) * 2.0 > d
        {
            break;
        }
        for dj in -r..=r {
            for di in -r..=r {
                if di.abs() != r && dj.abs() != r {
                    continue;
                }
                let cc = (c.0 + di, c.1 + dj);
                let Some(k) = Grille::k(cc.0, cc.1) else { continue };
                if !g.acc[k] {
                    continue;
                }
                let q = centre_case(cc);
                let (ex, ey) = (q[0] - p[0], q[1] - p[1]);
                let d = (ex * ex + ey * ey).sqrt();
                if best.is_none_or(|(bd, bc)| (d, (cc.1, cc.0)) < (bd, (bc.1, bc.0))) {
                    best = Some((d, cc));
                }
            }
        }
        if r > 60 && best.is_none() {
            break;
        }
    }
    best.map(|b| b.1)
}

fn dist_mur(p: P2, an: &[Vec<P2>]) -> f64 {
    let mut d = f64::MAX;
    for a in an {
        for i in 0..a.len() {
            d = d.min(geom::dist_seg(p, a[i], a[(i + 1) % a.len()]));
        }
    }
    d
}

pub fn choisir(b: &Bat, g: &Grille) -> Choix {
    // Candidats : (distance à la rue, rang d'éloignement, arête, abscisse).
    let mut best: Option<((u16, u8, u32, u32), Porte, (i32, i32))> = None;
    let mut ka = 0u32;
    let mut tous: Vec<Vec<P2>> = Vec::new();
    for poly in &b.polys {
        let an = anneaux_m(poly, None);
        if an.is_empty() {
            continue;
        }
        let ext = &an[0];
        for i in 0..ext.len() {
            let (pa, pb) = (ext[i], ext[(i + 1) % ext.len()]);
            let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
            let len = (dx * dx + dy * dy).sqrt();
            ka += 1;
            if len < 1.6 {
                continue;
            }
            let (t, n) = ([dx / len, dy / len], [dy / len, -dx / len]);
            let ns = ((len - 1.6) / 0.5).floor() as u32 + 1;
            let s0 = (len - (ns - 1) as f64 * 0.5) / 2.0;
            for ks in 0..ns {
                let s = s0 + ks as f64 * 0.5;
                let p = [pa[0] + t[0] * s, pa[1] + t[1] * s];
                for (r, off) in [1.0f64, 2.0, 3.0].iter().enumerate() {
                    let c = case_de([p[0] + n[0] * off, p[1] + n[1] * off]);
                    let Some(k) = Grille::k(c.0, c.1) else { continue };
                    if !g.acc[k] {
                        continue;
                    }
                    let cle = (g.dist[k], r as u8, ka, ks);
                    if best.as_ref().is_none_or(|x| cle < x.0) {
                        best = Some((cle, Porte { p, n }, c));
                    }
                    break;
                }
            }
        }
        tous.extend(an);
    }
    let cen = centroide(b);
    let p05 = plus_proche(g, cen);
    let p05_mur_m = p05.map(|c| dist_mur(centre_case(c), &tous)).unwrap_or(f64::NAN);
    let p05_rue_m = p05.and_then(|c| rue(g, c));
    match best {
        Some((cle, porte, c)) => {
            let ecart = p05.map(|q| {
                let (a, bb) = (centre_case(q), centre_case(c));
                let (ex, ey) = (a[0] - bb[0], a[1] - bb[1]);
                (ex * ex + ey * ey).sqrt()
            });
            Choix { porte: Some(porte), case: Some(c), methode: Methode::Mur(cle.1 + 1), rue_m: rue(g, c), p05_case: p05, p05_mur_m, p05_rue_m, ecart_m: ecart.unwrap_or(f64::NAN) }
        }
        None => {
            // Règle de P-05 : la porte va au point du mur le plus proche.
            let porte = p05.and_then(|c| {
                let q = centre_case(c);
                let mut bp: Option<(f64, Porte)> = None;
                for a in &tous {
                    for i in 0..a.len() {
                        let (pa, pb) = (a[i], a[(i + 1) % a.len()]);
                        let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
                        let l2 = dx * dx + dy * dy;
                        if l2 == 0.0 {
                            continue;
                        }
                        let t = (((q[0] - pa[0]) * dx + (q[1] - pa[1]) * dy) / l2).clamp(0.0, 1.0);
                        let p = [pa[0] + t * dx, pa[1] + t * dy];
                        let d = geom::dist_seg(q, pa, pb);
                        let l = l2.sqrt();
                        if bp.as_ref().is_none_or(|x| d < x.0) {
                            bp = Some((d, Porte { p, n: [dy / l, -dx / l] }));
                        }
                    }
                }
                bp.map(|x| x.1)
            });
            Choix {
                porte,
                case: p05,
                methode: if p05.is_some() { Methode::Centroide } else { Methode::Aucune },
                rue_m: p05_rue_m,
                p05_case: p05,
                p05_mur_m,
                p05_rue_m,
                ecart_m: 0.0,
            }
        }
    }
}
