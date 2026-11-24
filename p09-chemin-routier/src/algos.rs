//! Recherches sans prétraitement lourd : Dijkstra (référence exacte),
//! Dijkstra bidirectionnel, A* (borne euclidienne), ALT (repères).
//!
//! Tout est en entiers (temps en ms) ; le tas est ordonné par (clé, nœud),
//! donc les égalités sont départagées par l'identifiant du nœud, et un
//! prédécesseur n'est remplacé que par un chemin strictement meilleur :
//! même requête, même itinéraire.

use crate::classes::INF;
use crate::graphe::Graphe;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Graphe orienté en CSR : arcs sortants et entrants (identifiants d'arc).
pub struct Csr {
    pub n: usize,
    pub tete: Vec<u32>,  // tête de l'arc a
    pub queue: Vec<u32>, // queue de l'arc a
    pub out_deb: Vec<u32>,
    pub out_arc: Vec<u32>,
    pub in_deb: Vec<u32>,
    pub in_arc: Vec<u32>,
}

impl Csr {
    pub fn de(g: &Graphe) -> Csr {
        let n = g.n();
        let m = g.m();
        let mut tete = vec![0u32; 2 * m];
        let mut queue = vec![0u32; 2 * m];
        for e in 0..m {
            queue[2 * e] = g.u[e];
            tete[2 * e] = g.v[e];
            queue[2 * e + 1] = g.v[e];
            tete[2 * e + 1] = g.u[e];
        }
        let construire = |cle: &[u32]| {
            let mut deb = vec![0u32; n + 1];
            for &x in cle {
                deb[x as usize + 1] += 1;
            }
            for i in 0..n {
                deb[i + 1] += deb[i];
            }
            let mut pos = deb.clone();
            let mut arcs = vec![0u32; cle.len()];
            for (a, &x) in cle.iter().enumerate() {
                arcs[pos[x as usize] as usize] = a as u32;
                pos[x as usize] += 1;
            }
            (deb, arcs)
        };
        let (out_deb, out_arc) = construire(&queue);
        let (in_deb, in_arc) = construire(&tete);
        Csr { n, tete, queue, out_deb, out_arc, in_deb, in_arc }
    }
}

/// Résultat d'une requête : durée (ms), arcs du chemin, nœuds explorés.
#[derive(Clone, Debug, Default)]
pub struct Trajet {
    pub duree: u32,
    pub arcs: Vec<u32>,
    pub explores: u32,
}

/// État réutilisable (pas d'allocation par requête : seules les cases
/// touchées sont remises à zéro).
pub struct Etat {
    dist: Vec<u32>,
    pred: Vec<u32>,
    dist_b: Vec<u32>,
    pred_b: Vec<u32>,
    touches: Vec<u32>,
    tas: BinaryHeap<Reverse<(u32, u32)>>,
    tas_b: BinaryHeap<Reverse<(u32, u32)>>,
}

impl Etat {
    pub fn new(n: usize) -> Etat {
        Etat {
            dist: vec![INF; n],
            pred: vec![u32::MAX; n],
            dist_b: vec![INF; n],
            pred_b: vec![u32::MAX; n],
            touches: Vec::new(),
            tas: BinaryHeap::new(),
            tas_b: BinaryHeap::new(),
        }
    }
    fn raz(&mut self) {
        for &x in &self.touches {
            self.dist[x as usize] = INF;
            self.pred[x as usize] = u32::MAX;
            self.dist_b[x as usize] = INF;
            self.pred_b[x as usize] = u32::MAX;
        }
        self.touches.clear();
        self.tas.clear();
        self.tas_b.clear();
    }
    fn chemin(&self, c: &Csr, s: u32, t: u32) -> Vec<u32> {
        let mut arcs = Vec::new();
        let mut x = t;
        while x != s {
            let a = self.pred[x as usize];
            arcs.push(a);
            x = c.queue[a as usize];
        }
        arcs.reverse();
        arcs
    }
}

/// Heuristique de A* : rien, borne euclidienne, ou repères (ALT).
pub enum Borne<'a> {
    Zero,
    Euclide { xy: &'a [[i64; 2]], vmax: u16 },
    Alt { reperes: &'a Reperes },
}

impl Borne<'_> {
    #[inline]
    fn h(&self, v: u32, t: u32) -> u32 {
        match self {
            Borne::Zero => 0,
            Borne::Euclide { xy, vmax } => {
                let (a, b) = (xy[v as usize], xy[t as usize]);
                let (dx, dy) = ((a[0] - b[0]) as f64, (a[1] - b[1]) as f64);
                // mm, arrondi vers le bas : borne inférieure du temps.
                let mm = ((dx * dx + dy * dy).sqrt() * (1000.0 / 16.0)) as u64;
                (36 * mm / (10 * *vmax as u64)) as u32
            }
            Borne::Alt { reperes } => reperes.h(v, t),
        }
    }
}

/// Dijkstra (borne nulle) ou A* (borne admissible), avec réouverture : exact
/// même si la borne n'est pas tout à fait monotone (arrondis).
pub fn astar(c: &Csr, w: &[u32], e: &mut Etat, s: u32, t: u32, b: &Borne) -> Option<Trajet> {
    e.raz();
    e.dist[s as usize] = 0;
    e.touches.push(s);
    e.tas.push(Reverse((b.h(s, t), s)));
    let mut explores = 0u32;
    while let Some(Reverse((f, x))) = e.tas.pop() {
        let dx = e.dist[x as usize];
        if f != dx.saturating_add(b.h(x, t)) {
            continue; // entrée périmée
        }
        explores += 1;
        if x == t {
            return Some(Trajet { duree: dx, arcs: e.chemin(c, s, t), explores });
        }
        for k in c.out_deb[x as usize]..c.out_deb[x as usize + 1] {
            let a = c.out_arc[k as usize];
            let wa = w[a as usize];
            if wa >= INF {
                continue;
            }
            let y = c.tete[a as usize];
            let nd = dx + wa;
            if nd < e.dist[y as usize] {
                if e.dist[y as usize] == INF {
                    e.touches.push(y);
                }
                e.dist[y as usize] = nd;
                e.pred[y as usize] = a;
                e.tas.push(Reverse((nd.saturating_add(b.h(y, t)), y)));
            }
        }
    }
    None
}

/// Dijkstra bidirectionnel. Arrêt quand min(avant) + min(arrière) ≥ meilleur.
pub fn bidir(c: &Csr, w: &[u32], e: &mut Etat, s: u32, t: u32) -> Option<Trajet> {
    e.raz();
    if s == t {
        return Some(Trajet::default());
    }
    e.dist[s as usize] = 0;
    e.dist_b[t as usize] = 0;
    e.touches.push(s);
    e.touches.push(t);
    e.tas.push(Reverse((0, s)));
    e.tas_b.push(Reverse((0, t)));
    let mut best = INF;
    let mut milieu = u32::MAX;
    let mut explores = 0u32;
    loop {
        let tf = e.tas.peek().map(|r| r.0.0).unwrap_or(INF);
        let tb = e.tas_b.peek().map(|r| r.0.0).unwrap_or(INF);
        if tf.saturating_add(tb) >= best || (tf == INF && tb == INF) {
            break;
        }
        let avant = tf <= tb;
        let Reverse((d, x)) = if avant { e.tas.pop().unwrap() } else { e.tas_b.pop().unwrap() };
        let (dd, autre) = if avant { (e.dist[x as usize], e.dist_b[x as usize]) } else { (e.dist_b[x as usize], e.dist[x as usize]) };
        if d != dd {
            continue;
        }
        explores += 1;
        if autre < INF && d + autre < best {
            best = d + autre;
            milieu = x;
        }
        let (deb, arcs) = if avant { (&c.out_deb, &c.out_arc) } else { (&c.in_deb, &c.in_arc) };
        for k in deb[x as usize]..deb[x as usize + 1] {
            let a = arcs[k as usize];
            let wa = w[a as usize];
            if wa >= INF {
                continue;
            }
            let y = if avant { c.tete[a as usize] } else { c.queue[a as usize] };
            let nd = d + wa;
            let cur = if avant { e.dist[y as usize] } else { e.dist_b[y as usize] };
            if nd < cur {
                if e.dist[y as usize] == INF && e.dist_b[y as usize] == INF {
                    e.touches.push(y);
                }
                let other = if avant {
                    e.dist[y as usize] = nd;
                    e.pred[y as usize] = a;
                    e.tas.push(Reverse((nd, y)));
                    e.dist_b[y as usize]
                } else {
                    e.dist_b[y as usize] = nd;
                    e.pred_b[y as usize] = a;
                    e.tas_b.push(Reverse((nd, y)));
                    e.dist[y as usize]
                };
                if other < INF && nd + other < best {
                    best = nd + other;
                    milieu = y;
                }
            }
        }
    }
    if best >= INF {
        return None;
    }
    let mut arcs = Vec::new();
    let mut x = milieu;
    while x != s {
        let a = e.pred[x as usize];
        arcs.push(a);
        x = c.queue[a as usize];
    }
    arcs.reverse();
    let mut x = milieu;
    while x != t {
        let a = e.pred_b[x as usize];
        arcs.push(a);
        x = c.tete[a as usize];
    }
    Some(Trajet { duree: best, arcs, explores })
}

/// Dijkstra complet depuis une source (en avant) ou vers une cible (en
/// arrière) : distances à tous les nœuds.
pub fn tous(c: &Csr, w: &[u32], s: u32, avant: bool) -> Vec<u32> {
    let mut dist = vec![INF; c.n];
    let mut tas = BinaryHeap::new();
    dist[s as usize] = 0;
    tas.push(Reverse((0u32, s)));
    while let Some(Reverse((d, x))) = tas.pop() {
        if d != dist[x as usize] {
            continue;
        }
        let (deb, arcs) = if avant { (&c.out_deb, &c.out_arc) } else { (&c.in_deb, &c.in_arc) };
        for k in deb[x as usize]..deb[x as usize + 1] {
            let a = arcs[k as usize];
            let wa = w[a as usize];
            if wa >= INF {
                continue;
            }
            let y = if avant { c.tete[a as usize] } else { c.queue[a as usize] };
            if d + wa < dist[y as usize] {
                dist[y as usize] = d + wa;
                tas.push(Reverse((d + wa, y)));
            }
        }
    }
    dist
}

/// Repères de ALT : distances depuis et vers chaque repère.
pub struct Reperes {
    pub noeuds: Vec<u32>,
    pub depuis: Vec<Vec<u32>>, // d(L, v)
    pub vers: Vec<Vec<u32>>,   // d(v, L)
}

impl Reperes {
    /// Repères choisis par la méthode du plus éloigné, à partir d'un nœud
    /// de départ fixe ; distances calculées en parallèle.
    pub fn choisir(c: &Csr, w: &[u32], depart: u32, k: usize) -> Reperes {
        use rayon::prelude::*;
        let argmax = |d: &[u32]| {
            let mut best = (0u32, depart);
            for (v, &x) in d.iter().enumerate() {
                if x < INF && x > best.0 {
                    best = (x, v as u32);
                }
            }
            best.1
        };
        let mut noeuds = Vec::new();
        let mut cur = argmax(&tous(c, w, depart, true));
        let mut min_d = vec![INF; c.n];
        for _ in 0..k {
            noeuds.push(cur);
            let d = tous(c, w, cur, true);
            for v in 0..c.n {
                min_d[v] = min_d[v].min(d[v]);
            }
            cur = argmax(&min_d);
        }
        let depuis: Vec<Vec<u32>> = noeuds.par_iter().map(|&l| tous(c, w, l, true)).collect();
        let vers: Vec<Vec<u32>> = noeuds.par_iter().map(|&l| tous(c, w, l, false)).collect();
        Reperes { noeuds, depuis, vers }
    }

    #[inline]
    pub fn h(&self, v: u32, t: u32) -> u32 {
        let (v, t) = (v as usize, t as usize);
        let mut h = 0u32;
        for i in 0..self.noeuds.len() {
            // d(v,t) ≥ d(v,L) − d(t,L) et d(v,t) ≥ d(L,t) − d(L,v).
            let (vl, tl) = (self.vers[i][v], self.vers[i][t]);
            if vl < INF && tl < INF && vl > tl {
                h = h.max(vl - tl);
            }
            let (lt, lv) = (self.depuis[i][t], self.depuis[i][v]);
            if lt < INF && lv < INF && lt > lv {
                h = h.max(lt - lv);
            }
        }
        h
    }

    pub fn octets(&self) -> usize {
        self.noeuds.len() * 2 * self.depuis.first().map_or(0, |d| d.len()) * 4
    }
}

/// Plus grande composante fortement connexe pour une métrique : nœuds
/// atteignables depuis `racine` et qui l'atteignent.
pub fn cfc_de(c: &Csr, w: &[u32], racine: u32) -> Vec<bool> {
    let a = tous(c, w, racine, true);
    let b = tous(c, w, racine, false);
    (0..c.n).map(|v| a[v] < INF && b[v] < INF).collect()
}
