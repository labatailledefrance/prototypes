//! Construction du graphe routier à partir des morceaux des tiles.
//!
//! 1. **Recousure** : les morceaux d'un même tronçon (même identifiant BD
//!    TOPO) sont triés par rang et mis bout à bout quand les rangs se
//!    suivent et que les extrémités coïncident (entiers au 1/16 m). Un trou
//!    dans les rangs (partie du tronçon hors des tiles lues) laisse deux
//!    fragments. La même fonction recoud des morceaux, des fragments de
//!    tiles ou des fragments de départements : f(A ∪ B) = f(f(A) ∪ f(B)).
//!    Analogie : le réassemblage IP des fragments par (identifiant, décalage).
//! 2. **Nœuds** : la BD TOPO 3 n'a pas de table de nœuds ; les tronçons sont
//!    coupés aux intersections, et deux tronçons qui se touchent partagent
//!    une extrémité. Un nœud est donc une extrémité de fragment, identifiée
//!    par ses coordonnées entières (x, y). Les points de coupe aux bords des
//!    tiles ne deviennent jamais des nœuds (ils sont recousus avant).
//! 3. **Numérotation canonique** : nœuds triés par (y, x), arêtes par
//!    (identifiant, rang de début) : le graphe ne dépend ni de l'ordre de
//!    lecture des tiles ni du nombre de threads.

use crate::tuiles::{Attr, Morceau, Pont, QUANT, TileId};
use std::collections::BTreeMap;

/// Fragment recousu d'un tronçon : rangs r0..=r1, points globaux.
#[derive(Clone, Debug)]
pub struct Fragment {
    pub id: u64,
    pub r0: u16,
    pub r1: u16,
    pub attr: Attr,
    pub pts: Vec<[i64; 3]>,
    /// Morceaux d'origine (tile, rang) : pour suivre l'itinéraire en tile
    /// active (positions locales de la tile).
    pub morceaux: Vec<(TileId, u16)>,
}

#[derive(Default, Debug, Clone)]
pub struct StatsRecousure {
    pub entree: usize,
    pub sortie: usize,
    pub jointures: usize,
    pub doublons: usize,       // même (id, rang) deux fois
    pub ruptures: usize,       // rangs consécutifs mais extrémités différentes
    pub trous: usize,          // rangs non consécutifs (partie hors des tiles)
    pub attr_differents: usize,
}

pub fn de_morceaux(ms: Vec<Morceau>) -> Vec<Fragment> {
    ms.into_iter()
        .map(|m| Fragment { id: m.id, r0: m.rang, r1: m.rang, attr: m.attr, morceaux: vec![(m.tile, m.rang)], pts: m.pts })
        .collect()
}

/// Recoud des fragments : résultat canonique (trié par identifiant, rang).
pub fn recoudre(mut fs: Vec<Fragment>) -> (Vec<Fragment>, StatsRecousure) {
    let mut st = StatsRecousure { entree: fs.len(), ..Default::default() };
    fs.sort_by(|a, b| (a.id, a.r0, &a.pts).cmp(&(b.id, b.r0, &b.pts)));
    let mut out: Vec<Fragment> = Vec::with_capacity(fs.len());
    for f in fs {
        if let Some(p) = out.last_mut() {
            if p.id == f.id {
                if f.r0 <= p.r1 {
                    st.doublons += 1;
                } else if f.r0 == p.r1 + 1 {
                    if p.pts.last().map(|q| [q[0], q[1]]) == f.pts.first().map(|q| [q[0], q[1]]) {
                        if p.attr != f.attr {
                            st.attr_differents += 1;
                        }
                        st.jointures += 1;
                        p.r1 = f.r1;
                        p.pts.extend_from_slice(&f.pts[1..]);
                        p.morceaux.extend_from_slice(&f.morceaux);
                        continue;
                    }
                    st.ruptures += 1;
                } else {
                    st.trous += 1;
                }
            }
        }
        out.push(f);
    }
    st.sortie = out.len();
    (out, st)
}

pub fn longueur_m(pts: &[[i64; 3]]) -> f64 {
    let mut l = 0.0;
    for s in pts.windows(2) {
        let (dx, dy) = ((s[1][0] - s[0][0]) as f64, (s[1][1] - s[0][1]) as f64);
        l += (dx * dx + dy * dy).sqrt();
    }
    l / QUANT as f64
}

/// Natures de la BD TOPO (ordre de P-07).
pub const NATURES: &[&str] = &[
    "Type autoroutier", "Route à 2 chaussées", "Route à 1 chaussée", "Bretelle", "Rond-point", "Route empierrée",
    "Chemin", "Sentier", "Escalier", "Bac ou liaison maritime", "Piste cyclable", "Bac ou liaison fluviale",
];

/// Réseau carrossable : tout ce qu'une classe de véhicule peut emprunter
/// (sentiers, escaliers, pistes cyclables et bacs exclus de la topologie).
pub fn carrossable(a: &Attr) -> bool {
    a.nature <= 6
}

/// Ouvrage porté par une arête : pont de la BD TOPO ou ouvrage implicite
/// (tronçon au-dessus du sol sans pont associé).
#[derive(Clone, Debug)]
pub struct Ouvrage {
    pub pont: Option<u64>, // identifiant BD TOPO du pont
    pub charge_t: u16,     // charge limite (heuristique B-03), tonnes
    pub largeur_dm: u16,
}

pub struct Graphe {
    /// Coordonnées des nœuds (1/16 m, Lambert-93).
    pub xy: Vec<[i64; 2]>,
    /// Arêtes : extrémités (sens de numérisation u → v).
    pub u: Vec<u32>,
    pub v: Vec<u32>,
    pub attr: Vec<Attr>,
    pub long_mm: Vec<u32>,
    pub id: Vec<u64>,
    pub r0: Vec<u16>,
    pub ouvrage: Vec<Option<u32>>, // indice dans `ouvrages`
    pub ouvrages: Vec<Ouvrage>,
    pub frag: Vec<u32>, // indice du fragment (géométrie)
}

impl Graphe {
    pub fn n(&self) -> usize {
        self.xy.len()
    }
    pub fn m(&self) -> usize {
        self.u.len()
    }
}

/// Graphe à partir des fragments retenus par `garder`. Les boucles (u = v)
/// restent des arêtes : elles ne servent jamais à un plus court chemin.
pub fn construire(fs: &[Fragment], garder: impl Fn(&Fragment) -> bool) -> Graphe {
    let sel: Vec<u32> = (0..fs.len() as u32).filter(|&k| garder(&fs[k as usize])).collect();
    let mut cles: Vec<[i64; 2]> = Vec::with_capacity(sel.len() * 2);
    for &k in &sel {
        let p = &fs[k as usize].pts;
        cles.push([p[0][1], p[0][0]]);
        cles.push([p[p.len() - 1][1], p[p.len() - 1][0]]);
    }
    cles.sort_unstable();
    cles.dedup();
    let idx = |p: &[i64; 3]| cles.binary_search(&[p[1], p[0]]).unwrap() as u32;
    let mut g = Graphe {
        xy: cles.iter().map(|c| [c[1], c[0]]).collect(),
        u: Vec::new(),
        v: Vec::new(),
        attr: Vec::new(),
        long_mm: Vec::new(),
        id: Vec::new(),
        r0: Vec::new(),
        ouvrage: Vec::new(),
        ouvrages: Vec::new(),
        frag: Vec::new(),
    };
    for &k in &sel {
        let f = &fs[k as usize];
        g.u.push(idx(&f.pts[0]));
        g.v.push(idx(&f.pts[f.pts.len() - 1]));
        g.attr.push(f.attr);
        g.long_mm.push((longueur_m(&f.pts) * 1000.0) as u32);
        g.id.push(f.id);
        g.r0.push(f.r0);
        g.ouvrage.push(None);
        g.frag.push(k);
    }
    g
}

/// Empreinte canonique du graphe (BLAKE3).
pub fn empreinte(g: &Graphe) -> String {
    let mut h = blake3::Hasher::new();
    for p in &g.xy {
        h.update(&p[0].to_le_bytes());
        h.update(&p[1].to_le_bytes());
    }
    for e in 0..g.m() {
        h.update(&g.u[e].to_le_bytes());
        h.update(&g.v[e].to_le_bytes());
        h.update(&g.id[e].to_le_bytes());
        h.update(&g.r0[e].to_le_bytes());
        h.update(&g.long_mm[e].to_le_bytes());
        let o = g.ouvrage[e].map(|o| g.ouvrages[o as usize].charge_t).unwrap_or(0);
        h.update(&o.to_le_bytes());
    }
    h.finalize().to_hex()[..16].to_string()
}

pub fn empreinte_fragments(fs: &[Fragment]) -> String {
    let mut h = blake3::Hasher::new();
    for f in fs {
        h.update(&f.id.to_le_bytes());
        h.update(&f.r0.to_le_bytes());
        h.update(&f.r1.to_le_bytes());
        for p in &f.pts {
            h.update(&p[0].to_le_bytes());
            h.update(&p[1].to_le_bytes());
            h.update(&p[2].to_le_bytes());
        }
    }
    h.finalize().to_hex()[..16].to_string()
}

/// Distance d'un point à une polyligne, en 1/16 m.
fn dist_pt_ligne(p: [i64; 2], l: &[[i64; 3]]) -> f64 {
    let mut best = f64::MAX;
    let (px, py) = (p[0] as f64, p[1] as f64);
    for s in l.windows(2) {
        let (ax, ay, bx, by) = (s[0][0] as f64, s[0][1] as f64, s[1][0] as f64, s[1][1] as f64);
        let (dx, dy) = (bx - ax, by - ay);
        let d2 = dx * dx + dy * dy;
        let t = if d2 == 0.0 { 0.0 } else { (((px - ax) * dx + (py - ay) * dy) / d2).clamp(0.0, 1.0) };
        let (qx, qy) = (ax + t * dx - px, ay + t * dy - py);
        best = best.min((qx * qx + qy * qy).sqrt());
    }
    if l.len() == 1 {
        let (qx, qy) = (l[0][0] as f64 - px, l[0][1] as f64 - py);
        best = (qx * qx + qy * qy).sqrt();
    }
    best
}

#[derive(Default, Debug)]
pub struct StatsPonts {
    pub ponts: usize,
    pub ponts_associes: usize,
    pub aretes_sur_ouvrage: usize,
    pub aretes_pont: usize,
    pub aretes_implicites: usize,
}

/// Associe les ponts aux arêtes au-dessus du sol (règle de P-07 : sommet du
/// pont à moins de 5 m du tronçon), puis calcule la charge de chaque ouvrage.
/// Une arête portée par plusieurs ponts prend la plus faible charge.
pub fn associer_ponts(g: &mut Graphe, fs: &[Fragment], ponts: &[Pont], charge: impl Fn(Option<&Pont>, &Attr, u32) -> u16) -> StatsPonts {
    let mut st = StatsPonts { ponts: ponts.len(), ..Default::default() };
    // Index des arêtes sur ouvrage par tile de leurs sommets.
    let mut par_tile: BTreeMap<TileId, Vec<u32>> = BTreeMap::new();
    for e in 0..g.m() {
        if g.attr[e].pos_sol >= 1 {
            st.aretes_sur_ouvrage += 1;
            let mut ts: Vec<TileId> = fs[g.frag[e] as usize].pts.iter().map(|p| TileId::de_q(p[0], p[1])).collect();
            ts.sort();
            ts.dedup();
            for t in ts {
                par_tile.entry(t).or_default().push(e as u32);
            }
        }
    }
    let seuil = 5.0 * QUANT as f64;
    let mut porte: Vec<Vec<u32>> = vec![Vec::new(); g.m()];
    for (k, p) in ponts.iter().enumerate() {
        let mut cand: Vec<u32> = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(v) = par_tile.get(&TileId { ix: p.tile.ix + dx, iy: p.tile.iy + dy }) {
                    cand.extend(v);
                }
            }
        }
        cand.sort_unstable();
        cand.dedup();
        let mut ok = false;
        for e in cand {
            let l = &fs[g.frag[e as usize] as usize].pts;
            if p.pts.iter().any(|q| dist_pt_ligne(*q, l) <= seuil) {
                porte[e as usize].push(k as u32);
                ok = true;
            }
        }
        if ok {
            st.ponts_associes += 1;
        }
    }
    for e in 0..g.m() {
        if g.attr[e].pos_sol < 1 {
            continue;
        }
        let a = g.attr[e];
        let o = if porte[e].is_empty() {
            st.aretes_implicites += 1;
            Ouvrage { pont: None, charge_t: charge(None, &a, g.long_mm[e]), largeur_dm: a.largeur_dm }
        } else {
            st.aretes_pont += 1;
            let mut best: Option<Ouvrage> = None;
            for &k in &porte[e] {
                let p = &ponts[k as usize];
                let c = charge(Some(p), &a, g.long_mm[e]);
                let lr = if p.largeur_dm > 0 { p.largeur_dm } else { a.largeur_dm };
                if best.as_ref().is_none_or(|b| (c, p.id) < (b.charge_t, b.pont.unwrap())) {
                    best = Some(Ouvrage { pont: Some(p.id), charge_t: c, largeur_dm: lr });
                }
            }
            best.unwrap()
        };
        g.ouvrage[e] = Some(g.ouvrages.len() as u32);
        g.ouvrages.push(o);
    }
    st
}

/// Composantes faiblement connexes (union-find à racine minimale).
pub fn composantes(g: &Graphe) -> Vec<u32> {
    let mut p: Vec<u32> = (0..g.n() as u32).collect();
    fn racine(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    for e in 0..g.m() {
        let (a, b) = (racine(&mut p, g.u[e]), racine(&mut p, g.v[e]));
        if a != b {
            let (lo, hi) = (a.min(b), a.max(b));
            p[hi as usize] = lo;
        }
    }
    (0..g.n() as u32).map(|x| racine(&mut p, x)).collect()
}
