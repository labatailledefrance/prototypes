//! Index spatiaux des humains d'une tile, pour la perception des zombies.
//!
//! Une requête de perception demande :
//! - l'**ouïe** : l'humain le plus proche à moins de `ouie` mètres, dans
//!   toutes les directions (à travers les murs) ;
//! - la **vue** : les `k` humains les plus proches dans le cône de vue, entre
//!   `ouie` et `vue` mètres, candidats au tracé de ligne de vue.
//!
//! Toutes les structures rendent exactement le même résultat : les égalités
//! de distance sont départagées par l'identifiant de l'humain. On peut donc
//! comparer leurs sommes de contrôle (voir les tests).
//!
//! Analogie : quatre façons d'indexer la même table pour une requête
//! « WHERE distance < r ORDER BY distance LIMIT k ». La grille de P-01 est un
//! parcours de plage sans tri ; les anneaux et l'arbre k-d lisent dans l'ordre
//! des distances et s'arrêtent dès que la suite ne peut plus améliorer le
//! résultat, comme un index trié avec LIMIT.

use std::mem::MaybeUninit;

use clap::ValueEnum;

use crate::ville::COTE_F;

/// Nombre maximal de candidats à la ligne de vue.
pub const RAYONS_MAX: usize = 8;
/// Marge des boîtes (m) : un point posé sur une frontière de case, à
/// l'arrondi près, n'est jamais élagué à tort.
const EPS: f32 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Structure {
    /// Grille uniforme de P-01 : toutes les cases du carré englobant la vue,
    /// en un seul passage.
    Grille,
    /// Grille uniforme parcourue par anneaux de distance croissante : ouïe
    /// d'abord (si un humain est entendu, la vue est inutile), puis vue ;
    /// élagage des cases hors du disque et du cône, arrêt anticipé.
    Anneaux,
    /// Deux niveaux : cases grossières (8 × la case fine) qui portent le
    /// nombre d'humains, puis cases fines. Mêmes règles que les anneaux.
    Hierarchique,
    /// Arbre k-d (feuilles de 8 humains, boîtes englobantes ajustées : une
    /// BVH de points), parcouru du plus proche au plus loin.
    Kdtree,
}

#[derive(Clone, Copy, Default)]
pub struct Pt {
    pub p: [f32; 2],
    pub id: u32,
}

#[derive(Clone, Copy)]
pub struct Cand {
    pub dd: f32,
    pub id: u32,
    pub p: [f32; 2],
}

const VIDE: Cand = Cand {
    dd: f32::INFINITY,
    id: u32::MAX,
    p: [0.0; 2],
};

/// Ordre total des candidats : distance, puis identifiant.
#[inline(always)]
fn avant(dd: f32, id: u32, c: &Cand) -> bool {
    dd < c.dd || (dd == c.dd && id < c.id)
}

pub struct Requete {
    pub pos: [f32; 2],
    pub dir: [f32; 2],
    pub ouie2: f32,
    pub vue2: f32,
    pub cos_demi: f32,
    pub sin_demi: f32,
    pub k: usize,
    /// Faux si la carte de proximité garantit qu'aucun humain n'est à portée
    /// d'ouïe : la phase d'ouïe est sautée (rejet rapide).
    pub ouie_possible: bool,
}

/// Travail effectué, pour expliquer les temps.
#[derive(Default, Clone, Copy)]
pub struct Compte {
    /// Cases parcourues, vides comprises (nœuds pour l'arbre k-d).
    pub cases: u64,
    /// Humains dont la distance a été calculée.
    pub candidats: u64,
}

pub struct Collecte {
    pub entendu: Option<Cand>,
    pub vus: [Cand; RAYONS_MAX],
    pub nb: usize,
}

impl Collecte {
    fn new() -> Self {
        Collecte {
            entendu: None,
            vus: [VIDE; RAYONS_MAX],
            nb: 0,
        }
    }

    #[inline(always)]
    fn ouie(&mut self, q: &Requete, e: &Pt, dd: f32) {
        if dd <= q.ouie2 && self.entendu.is_none_or(|b| avant(dd, e.id, &b)) {
            self.entendu = Some(Cand {
                dd,
                id: e.id,
                p: e.p,
            });
        }
    }

    #[inline(always)]
    fn vue(&mut self, q: &Requete, e: &Pt, d: [f32; 2], dd: f32) {
        if dd > q.ouie2
            && dd <= q.vue2
            && q.k > 0
            && d[0] * q.dir[0] + d[1] * q.dir[1] >= q.cos_demi * dd.sqrt()
            && (self.nb < q.k || avant(dd, e.id, &self.vus[self.nb - 1]))
        {
            // Insertion dans les k plus proches (tableau trié).
            let mut i = self.nb.min(q.k - 1);
            while i > 0 && avant(dd, e.id, &self.vus[i - 1]) {
                self.vus[i] = self.vus[i - 1];
                i -= 1;
            }
            self.vus[i] = Cand {
                dd,
                id: e.id,
                p: e.p,
            };
            self.nb = (self.nb + 1).min(q.k);
        }
    }

    /// Distance² au-delà de laquelle aucun humain ne peut plus être entendu
    /// plus près que l'actuel.
    #[inline(always)]
    fn borne_ouie(&self, q: &Requete) -> f32 {
        self.entendu.map_or(q.ouie2, |c| c.dd)
    }

    /// Idem pour la vue : distance du k-ième candidat, ou distance de vue.
    #[inline(always)]
    fn borne_vue(&self, q: &Requete) -> f32 {
        if q.k > 0 && self.nb == q.k {
            self.vus[q.k - 1].dd
        } else {
            q.vue2
        }
    }
}

#[inline(always)]
fn delta(q: &Requete, e: &Pt) -> ([f32; 2], f32) {
    let d = [e.p[0] - q.pos[0], e.p[1] - q.pos[1]];
    (d, d[0] * d[0] + d[1] * d[1])
}

/// Distance² minimale de `p` à la boîte [mn, mx] (élargie de EPS).
#[inline(always)]
fn d2_boite(p: [f32; 2], mn: [f32; 2], mx: [f32; 2]) -> f32 {
    let dx = (mn[0] - EPS - p[0]).max(p[0] - mx[0] - EPS).max(0.0);
    let dy = (mn[1] - EPS - p[1]).max(p[1] - mx[1] - EPS).max(0.0);
    dx * dx + dy * dy
}

/// La boîte peut-elle toucher le cône de vue ? Test conservateur : on
/// élargit le demi-angle du cône de l'angle sous lequel on voit le cercle
/// circonscrit à la boîte.
#[inline(always)]
fn cone_boite(q: &Requete, mn: [f32; 2], mx: [f32; 2]) -> bool {
    let c = [(mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5];
    let h = [(mx[0] - mn[0]) * 0.5 + EPS, (mx[1] - mn[1]) * 0.5 + EPS];
    let rho2 = (h[0] * h[0] + h[1] * h[1]) * 1.0001;
    let v = [c[0] - q.pos[0], c[1] - q.pos[1]];
    let l2 = v[0] * v[0] + v[1] * v[1];
    if l2 <= rho2 {
        return true;
    }
    let l = l2.sqrt();
    let sinb = (rho2 / l2).sqrt();
    let cosb = (1.0 - sinb * sinb).max(0.0).sqrt();
    // cos(α + β), avec α le demi-angle du cône et β l'angle du cercle.
    let cos_ab = q.cos_demi * cosb - q.sin_demi * sinb;
    v[0] * q.dir[0] + v[1] * q.dir[1] >= cos_ab * l
}

// ------------------------------------------------------------ Grille uniforme

/// Grille uniforme reconstruite par tri par dénombrement (comme P-01), avec
/// des éléments compacts (12 octets).
pub struct GrilleH {
    pub cote: usize,
    pub taille: f32,
    inv: f32,
    debut: Vec<u32>,
    curseur: Vec<u32>,
    pts: Vec<Pt>,
}

impl GrilleH {
    pub fn new(taille: f32) -> Self {
        let cote = (COTE_F / taille).ceil() as usize;
        GrilleH {
            cote,
            taille,
            inv: 1.0 / taille,
            debut: vec![0; cote * cote + 1],
            curseur: vec![0; cote * cote],
            pts: Vec::new(),
        }
    }

    #[inline(always)]
    pub fn coord(&self, p: [f32; 2]) -> (i32, i32) {
        let m = self.cote as i32 - 1;
        (
            ((p[0] * self.inv) as i32).clamp(0, m),
            ((p[1] * self.inv) as i32).clamp(0, m),
        )
    }

    pub fn reconstruire(&mut self, pts: &[[f32; 2]]) {
        self.debut.fill(0);
        for &p in pts {
            let (x, y) = self.coord(p);
            self.debut[y as usize * self.cote + x as usize + 1] += 1;
        }
        for c in 1..self.debut.len() {
            self.debut[c] += self.debut[c - 1];
        }
        self.curseur
            .copy_from_slice(&self.debut[..self.debut.len() - 1]);
        self.pts.resize(pts.len(), Pt::default());
        for (i, &p) in pts.iter().enumerate() {
            let (x, y) = self.coord(p);
            let c = &mut self.curseur[y as usize * self.cote + x as usize];
            self.pts[*c as usize] = Pt { p, id: i as u32 };
            *c += 1;
        }
    }

    #[inline(always)]
    fn case(&self, x: i32, y: i32) -> &[Pt] {
        let c = y as usize * self.cote + x as usize;
        &self.pts[self.debut[c] as usize..self.debut[c + 1] as usize]
    }

    #[inline(always)]
    fn boite(&self, x: i32, y: i32) -> ([f32; 2], [f32; 2]) {
        let s = self.taille;
        (
            [x as f32 * s, y as f32 * s],
            [(x + 1) as f32 * s, (y + 1) as f32 * s],
        )
    }

    /// P-01 : toutes les cases du carré de côté 2 × vue, un seul passage.
    fn chercher_naif(&self, q: &Requete, c: &mut Compte) -> Collecte {
        let mut r = Collecte::new();
        let rc = (q.vue2.sqrt() / self.taille).ceil() as i32;
        let (cx, cy) = self.coord(q.pos);
        let m = self.cote as i32 - 1;
        for y in (cy - rc).max(0)..=(cy + rc).min(m) {
            for x in (cx - rc).max(0)..=(cx + rc).min(m) {
                c.cases += 1;
                for e in self.case(x, y) {
                    c.candidats += 1;
                    let (d, dd) = delta(q, e);
                    if dd <= q.ouie2 {
                        r.ouie(q, e, dd);
                    } else {
                        r.vue(q, e, d, dd);
                    }
                }
            }
        }
        r
    }

    /// Parcours par anneaux de Tchebychev autour de la case du zombie. Un
    /// anneau dont la distance minimale dépasse la borne courante arrête le
    /// parcours ; une case hors borne (ou hors cône) est sautée.
    #[inline(always)]
    fn anneaux(
        &self,
        q: &Requete,
        c: &mut Compte,
        mut visiter: impl FnMut(i32, i32, &mut Compte) -> f32,
        borne0: f32,
    ) {
        let (cx, cy) = self.coord(q.pos);
        let m = self.cote as i32 - 1;
        let rmax = cx.max(m - cx).max(cy).max(m - cy);
        let s = self.taille;
        let mut borne = borne0;
        for r in 0..=rmax {
            if r > 0 {
                // Distance de pos au bord du carré des anneaux 0..r-1 :
                // tout point de l'anneau r est au moins aussi loin.
                let (x0, x1) = ((cx - r + 1) as f32 * s, (cx + r) as f32 * s);
                let (y0, y1) = ((cy - r + 1) as f32 * s, (cy + r) as f32 * s);
                let marge = (q.pos[0] - x0)
                    .min(x1 - q.pos[0])
                    .min(q.pos[1] - y0)
                    .min(y1 - q.pos[1])
                    - EPS;
                if marge > 0.0 && marge * marge > borne {
                    return;
                }
            }
            let mut une = |x: i32, y: i32, c: &mut Compte, borne: &mut f32| {
                if x < 0 || y < 0 || x > m || y > m {
                    return;
                }
                let (mn, mx) = self.boite(x, y);
                if d2_boite(q.pos, mn, mx) > *borne {
                    return;
                }
                c.cases += 1;
                *borne = visiter(x, y, c);
            };
            if r == 0 {
                une(cx, cy, c, &mut borne);
                continue;
            }
            for x in cx - r..=cx + r {
                une(x, cy - r, c, &mut borne);
                une(x, cy + r, c, &mut borne);
            }
            for y in cy - r + 1..=cy + r - 1 {
                une(cx - r, y, c, &mut borne);
                une(cx + r, y, c, &mut borne);
            }
        }
    }

    fn chercher_anneaux(&self, q: &Requete, c: &mut Compte) -> Collecte {
        let mut r = Collecte::new();
        // 1. Ouïe : le plus proche à moins de `ouie`.
        if q.ouie_possible {
            self.anneaux(
                q,
                c,
                |x, y, c| {
                    for e in self.case(x, y) {
                        c.candidats += 1;
                        let (_, dd) = delta(q, e);
                        r.ouie(q, e, dd);
                    }
                    r.borne_ouie(q)
                },
                q.ouie2,
            );
        }
        // Un humain entendu l'emporte toujours sur un humain vu (plus loin).
        if r.entendu.is_some() || q.k == 0 {
            return r;
        }
        // 2. Vue : les k plus proches dans le cône.
        self.anneaux(
            q,
            c,
            |x, y, c| {
                let pts = self.case(x, y);
                if !pts.is_empty() {
                    let (mn, mx) = self.boite(x, y);
                    if cone_boite(q, mn, mx) {
                        for e in pts {
                            c.candidats += 1;
                            let (d, dd) = delta(q, e);
                            r.vue(q, e, d, dd);
                        }
                    }
                }
                r.borne_vue(q)
            },
            q.vue2,
        );
        r
    }
}

// ---------------------------------------------------------- Grille à 2 niveaux

/// Cases fines par côté de case grossière.
const FACTEUR: usize = 8;

pub struct Hier {
    fine: GrilleH,
    cote_g: usize,
    taille_g: f32,
    /// Nombre d'humains par case grossière.
    compte: Vec<u32>,
}

impl Hier {
    pub fn new(taille: f32) -> Self {
        let fine = GrilleH::new(taille);
        let cote_g = fine.cote.div_ceil(FACTEUR);
        Hier {
            fine,
            cote_g,
            taille_g: taille * FACTEUR as f32,
            compte: vec![0; cote_g * cote_g],
        }
    }

    pub fn reconstruire(&mut self, pts: &[[f32; 2]]) {
        self.fine.reconstruire(pts);
        self.compte.fill(0);
        for &p in pts {
            let (x, y) = self.fine.coord(p);
            let (gx, gy) = (x as usize / FACTEUR, y as usize / FACTEUR);
            self.compte[gy * self.cote_g + gx] += 1;
        }
    }

    /// Cases grossières non vides à moins de `borne`, triées par distance,
    /// puis cases fines de chacune, avec arrêt dès qu'une case grossière est
    /// plus loin que la borne courante.
    fn passe(&self, q: &Requete, c: &mut Compte, vue: bool, r: &mut Collecte) {
        let borne0 = if vue { q.vue2 } else { q.ouie2 };
        let rayon = borne0.sqrt();
        let sg = self.taille_g;
        let mg = self.cote_g as i32 - 1;
        let gx0 = (((q.pos[0] - rayon) / sg) as i32).clamp(0, mg);
        let gx1 = (((q.pos[0] + rayon) / sg) as i32).clamp(0, mg);
        let gy0 = (((q.pos[1] - rayon) / sg) as i32).clamp(0, mg);
        let gy1 = (((q.pos[1] + rayon) / sg) as i32).clamp(0, mg);
        // Au plus (2 × 200 / 100 + 2)² = 36 cases grossières pour 200 m.
        let mut liste: [MaybeUninit<(f32, i32, i32)>; 64] = [const { MaybeUninit::uninit() }; 64];
        let mut n = 0usize;
        for gy in gy0..=gy1 {
            for gx in gx0..=gx1 {
                c.cases += 1;
                if self.compte[gy as usize * self.cote_g + gx as usize] == 0 {
                    continue;
                }
                let mn = [gx as f32 * sg, gy as f32 * sg];
                let mx = [(gx + 1) as f32 * sg, (gy + 1) as f32 * sg];
                let dd = d2_boite(q.pos, mn, mx);
                if dd > borne0 || (vue && !cone_boite(q, mn, mx)) || n == liste.len() {
                    continue;
                }
                // Insertion triée (peu d'éléments).
                // SAFETY : les entrées 0..n ont toutes été écrites.
                let mut i = n;
                while i > 0 && unsafe { liste[i - 1].assume_init() }.0 > dd {
                    liste[i] = liste[i - 1];
                    i -= 1;
                }
                liste[i].write((dd, gx, gy));
                n += 1;
            }
        }
        let m = self.fine.cote as i32 - 1;
        let f = FACTEUR as i32;
        for e in &liste[..n] {
            // SAFETY : idem.
            let (dd, gx, gy) = unsafe { e.assume_init() };
            let borne = if vue { r.borne_vue(q) } else { r.borne_ouie(q) };
            if dd > borne {
                break;
            }
            for y in gy * f..((gy + 1) * f).min(m + 1) {
                for x in gx * f..((gx + 1) * f).min(m + 1) {
                    c.cases += 1;
                    let pts = self.fine.case(x, y);
                    if pts.is_empty() {
                        continue;
                    }
                    let borne = if vue { r.borne_vue(q) } else { r.borne_ouie(q) };
                    let (mn, mx) = self.fine.boite(x, y);
                    if d2_boite(q.pos, mn, mx) > borne || (vue && !cone_boite(q, mn, mx)) {
                        continue;
                    }
                    for e in pts {
                        c.candidats += 1;
                        let (d, dd) = delta(q, e);
                        if vue {
                            r.vue(q, e, d, dd);
                        } else {
                            r.ouie(q, e, dd);
                        }
                    }
                }
            }
        }
    }

    fn chercher(&self, q: &Requete, c: &mut Compte) -> Collecte {
        let mut r = Collecte::new();
        if q.ouie_possible {
            self.passe(q, c, false, &mut r);
        }
        if r.entendu.is_none() && q.k > 0 {
            self.passe(q, c, true, &mut r);
        }
        r
    }
}

// ------------------------------------------------------------------ Arbre k-d

const FEUILLE: usize = 8;

#[derive(Clone, Copy)]
struct Noeud {
    mn: [f32; 2],
    mx: [f32; 2],
    debut: u32,
    fin: u32,
    /// Indice du premier enfant (le second suit) ; 0 = feuille.
    enfant: u32,
}

pub struct Kd {
    noeuds: Vec<Noeud>,
    pts: Vec<Pt>,
}

impl Kd {
    pub fn new() -> Self {
        Kd {
            noeuds: Vec::new(),
            pts: Vec::new(),
        }
    }

    pub fn reconstruire(&mut self, pts: &[[f32; 2]]) {
        self.pts.clear();
        self.pts
            .extend(pts.iter().enumerate().map(|(i, &p)| Pt { p, id: i as u32 }));
        self.noeuds.clear();
        self.noeuds.push(Noeud {
            mn: [0.0; 2],
            mx: [0.0; 2],
            debut: 0,
            fin: pts.len() as u32,
            enfant: 0,
        });
        // Construction en largeur, sans récursion.
        let mut i = 0;
        while i < self.noeuds.len() {
            let (debut, fin) = (self.noeuds[i].debut as usize, self.noeuds[i].fin as usize);
            let mut mn = [f32::INFINITY; 2];
            let mut mx = [f32::NEG_INFINITY; 2];
            for e in &self.pts[debut..fin] {
                for a in 0..2 {
                    mn[a] = mn[a].min(e.p[a]);
                    mx[a] = mx[a].max(e.p[a]);
                }
            }
            self.noeuds[i].mn = mn;
            self.noeuds[i].mx = mx;
            if fin - debut > FEUILLE {
                // Coupe à la médiane sur l'axe le plus étendu ; ordre total
                // (coordonnée, puis identifiant) : construction déterministe.
                let ax = if mx[0] - mn[0] >= mx[1] - mn[1] { 0 } else { 1 };
                let mil = (debut + fin) / 2;
                self.pts[debut..fin].select_nth_unstable_by(mil - debut, |a, b| {
                    a.p[ax].total_cmp(&b.p[ax]).then(a.id.cmp(&b.id))
                });
                let g = self.noeuds.len() as u32;
                self.noeuds[i].enfant = g;
                for (d, f) in [(debut, mil), (mil, fin)] {
                    self.noeuds.push(Noeud {
                        mn: [0.0; 2],
                        mx: [0.0; 2],
                        debut: d as u32,
                        fin: f as u32,
                        enfant: 0,
                    });
                }
            }
            i += 1;
        }
    }

    fn passe(&self, q: &Requete, c: &mut Compte, vue: bool, r: &mut Collecte) {
        if self.pts.is_empty() {
            return;
        }
        let borne = |r: &Collecte| if vue { r.borne_vue(q) } else { r.borne_ouie(q) };
        // Pile non initialisée : une pile de 64 entrées remise à zéro à
        // chaque requête coûtait autant que le parcours lui-même.
        let mut pile: [MaybeUninit<(u32, f32)>; 64] = [const { MaybeUninit::uninit() }; 64];
        let n0 = &self.noeuds[0];
        let d0 = d2_boite(q.pos, n0.mn, n0.mx);
        if d0 > borne(r) {
            return;
        }
        pile[0].write((0, d0));
        let mut h = 1usize;
        while h > 0 {
            h -= 1;
            // SAFETY : les entrées 0..h ont toutes été écrites.
            let (i, dd) = unsafe { pile[h].assume_init() };
            if dd > borne(r) {
                continue;
            }
            let n = &self.noeuds[i as usize];
            c.cases += 1;
            if n.enfant == 0 {
                for e in &self.pts[n.debut as usize..n.fin as usize] {
                    c.candidats += 1;
                    let (d, dd) = delta(q, e);
                    if vue {
                        r.vue(q, e, d, dd);
                    } else {
                        r.ouie(q, e, dd);
                    }
                }
                continue;
            }
            // Le plus proche des deux enfants est empilé en dernier, donc
            // visité en premier.
            let b = borne(r);
            let mut enf = [(0u32, f32::INFINITY); 2];
            let mut k = 0;
            for j in [n.enfant, n.enfant + 1] {
                let e = &self.noeuds[j as usize];
                let de = d2_boite(q.pos, e.mn, e.mx);
                if de <= b && (!vue || cone_boite(q, e.mn, e.mx)) {
                    enf[k] = (j, de);
                    k += 1;
                }
            }
            if k == 2 && enf[0].1 < enf[1].1 {
                enf.swap(0, 1);
            }
            for &x in &enf[..k] {
                pile[h].write(x);
                h += 1;
            }
        }
    }

    fn chercher(&self, q: &Requete, c: &mut Compte) -> Collecte {
        let mut r = Collecte::new();
        if q.ouie_possible {
            self.passe(q, c, false, &mut r);
        }
        if r.entendu.is_none() && q.k > 0 {
            self.passe(q, c, true, &mut r);
        }
        r
    }
}

// ---------------------------------------------------------------------- Index

pub enum Index {
    Grille(GrilleH),
    Anneaux(GrilleH),
    Hierarchique(Hier),
    Kdtree(Kd),
}

impl Index {
    /// `case` : côté d'une case (fine) en mètres ; ignoré par l'arbre k-d.
    pub fn new(s: Structure, case: f32) -> Self {
        match s {
            Structure::Grille => Index::Grille(GrilleH::new(case)),
            Structure::Anneaux => Index::Anneaux(GrilleH::new(case)),
            Structure::Hierarchique => Index::Hierarchique(Hier::new(case)),
            Structure::Kdtree => Index::Kdtree(Kd::new()),
        }
    }

    pub fn reconstruire(&mut self, pts: &[[f32; 2]]) {
        match self {
            Index::Grille(g) | Index::Anneaux(g) => g.reconstruire(pts),
            Index::Hierarchique(h) => h.reconstruire(pts),
            Index::Kdtree(k) => k.reconstruire(pts),
        }
    }

    #[inline(always)]
    pub fn chercher(&self, q: &Requete, c: &mut Compte) -> Collecte {
        match self {
            Index::Grille(g) => g.chercher_naif(q, c),
            Index::Anneaux(g) => g.chercher_anneaux(q, c),
            Index::Hierarchique(h) => h.chercher(q, c),
            Index::Kdtree(k) => k.chercher(q, c),
        }
    }
}

// ----------------------------------------------------------- Carte de proximité

/// Distance de Tchebychev, en cases de `taille`, de chaque case à la plus
/// proche case occupée par un humain. Donne une borne inférieure de la
/// distance euclidienne au plus proche humain, lue en O(1).
///
/// Sert au **rejet rapide** (aucun humain à portée : pas de requête) et à la
/// **veille profonde** (plus d'humain à quelques centaines de mètres).
/// Analogie : une table de routage « distance au préfixe le plus proche »,
/// recalculée à chaque tick par deux balayages (chamfer), comme un
/// Bellman-Ford sur une grille.
pub struct Proximite {
    /// P-04 : origine (m) du carré couvert, qui déborde de la tile (halo).
    origine: f32,
    cote: usize,
    taille: f32,
    inv: f32,
    cheb: Vec<u16>,
}

impl Proximite {
    #[allow(dead_code)]
    pub fn new(taille: f32) -> Self {
        Self::avec_etendue(taille, 0.0, COTE_F)
    }

    /// Carré [origine, origine + etendue[ (P-04 : tile plus halo).
    pub fn avec_etendue(taille: f32, origine: f32, etendue: f32) -> Self {
        let cote = (etendue / taille).ceil() as usize;
        Proximite {
            origine,
            cote,
            taille,
            inv: 1.0 / taille,
            cheb: vec![u16::MAX; cote * cote],
        }
    }

    #[inline(always)]
    fn coord(&self, p: [f32; 2]) -> (usize, usize) {
        let m = self.cote as i32 - 1;
        (
            (((p[0] - self.origine) * self.inv) as i32).clamp(0, m) as usize,
            (((p[1] - self.origine) * self.inv) as i32).clamp(0, m) as usize,
        )
    }

    pub fn reconstruire(&mut self, pts: &[[f32; 2]]) {
        let n = self.cote;
        self.cheb.fill(u16::MAX);
        for &p in pts {
            let (x, y) = self.coord(p);
            self.cheb[y * n + x] = 0;
        }
        let d = &mut self.cheb;
        let lire = |d: &Vec<u16>, x: isize, y: isize| -> u16 {
            if x < 0 || y < 0 || x >= n as isize || y >= n as isize {
                u16::MAX
            } else {
                d[y as usize * n + x as usize]
            }
        };
        // Balayage aller puis retour, 8 voisins de coût 1 : distance de
        // Tchebychev exacte.
        for y in 0..n as isize {
            for x in 0..n as isize {
                let m = [(-1, 0), (-1, -1), (0, -1), (1, -1)]
                    .iter()
                    .map(|&(dx, dy)| lire(d, x + dx, y + dy))
                    .min()
                    .unwrap();
                let k = y as usize * n + x as usize;
                d[k] = d[k].min(m.saturating_add(1));
            }
        }
        for y in (0..n as isize).rev() {
            for x in (0..n as isize).rev() {
                let m = [(1, 0), (1, 1), (0, 1), (-1, 1)]
                    .iter()
                    .map(|&(dx, dy)| lire(d, x + dx, y + dy))
                    .min()
                    .unwrap();
                let k = y as usize * n + x as usize;
                d[k] = d[k].min(m.saturating_add(1));
            }
        }
    }

    /// Borne inférieure (m) de la distance de `p` au plus proche humain.
    /// Deux cases à `c` cases de Tchebychev sont séparées d'au moins
    /// `(c − 1) × taille` sur un axe.
    #[inline(always)]
    pub fn distance_min(&self, p: [f32; 2]) -> f32 {
        let (x, y) = self.coord(p);
        match self.cheb[y * self.cote + x] {
            u16::MAX => f32::INFINITY,
            c => c.saturating_sub(1) as f32 * self.taille,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alea::Alea;

    fn nuage(a: &mut Alea, n: usize, groupe: bool) -> Vec<[f32; 2]> {
        (0..n)
            .map(|_| {
                if groupe {
                    [440.0 + a.f32() * 120.0, 440.0 + a.f32() * 120.0]
                } else {
                    [a.f32() * 999.9, a.f32() * 999.9]
                }
            })
            .collect()
    }

    fn signature(r: &Collecte) -> (Option<u32>, Vec<u32>) {
        (
            r.entendu.map(|c| c.id),
            r.vus[..r.nb].iter().map(|c| c.id).collect(),
        )
    }

    /// Toutes les structures rendent le même résultat que la grille de P-01,
    /// à toutes les tailles de case, sur des nuages groupés et dispersés.
    #[test]
    fn structures_equivalentes() {
        let mut a = Alea::new(7);
        for groupe in [false, true] {
            // Doublons volontaires : égalités de distance.
            let mut pts = nuage(&mut a, 1000, groupe);
            pts.extend_from_slice(&pts[..50].to_vec());
            let mut refe = Index::new(Structure::Grille, 25.0);
            refe.reconstruire(&pts);
            let mut autres: Vec<Index> = vec![];
            for s in [
                Structure::Grille,
                Structure::Anneaux,
                Structure::Hierarchique,
            ] {
                for case in [5.0, 10.0, 12.5, 25.0, 50.0, 100.0] {
                    autres.push(Index::new(s, case));
                }
            }
            autres.push(Index::new(Structure::Kdtree, 0.0));
            for x in &mut autres {
                x.reconstruire(&pts);
            }
            let demi = 60f32.to_radians();
            for _ in 0..3000 {
                let cap = a.f32() * std::f32::consts::TAU;
                let q = Requete {
                    pos: if groupe && a.f32() < 0.5 {
                        [400.0 + a.f32() * 200.0, 400.0 + a.f32() * 200.0]
                    } else {
                        [a.f32() * 999.0, a.f32() * 999.0]
                    },
                    dir: [cap.cos(), cap.sin()],
                    ouie2: 40.0 * 40.0,
                    vue2: 200.0 * 200.0,
                    cos_demi: demi.cos(),
                    sin_demi: demi.sin(),
                    k: 4,
                    ouie_possible: true,
                };
                let mut c = Compte::default();
                let attendu = signature(&refe.chercher(&q, &mut c));
                for x in &autres {
                    let r = x.chercher(&q, &mut c);
                    // Seul compte le résultat utile : entendu, sinon vus.
                    let s = signature(&r);
                    if attendu.0.is_some() {
                        assert_eq!(s.0, attendu.0);
                    } else {
                        assert_eq!(s, attendu);
                    }
                }
            }
        }
    }

    /// La carte de proximité est une borne inférieure de la distance réelle.
    #[test]
    fn proximite_borne_inferieure() {
        let mut a = Alea::new(3);
        let pts = nuage(&mut a, 200, true);
        let mut p = Proximite::new(25.0);
        p.reconstruire(&pts);
        for _ in 0..5000 {
            let q = [a.f32() * 999.0, a.f32() * 999.0];
            let vrai = pts
                .iter()
                .map(|h| ((h[0] - q[0]).powi(2) + (h[1] - q[1]).powi(2)).sqrt())
                .fold(f32::INFINITY, f32::min);
            let b = p.distance_min(q);
            assert!(b <= vrai, "{b} > {vrai}");
            // Et pas trop lâche : (c − 1) × s contre au plus (c + 1) × s × √2.
            assert!(
                b >= vrai / std::f32::consts::SQRT_2 - 50.0,
                "borne lâche : {b} pour {vrai}"
            );
        }
    }
}
