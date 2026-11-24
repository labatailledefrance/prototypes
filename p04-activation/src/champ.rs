//! Flow fields : champ d'intégration (distance à pied aux sources) sur une
//! grille de cases de 1, 2 ou 5 m, et direction de descente.
//!
//! Analogie : une table de routage à vecteur de distances calculée d'un coup
//! pour toute la tile. Chaque case connaît sa distance à la source la plus
//! proche ; un zombie suit la pente, comme un paquet suit le prochain saut.
//!
//! - **Terrain** : coût d'entrée de chaque case (0 = infranchissable,
//!   2 = rue, 3 = rue au contact d'un mur si la **marge** est active, 80 =
//!   barricade, franchissable par entassement selon A-05, donc très chère
//!   mais pas interdite). La marge éloigne la foule des façades : sans elle,
//!   les plus courts chemins rasent les murs.
//! - **Calcul** : algorithme de Dial (Dijkstra à seaux, coûts entiers 5 en
//!   ligne droite et 7 en diagonale, soit √2 à 1 % près). Pas de passage en
//!   diagonale entre deux coins de mur.
//! - **Recalcul partiel** après l'apparition d'un obstacle : on invalide les
//!   cases dont le plus court chemin passait par l'obstacle (ses
//!   « descendants »), puis on les recalcule depuis leur bordure. Analogie :
//!   un retrait de route qui n'invalide que les préfixes appris par ce lien.
//!   Le résultat est identique au calcul complet (vérifié par les tests).
//! - **Horodatage** : chaque case porte l'horodatage de sa source (dernière
//!   observation directe), comme l'alerte relayée. Il sert à l'échange de
//!   bordures entre tiles, où il empêche le « comptage à l'infini ».

use crate::ville::{COTE, Ville};

pub const INF: u16 = u16::MAX;
pub const ORTHO: u16 = 5;
pub const DIAG: u16 = 7;
/// Coût de base d'une case de barricade (× celui d'une rue).
pub const COUT_BARRICADE: u8 = 40;
/// Unités de distance pour une case en ligne droite dans une rue (coût 2
/// × 5) : distance en mètres = d × résolution / UNITE.
pub const UNITE: f32 = 10.0;
/// Marge autour de la tile : un anneau de halo (cases de la tile voisine,
/// pour l'échange de bordures), puis un anneau toujours infranchissable.
pub const BORD: usize = 2;
/// Seaux de l'algorithme de Dial : puissance de 2 > plus grand coût d'arête.
const SEAUX: usize = 1024;

/// Règle de passage d'une case grossière (2 ou 5 m) : franchissable si
/// au moins la moitié de ses mètres carrés sont libres, ou si un seul l'est.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Regle {
    Majorite,
    Toute,
}

/// Case fine (1 m) : libre, bâtie, ou barricade.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fin {
    Libre,
    Bati,
    Barricade,
}

pub fn fin_de(v: &Ville, x: i32, y: i32) -> Fin {
    if v.barricade(x, y) {
        Fin::Barricade
    } else if v.bati(x, y) {
        Fin::Bati
    } else {
        Fin::Libre
    }
}

#[derive(Clone)]
pub struct Terrain {
    /// Côté d'une case, en mètres.
    pub res: usize,
    /// Cases par côté, sans la marge.
    pub n: usize,
    /// Largeur de la grille avec la marge.
    pub w: usize,
    pub cout: Vec<u8>,
    /// Arêtes sortantes franchissables de chaque case (bit k : vers le
    /// voisin k, case franchissable et pas de coin coupé). Précalculé : la
    /// boucle de Dial ne relit pas les coûts des coins.
    pub masque: Vec<u8>,
    /// Coût de base (0, 1 ou barricade), avant marge ; l'anneau extérieur
    /// porte celui de la 2e rangée voisine (marge exacte au bord du halo).
    base: Vec<u8>,
    marge: bool,
    inv_res: f32,
    off: [isize; 8],
    regle: Regle,
}

/// Déplacements vers les 8 voisins : (dx, dy, coût de base).
pub const DIRS: [(i32, i32, u16); 8] = [
    (1, 0, ORTHO),
    (-1, 0, ORTHO),
    (0, 1, ORTHO),
    (0, -1, ORTHO),
    (1, 1, DIAG),
    (-1, 1, DIAG),
    (1, -1, DIAG),
    (-1, -1, DIAG),
];
const VECT: [[f32; 2]; 8] = {
    let r = std::f32::consts::FRAC_1_SQRT_2;
    [
        [1.0, 0.0],
        [-1.0, 0.0],
        [0.0, 1.0],
        [0.0, -1.0],
        [r, r],
        [-r, r],
        [r, -r],
        [-r, -r],
    ]
};

impl Terrain {
    /// Terrain d'une tile de `COTE` m, cases de `res` m. `fin(x, y)` donne la
    /// case de 1 m, coordonnées locales, y compris hors de la tile (halo) ;
    /// `halo` : lire aussi l'anneau de cases voisines.
    pub fn depuis(res: usize, regle: Regle, marge: bool, halo: bool, fin: impl Fn(i32, i32) -> Fin) -> Terrain {
        assert!(COTE % res == 0);
        let n = COTE / res;
        Self::depuis_n(n, res, regle, marge, halo, fin)
    }

    pub fn depuis_n(
        n: usize,
        res: usize,
        regle: Regle,
        marge: bool,
        halo: bool,
        fin: impl Fn(i32, i32) -> Fin,
    ) -> Terrain {
        let w = n + 2 * BORD;
        let wi = w as isize;
        let mut t = Terrain {
            res,
            n,
            w,
            cout: vec![0; w * w],
            masque: vec![0; w * w],
            base: vec![0; w * w],
            marge,
            inv_res: 1.0 / res as f32,
            off: DIRS.map(|(dx, dy, _)| dx as isize + dy as isize * wi),
            regle,
        };
        let lo = if halo { -1 } else { 0 };
        let hi = if halo { n as i32 + 1 } else { n as i32 };
        let (lb, hb) = if halo { (lo - 1, hi + 1) } else { (lo, hi) };
        for cy in lb..hb {
            for cx in lb..hb {
                let c = t.cout_case(cx, cy, &fin);
                let i = t.ixy(cx, cy);
                t.base[i] = c;
            }
        }
        for cy in lo..hi {
            for cx in lo..hi {
                let i = t.ixy(cx, cy);
                t.cout[i] = t.cout_final(i);
            }
        }
        for y in 1..w - 1 {
            for x in 1..w - 1 {
                let i = y * w + x;
                t.masque[i] = t.calc_masque(i);
            }
        }
        t
    }

    /// Coût final : rue 2, rue contre un mur 3 (marge), barricade 80.
    fn cout_final(&self, i: usize) -> u8 {
        let b = self.base[i];
        if b == 0 {
            return 0;
        }
        let pres_mur = self.marge && (0..8).any(|k| self.base[(i as isize + self.off[k]) as usize] == 0);
        b * 2 + pres_mur as u8
    }

    fn calc_masque(&self, u: usize) -> u8 {
        let mut m = 0u8;
        for k in 0..8 {
            let v = (u as isize + self.off[k]) as usize;
            if self.cout[v] != 0 && self.arete(u, k) {
                m |= 1 << k;
            }
        }
        m
    }

    /// Change le coût d'une case et met à jour les masques alentour.
    pub fn fixer_cout(&mut self, i: usize, c: u8) {
        self.cout[i] = c;
        let w = self.w as isize;
        for dy in -2..=2isize {
            for dx in -2..=2isize {
                let j = i as isize + dy * w + dx;
                let (x, y) = (j % w, j / w);
                if x >= 1 && y >= 1 && x < w - 1 && y < w - 1 {
                    self.masque[j as usize] = self.calc_masque(j as usize);
                }
            }
        }
    }

    /// P-04 : terrain vide d'une tile avec halo, à remplir par tranches
    /// (`avancer_chantier`). Les anneaux extérieurs restent à 0 : murs.
    pub fn vide(res: usize, regle: Regle, marge: bool) -> Terrain {
        assert!(COTE % res == 0);
        let n = COTE / res;
        let w = n + 2 * BORD;
        let wi = w as isize;
        Terrain {
            res,
            n,
            w,
            cout: vec![0; w * w],
            masque: vec![0; w * w],
            base: vec![0; w * w],
            marge,
            inv_res: 1.0 / res as f32,
            off: DIRS.map(|(dx, dy, _)| dx as isize + dy as isize * wi),
            regle,
        }
    }

    /// P-04 : avance la construction du terrain (avec halo) d'au plus
    /// `budget` unités (une unité = une case de 1 m lue, ou une case
    /// grossière finalisée) ; renvoie vrai quand le terrain est complet.
    /// Trois passes successives, chacune par lignes : coût de base (lit la
    /// carte), coût final (marge, lit les voisines de la passe 1), masques
    /// d'arêtes (lit les coûts de la passe 2). Le résultat est identique à
    /// `depuis(.., halo = true, ..)`, quel que soit le budget.
    pub fn avancer_chantier(&mut self, ch: &mut ChantierTerrain, budget: u64, fin: &impl Fn(i32, i32) -> Fin) -> bool {
        let n = self.n as i32;
        let mut fait = 0u64;
        loop {
            match ch.passe {
                0 => {
                    if ch.ligne >= n + 2 {
                        ch.passe = 1;
                        ch.ligne = -1;
                        continue;
                    }
                    let cy = ch.ligne;
                    for cx in -2..n + 2 {
                        let c = self.cout_case(cx, cy, fin);
                        let i = self.ixy(cx, cy);
                        self.base[i] = c;
                    }
                    fait += ((n + 4) * (self.res * self.res) as i32) as u64;
                }
                1 => {
                    if ch.ligne >= n + 1 {
                        ch.passe = 2;
                        ch.ligne = 1;
                        continue;
                    }
                    let cy = ch.ligne;
                    for cx in -1..n + 1 {
                        let i = self.ixy(cx, cy);
                        self.cout[i] = self.cout_final(i);
                    }
                    fait += (n + 2) as u64;
                }
                2 => {
                    if ch.ligne >= self.w as i32 - 1 {
                        ch.passe = 3;
                        continue;
                    }
                    let y = ch.ligne as usize;
                    for x in 1..self.w - 1 {
                        let i = y * self.w + x;
                        self.masque[i] = self.calc_masque(i);
                    }
                    fait += 2 * self.w as u64;
                }
                _ => return true,
            }
            ch.ligne += 1;
            if fait >= budget {
                return false;
            }
        }
    }

    /// Terrain d'une tile seule (pas de halo).
    pub fn de_ville(v: &Ville, res: usize, regle: Regle, marge: bool) -> Terrain {
        Self::depuis(res, regle, marge, false, |x, y| fin_de(v, x, y))
    }

    fn cout_case(&self, cx: i32, cy: i32, fin: &impl Fn(i32, i32) -> Fin) -> u8 {
        let r = self.res as i32;
        let (mut libres, mut barricade) = (0, false);
        for y in cy * r..(cy + 1) * r {
            for x in cx * r..(cx + 1) * r {
                match fin(x, y) {
                    Fin::Libre => libres += 1,
                    Fin::Barricade => {
                        libres += 1;
                        barricade = true
                    }
                    Fin::Bati => {}
                }
            }
        }
        let total = r * r;
        // Une barricade occupe la rue : elle compte comme libre pour la
        // franchissabilité, puis rend la case chère. Une barricade ne rend
        // donc jamais franchissable une case qui ne l'était pas : la poser
        // ne fait que monter des coûts, la détruire que les baisser.
        let ok = match self.regle {
            Regle::Majorite => 2 * libres >= total,
            Regle::Toute => libres > 0,
        };
        match (ok, barricade) {
            (false, _) => 0,
            (true, true) => COUT_BARRICADE,
            (true, false) => 1,
        }
    }

    /// Recalcule les cases qui recouvrent le rectangle de cases de 1 m
    /// [x0, x1] × [y0, y1] ; renvoie celles dont le coût a monté, et
    /// celles dont il a baissé (traitées par deux recalculs différents).
    pub fn mettre_a_jour(
        &mut self,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        fin: impl Fn(i32, i32) -> Fin,
    ) -> Maj {
        let r = self.res as i32;
        let m = self.n as i32 - 1;
        let (cx0, cy0, cx1, cy1) = ((x0 / r).max(0), (y0 / r).max(0), (x1 / r).min(m), (y1 / r).min(m));
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let c = self.cout_case(cx, cy, &fin);
                let i = self.ixy(cx, cy);
                self.base[i] = c;
            }
        }
        // La marge des voisines peut changer aussi.
        let mut maj = Maj::default();
        for cy in (cy0 - 1).max(0)..=(cy1 + 1).min(m) {
            for cx in (cx0 - 1).max(0)..=(cx1 + 1).min(m) {
                let i = self.ixy(cx, cy);
                let (avant, c) = (self.cout[i], self.cout_final(i));
                if avant != c {
                    // 0 (infranchissable) est le coût le plus élevé.
                    if c == 0 || (avant != 0 && c > avant) {
                        maj.hausses.push(i as u32);
                    } else {
                        maj.baisses.push(i as u32);
                    }
                    self.cout[i] = c;
                }
            }
        }
        for cy in (cy0 - 3).max(-1)..=(cy1 + 3).min(m + 1) {
            for cx in (cx0 - 3).max(-1)..=(cx1 + 3).min(m + 1) {
                let i = self.ixy(cx, cy);
                self.masque[i] = self.calc_masque(i);
            }
        }
        maj
    }

    /// Indice d'une case, coordonnées de case (-1..=n avec le halo).
    #[inline(always)]
    pub fn ixy(&self, cx: i32, cy: i32) -> usize {
        (cy + BORD as i32) as usize * self.w + (cx + BORD as i32) as usize
    }

    /// Indice de la case qui contient le point local `p` (mètres).
    #[inline(always)]
    pub fn indice(&self, p: [f32; 2]) -> usize {
        let m = self.n as i32 - 1;
        let cx = ((p[0] * self.inv_res) as i32).clamp(0, m);
        let cy = ((p[1] * self.inv_res) as i32).clamp(0, m);
        self.ixy(cx, cy)
    }

    /// Centre de la case `i`, en mètres locaux.
    #[inline(always)]
    pub fn centre(&self, i: usize) -> [f32; 2] {
        let cx = (i % self.w) as f32 - BORD as f32;
        let cy = (i / self.w) as f32 - BORD as f32;
        [(cx + 0.5) * self.res as f32, (cy + 0.5) * self.res as f32]
    }

    /// Coordonnées de case de l'indice `i`.
    #[inline(always)]
    pub fn cxy(&self, i: usize) -> (i32, i32) {
        (
            (i % self.w) as i32 - BORD as i32,
            (i / self.w) as i32 - BORD as i32,
        )
    }

    /// Arête u → v (voisin k) franchissable ? Pas de coin coupé en diagonale.
    #[inline(always)]
    fn arete(&self, u: usize, k: usize) -> bool {
        if k < 4 {
            return true;
        }
        let (dx, dy, _) = DIRS[k];
        self.cout[(u as isize + dx as isize) as usize] != 0
            && self.cout[(u as isize + dy as isize * self.w as isize) as usize] != 0
    }

    /// Cases franchissables de l'intérieur.
    pub fn libres(&self) -> usize {
        (0..self.n as i32)
            .flat_map(|y| (0..self.n as i32).map(move |x| (x, y)))
            .filter(|&(x, y)| self.cout[self.ixy(x, y)] != 0)
            .count()
    }
}

/// P-04 : curseur de construction d'un terrain par tranches.
#[derive(Clone, Copy, Debug)]
pub struct ChantierTerrain {
    pub passe: u8,
    pub ligne: i32,
}

impl Default for ChantierTerrain {
    fn default() -> Self {
        ChantierTerrain { passe: 0, ligne: -2 }
    }
}

/// Cases modifiées par une mise à jour du terrain.
#[derive(Default, Debug)]
pub struct Maj {
    pub hausses: Vec<u32>,
    pub baisses: Vec<u32>,
}

/// Une source, ou une graine de bordure : case, distance initiale,
/// horodatage (tick de la dernière observation directe, modulo 2¹⁶).
#[derive(Clone, Copy, Debug)]
pub struct Graine {
    pub i: u32,
    pub d: u16,
    pub ts: u16,
}

#[derive(Clone)]
pub struct Champ {
    pub dist: Vec<u16>,
    pub ts: Vec<u16>,
    /// Aucune source : le champ ne guide personne.
    pub vide: bool,
    /// Version des sources ayant servi au calcul (bruits).
    pub version: u64,
}

impl Champ {
    pub fn new(t: &Terrain) -> Champ {
        Champ {
            dist: vec![INF; t.w * t.w],
            ts: vec![0; t.w * t.w],
            vide: true,
            version: u64::MAX,
        }
    }

    /// Direction de descente au point `p`, ou `None` (arrivé, ou rien à
    /// suivre). Gradient lissé sur les 8 voisins plus proches de la source,
    /// pondérés par la pente : 256 directions plutôt que 8.
    #[inline(always)]
    pub fn direction(&self, t: &Terrain, p: [f32; 2]) -> Option<[f32; 2]> {
        if self.vide {
            return None;
        }
        let i = t.indice(p);
        let d = self.dist[i];
        if d == 0 {
            return None;
        }
        if d == INF {
            // Case hors du champ (coin de mur à une résolution grossière) :
            // vers le voisin le plus proche de la source.
            let mut best = (INF, 0usize);
            for k in 0..8 {
                let j = (i as isize + t.off[k]) as usize;
                if self.dist[j] < best.0 {
                    best = (self.dist[j], j);
                }
            }
            if best.0 == INF {
                return None;
            }
            let c = t.centre(best.1);
            let v = [c[0] - p[0], c[1] - p[1]];
            let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
            return (l > 1e-6).then(|| [v[0] / l, v[1] / l]);
        }
        let mut g = [0f32; 2];
        for k in 0..8 {
            let j = (i as isize + t.off[k]) as usize;
            let v = self.dist[j];
            if v < d && (t.masque[i] >> k) & 1 == 1 {
                let s = (d - v) as f32 / DIRS[k].2 as f32;
                g[0] += s * VECT[k][0];
                g[1] += s * VECT[k][1];
            }
        }
        let l = (g[0] * g[0] + g[1] * g[1]).sqrt();
        (l > 1e-6).then(|| [g[0] / l, g[1] / l])
    }

    /// Distance en mètres (approximation 5/7) au point `p`.
    pub fn metres(&self, t: &Terrain, p: [f32; 2]) -> Option<f32> {
        let d = self.dist[t.indice(p)];
        (!self.vide && d != INF).then(|| d as f32 * t.res as f32 / UNITE)
    }
}

/// Mémoire de travail de l'algorithme de Dial, réutilisée d'un calcul à
/// l'autre (pas d'allocation en régime établi).
pub struct Dial {
    seaux: Vec<Vec<u32>>,
    marque: Vec<u32>,
    generation: u32,
    pile: Vec<u32>,
    reinit: Vec<u32>,
    graines: Vec<Graine>,
    /// Calcul en cours (interruptible) : graines triées et état.
    cours: Vec<Graine>,
    etat: Option<Etat>,
    fixees_cours: u64,
}

#[derive(Clone, Copy)]
struct Etat {
    cur: u32,
    gi: usize,
    en_attente: usize,
}

/// Statistiques d'un calcul.
#[derive(Default, Clone, Copy, Debug)]
pub struct Travail {
    /// Cases fixées (sorties d'un seau avec leur distance définitive).
    pub fixees: u64,
    /// Recalcul partiel : cases invalidées.
    pub invalidees: u64,
}

impl Dial {
    pub fn new(t: &Terrain) -> Dial {
        Dial {
            seaux: (0..SEAUX).map(|_| Vec::new()).collect(),
            marque: vec![0; t.w * t.w],
            generation: 0,
            pile: vec![],
            reinit: vec![],
            graines: vec![],
            cours: vec![],
            etat: None,
            fixees_cours: 0,
        }
    }

    /// Calcul complet depuis les sources, d'un seul tenant.
    pub fn calculer(&mut self, t: &Terrain, c: &mut Champ, sources: &[Graine]) -> Travail {
        self.abandonner();
        self.demarrer(c, sources);
        let (_, f) = self.avancer(t, c, u64::MAX);
        Travail {
            fixees: f,
            invalidees: 0,
        }
    }

    /// Commence un calcul complet interruptible dans `c` (qui ne doit plus
    /// être lu avant la fin) ; à poursuivre avec `avancer`.
    pub fn demarrer(&mut self, c: &mut Champ, sources: &[Graine]) {
        self.abandonner();
        c.dist.fill(INF);
        c.vide = sources.is_empty();
        self.cours.clear();
        self.cours.extend_from_slice(sources);
        self.cours.sort_unstable_by_key(|s| (s.d, s.i));
        self.fixees_cours = 0;
        self.etat = (!self.cours.is_empty()).then(|| Etat {
            cur: self.cours[0].d as u32,
            gi: 0,
            en_attente: 0,
        });
    }

    pub fn en_cours(&self) -> bool {
        self.etat.is_some()
    }

    /// Cases fixées depuis le début du calcul en cours (ou du dernier).
    pub fn fixees_cours(&self) -> u64 {
        self.fixees_cours
    }

    /// Abandonne le calcul en cours.
    pub fn abandonner(&mut self) {
        if self.etat.take().is_some() {
            for s in &mut self.seaux {
                s.clear();
            }
        }
    }

    /// Poursuit le calcul en cours jusqu'à fixer au moins `budget` cases ;
    /// renvoie (terminé, cases fixées pendant cet appel). Dijkstra à seaux
    /// depuis des graines de distances initiales quelconques ; ne fait que
    /// baisser des distances.
    pub fn avancer(&mut self, t: &Terrain, c: &mut Champ, budget: u64) -> (bool, u64) {
        let Some(mut e) = self.etat.take() else {
            return (true, 0);
        };
        let g = &self.cours;
        let masque = SEAUX - 1;
        let mut fixees = 0u64;
        let fini = loop {
            while e.gi < g.len() && g[e.gi].d as u32 == e.cur {
                let s = g[e.gi];
                e.gi += 1;
                if s.d < c.dist[s.i as usize] {
                    c.dist[s.i as usize] = s.d;
                    c.ts[s.i as usize] = s.ts;
                    self.seaux[e.cur as usize & masque].push(s.i);
                    e.en_attente += 1;
                }
            }
            if e.en_attente == 0 {
                if e.gi == g.len() {
                    break true;
                }
                e.cur = g[e.gi].d as u32;
                continue;
            }
            let mut seau = std::mem::take(&mut self.seaux[e.cur as usize & masque]);
            let cur = e.cur;
            for &u in &seau {
                e.en_attente -= 1;
                let u = u as usize;
                if c.dist[u] as u32 != cur {
                    continue;
                }
                fixees += 1;
                let tsu = c.ts[u];
                let mut m = t.masque[u];
                while m != 0 {
                    let k = m.trailing_zeros() as usize;
                    m &= m - 1;
                    let v = (u as isize + t.off[k]) as usize;
                    let nd = cur + DIRS[k].2 as u32 * t.cout[v] as u32;
                    if nd < c.dist[v] as u32 {
                        c.dist[v] = nd as u16;
                        c.ts[v] = tsu;
                        self.seaux[nd as usize & masque].push(v as u32);
                        e.en_attente += 1;
                    }
                }
            }
            seau.clear();
            self.seaux[cur as usize & masque] = seau;
            e.cur += 1;
            if e.cur >= INF as u32 {
                // Saturation (champ de plus de 13 km à 1 m) : le reste
                // reste à l'infini.
                for s in &mut self.seaux {
                    s.clear();
                }
                break true;
            }
            if fixees >= budget {
                break false;
            }
        };
        self.fixees_cours += fixees;
        if !fini {
            self.etat = Some(e);
        }
        (fini, fixees)
    }

    /// Propagation synchrone depuis des graines (recalculs partiels).
    fn propager(&mut self, t: &Terrain, c: &mut Champ, g: &mut Vec<Graine>) -> u64 {
        assert!(self.etat.is_none(), "recalcul partiel pendant un calcul étalé");
        std::mem::swap(&mut self.cours, g);
        self.cours.sort_unstable_by_key(|s| (s.d, s.i));
        self.etat = (!self.cours.is_empty()).then(|| Etat {
            cur: self.cours[0].d as u32,
            gi: 0,
            en_attente: 0,
        });
        self.fixees_cours = 0;
        let (_, f) = self.avancer(t, c, u64::MAX);
        std::mem::swap(&mut self.cours, g);
        f
    }

    fn nouvelle_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.marque.fill(0);
            self.generation = 1;
        }
    }

    /// Recalcul partiel après une **hausse** de coût (obstacle apparu) sur
    /// les cases `modifiees` (terrain déjà mis à jour). `sources` : les
    /// sources du champ (à réinjecter si elles sont invalidées).
    pub fn recalcul_hausse(
        &mut self,
        t: &Terrain,
        c: &mut Champ,
        modifiees: &[u32],
        sources: &[Graine],
    ) -> Travail {
        if c.vide {
            return Travail::default();
        }
        self.nouvelle_generation();
        let g_cour = self.generation;
        self.pile.clear();
        self.reinit.clear();
        // Ensemble de départ : les cases modifiées et leurs voisines (une
        // diagonale peut avoir été coupée par un nouveau coin de mur).
        for &m in modifiees {
            let m = m as usize;
            for j in std::iter::once(m).chain((0..8).map(|k| (m as isize + t.off[k]) as usize)) {
                if self.marque[j] != g_cour && c.dist[j] != INF {
                    self.marque[j] = g_cour;
                    self.pile.push(j as u32);
                }
            }
        }
        // Descendants : v dont un plus court chemin passe par u invalidé.
        while let Some(u) = self.pile.pop() {
            let u = u as usize;
            self.reinit.push(u as u32);
            let du = c.dist[u] as u32;
            for k in 0..8 {
                let v = (u as isize + t.off[k]) as usize;
                if self.marque[v] == g_cour || c.dist[v] == INF {
                    continue;
                }
                let cv = t.cout[v] as u32;
                if cv != 0 && c.dist[v] as u32 == du + DIRS[k].2 as u32 * cv {
                    self.marque[v] = g_cour;
                    self.pile.push(v as u32);
                }
            }
        }
        for &r in &self.reinit {
            c.dist[r as usize] = INF;
        }
        // Graines : bordure de la zone invalidée, et sources invalidées.
        let mut g = std::mem::take(&mut self.graines);
        g.clear();
        for &r in &self.reinit {
            let r = r as usize;
            let cr = t.cout[r] as u32;
            if cr == 0 {
                continue;
            }
            let mut best = (INF as u32, 0u16);
            for k in 0..8 {
                // Arête voisin → r : même test de coin que r → voisin.
                let n = (r as isize + t.off[k]) as usize;
                if self.marque[n] == g_cour || c.dist[n] == INF || t.cout[n] == 0 || !t.arete(r, k) {
                    continue;
                }
                let d = c.dist[n] as u32 + DIRS[k].2 as u32 * cr;
                if d < best.0 {
                    best = (d, c.ts[n]);
                }
            }
            if best.0 < INF as u32 {
                g.push(Graine {
                    i: r as u32,
                    d: best.0 as u16,
                    ts: best.1,
                });
            }
        }
        for s in sources {
            if self.marque[s.i as usize] == g_cour {
                g.push(*s);
            }
        }
        let f = self.propager(t, c, &mut g);
        self.graines = g;
        Travail {
            fixees: f,
            invalidees: self.reinit.len() as u64,
        }
    }

    /// Recalcul partiel après une **baisse** de coût (obstacle détruit).
    pub fn recalcul_baisse(&mut self, t: &Terrain, c: &mut Champ, modifiees: &[u32]) -> Travail {
        if c.vide {
            return Travail::default();
        }
        let mut g = std::mem::take(&mut self.graines);
        g.clear();
        // Les cases rouvertes, et leurs voisines : une diagonale entre deux
        // voisines a pu être rouverte par la disparition d'un coin.
        self.nouvelle_generation();
        let g_cour = self.generation;
        self.pile.clear();
        for &m in modifiees {
            let m = m as usize;
            for j in std::iter::once(m).chain((0..8).map(|k| (m as isize + t.off[k]) as usize)) {
                if self.marque[j] != g_cour {
                    self.marque[j] = g_cour;
                    self.pile.push(j as u32);
                }
            }
        }
        let candidates = std::mem::take(&mut self.pile);
        for &m in &candidates {
            let m = m as usize;
            let cm = t.cout[m] as u32;
            if cm == 0 {
                continue;
            }
            let mut best = (c.dist[m] as u32, c.ts[m]);
            for k in 0..8 {
                let n = (m as isize + t.off[k]) as usize;
                if c.dist[n] == INF || t.cout[n] == 0 || !t.arete(m, k) {
                    continue;
                }
                let d = c.dist[n] as u32 + DIRS[k].2 as u32 * cm;
                if d < best.0 {
                    best = (d, c.ts[n]);
                }
            }
            if best.0 < c.dist[m] as u32 {
                g.push(Graine {
                    i: m as u32,
                    d: best.0 as u16,
                    ts: best.1,
                });
                // `propager` n'injecte que ce qui améliore : on relâche la
                // distance pour que la graine soit prise.
                c.dist[m] = INF;
            }
        }
        self.pile = candidates;
        let f = self.propager(t, c, &mut g);
        self.graines = g;
        Travail {
            fixees: f,
            invalidees: 0,
        }
    }
}

// ------------------------------------------------------------ Frontières

/// Bordure exportée par une tile vers ses voisines : pour chaque case de
/// l'intérieur au bord, distance et horodatage.
pub struct Echange {
    pub n: usize,
    /// Côtés : 0 = x min, 1 = x max, 2 = y min, 3 = y max ; coins compris.
    pub cotes: [Vec<(u16, u16)>; 4],
}

impl Echange {
    pub fn exporter(t: &Terrain, c: &Champ) -> Echange {
        let n = t.n as i32;
        let lire = |x: i32, y: i32| {
            let i = t.ixy(x, y);
            if c.vide { (INF, 0) } else { (c.dist[i], c.ts[i]) }
        };
        Echange {
            n: t.n,
            cotes: [
                (0..n).map(|y| lire(0, y)).collect(),
                (0..n).map(|y| lire(n - 1, y)).collect(),
                (0..n).map(|x| lire(x, 0)).collect(),
                (0..n).map(|x| lire(x, n - 1)).collect(),
            ],
        }
    }
}

/// Graines de halo d'une tile à partir des bordures de ses 8 voisines
/// (`voisines[dy + 1][dx + 1]`, `None` = pas de voisine). Une graine plus
/// vieille que `age_max` (horodatage) est ignorée : une valeur relayée ne
/// survit pas à sa source, comme l'alerte (pas de comptage à l'infini).
pub fn graines_halo(
    t: &Terrain,
    voisines: &[[Option<&Echange>; 3]; 3],
    maintenant: u16,
    age_max: Option<u16>,
    g: &mut Vec<Graine>,
) {
    let n = t.n as i32;
    let mut pousser = |x: i32, y: i32, (d, ts): (u16, u16)| {
        if d == INF {
            return;
        }
        if let Some(a) = age_max
            && maintenant.wrapping_sub(ts) > a
        {
            return;
        }
        let i = t.ixy(x, y);
        if t.cout[i] != 0 {
            g.push(Graine { i: i as u32, d, ts });
        }
    };
    // Côtés : la case de halo (x = -1) est la case x = n - 1 de la voisine
    // de gauche, etc.
    if let Some(e) = voisines[1][0] {
        for y in 0..n {
            pousser(-1, y, e.cotes[1][y as usize]);
        }
    }
    if let Some(e) = voisines[1][2] {
        for y in 0..n {
            pousser(n, y, e.cotes[0][y as usize]);
        }
    }
    if let Some(e) = voisines[0][1] {
        for x in 0..n {
            pousser(x, -1, e.cotes[3][x as usize]);
        }
    }
    if let Some(e) = voisines[2][1] {
        for x in 0..n {
            pousser(x, n, e.cotes[2][x as usize]);
        }
    }
    // Coins.
    let coins = [
        (0, 0, -1, -1, 3, n - 1),
        (0, 2, n, -1, 3, 0),
        (2, 0, -1, n, 2, n - 1),
        (2, 2, n, n, 2, 0),
    ];
    for (vy, vx, x, y, cote, k) in coins {
        if let Some(e) = voisines[vy][vx] {
            pousser(x, y, e.cotes[cote][k as usize]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alea::Alea;
    use std::collections::BinaryHeap;

    /// Dijkstra de référence, avec un tas binaire.
    fn reference(t: &Terrain, sources: &[Graine]) -> Vec<u16> {
        let mut d = vec![INF as u32; t.w * t.w];
        let mut h = BinaryHeap::new();
        for s in sources {
            if (s.d as u32) < d[s.i as usize] {
                d[s.i as usize] = s.d as u32;
                h.push(std::cmp::Reverse((s.d as u32, s.i as usize)));
            }
        }
        while let Some(std::cmp::Reverse((du, u))) = h.pop() {
            if du != d[u] {
                continue;
            }
            for k in 0..8 {
                let v = (u as isize + t.off[k]) as usize;
                if t.cout[v] == 0 || !t.arete(u, k) {
                    continue;
                }
                let nd = du + DIRS[k].2 as u32 * t.cout[v] as u32;
                if nd < d[v] && nd < INF as u32 {
                    d[v] = nd;
                    h.push(std::cmp::Reverse((nd, v)));
                }
            }
        }
        d.into_iter().map(|x| x as u16).collect()
    }

    fn ville_test(graine: u64) -> Ville {
        Ville::generer(graine)
    }

    fn sources(t: &Terrain, a: &mut Alea, k: usize) -> Vec<Graine> {
        let mut v = vec![];
        while v.len() < k {
            let p = [a.f32() * 1000.0, a.f32() * 1000.0];
            let i = t.indice(p);
            if t.cout[i] != 0 {
                v.push(Graine {
                    i: i as u32,
                    d: 0,
                    ts: v.len() as u16,
                });
            }
        }
        v
    }

    #[test]
    fn dial_egale_dijkstra() {
        let v = ville_test(3);
        for res in [1, 2, 5] {
            let mut t = Terrain::de_ville(&v, res, Regle::Majorite, true);
            // Quelques barricades (coût élevé) au hasard.
            let mut a = Alea::new(7);
            for _ in 0..200 {
                let i = a.entre(0, t.w * t.w - 1);
                if t.cout[i] == 2 {
                    t.fixer_cout(i, COUT_BARRICADE * 2);
                }
            }
            let s = sources(&t, &mut a, 5);
            let mut c = Champ::new(&t);
            let mut d = Dial::new(&t);
            d.calculer(&t, &mut c, &s);
            assert_eq!(c.dist, reference(&t, &s), "résolution {res}");
        }
    }

    /// Le recalcul partiel donne exactement le calcul complet, après une
    /// hausse (barricade, effondrement) puis une baisse (barricade détruite).
    #[test]
    fn partiel_egale_complet() {
        let mut v = ville_test(5);
        for res in [1, 2, 5] {
            let mut t = Terrain::de_ville(&v, res, Regle::Majorite, true);
            let mut a = Alea::new(res as u64);
            let s = sources(&t, &mut a, 3);
            let mut c = Champ::new(&t);
            let mut d = Dial::new(&t);
            d.calculer(&t, &mut c, &s);
            // Barricade : une bande de 3 m sur 60 m, et un bâtiment effondré
            // qui bloque une zone (coût 0 → passage fermé).
            let (x0, y0) = (a.entre(100, 800) as i32, a.entre(100, 800) as i32);
            for y in y0..y0 + 3 {
                for x in x0..x0 + 60 {
                    v.poser_barricade(x as usize, y as usize);
                }
            }
            let m = t.mettre_a_jour(x0, y0, x0 + 60, y0 + 3, |x, y| fin_de(&v, x, y));
            assert!(m.baisses.is_empty(), "une barricade ne fait que monter les coûts");
            d.recalcul_hausse(&t, &mut c, &m.hausses, &s);
            assert_eq!(c.dist, reference(&t, &s), "hausse, résolution {res}");
            // Effondrement : un carré de 30 m devient infranchissable.
            let mut t2 = Terrain::de_ville(&v, res, Regle::Majorite, true);
            let mut c2 = Champ::new(&t2);
            d.calculer(&t2, &mut c2, &s);
            let (x1, y1) = (a.entre(100, 800) as i32, a.entre(100, 800) as i32);
            let mut modif = vec![];
            for cy in y1 / res as i32..(y1 + 30) / res as i32 {
                for cx in x1 / res as i32..(x1 + 30) / res as i32 {
                    let i = t2.ixy(cx, cy);
                    if t2.cout[i] != 0 {
                        t2.fixer_cout(i, 0);
                        modif.push(i as u32);
                    }
                }
            }
            d.recalcul_hausse(&t2, &mut c2, &modif, &s);
            let r2 = reference(&t2, &s);
            for (i, (&a, &b)) in c2.dist.iter().zip(&r2).enumerate() {
                // Les cases effondrées ne sont plus atteintes que si elles
                // sont sources : seules les cases franchissables comptent.
                if t2.cout[i] != 0 {
                    assert_eq!(a, b, "effondrement, résolution {res}, case {i}");
                }
            }
            // Baisse : on rouvre la zone effondrée.
            for &i in &modif {
                t2.fixer_cout(i as usize, 2);
            }
            d.recalcul_baisse(&t2, &mut c2, &modif);
            assert_eq!(c2.dist, reference(&t2, &s), "baisse, résolution {res}");
        }
    }
}
