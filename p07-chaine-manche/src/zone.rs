//! Zone accessible des zombies (P-04, B-01), sur tout le département.
//!
//! P-04 calculait la plus grande composante connexe d'un bloc de 55 tiles en
//! une seule passe (un tableau d'étiquettes pour tout le bloc). À l'échelle
//! d'un département (10⁹ cases de 2 m) ou de la France (10¹¹), ce tableau ne
//! tient pas en mémoire. On fait donc l'algorithme **distribué** classique :
//!
//! 1. chaque tile étiquette ses composantes (4-connexité sur la grille de
//!    2 m ; c'est la même connexité que la 8-connexité sans coupe de coin de
//!    P-04), en parallèle, et ne garde que les étiquettes de ses quatre bords ;
//! 2. une **union-find** relie les composantes des tiles voisines par leurs
//!    bords : quelques mégaoctets pour tout le département ;
//! 3. chaque tile réétiquette (même résultat, déterministe) et marque ses
//!    cases hors zone.
//!
//! Analogie : une table de routage par zone, où seuls les routeurs de
//! frontière échangent leurs annonces.
//!
//! **Critère d'accès** : une composante est accessible si une route ou un
//! chemin la traverse (troncons non souterrains), ou si c'est la plus grande.
//! Une île (Chausey, Tatihou) ou un îlot isolé par la mer garde ses zombies ;
//! une cour fermée, non.

use crate::raster::Bitmap;

pub const N: usize = 500;
pub const AUCUNE: u32 = u32::MAX;

/// Étiquetage d'une tile : étiquette par case (AUCUNE hors candidat).
pub struct Etiquettes {
    pub lab: Vec<u32>,
    pub tailles: Vec<u32>,
}

pub fn etiqueter(cand: &Bitmap) -> Etiquettes {
    let mut lab = vec![AUCUNE; N * N];
    let mut tailles = Vec::new();
    let mut pile: Vec<u32> = Vec::new();
    for k0 in 0..N * N {
        if lab[k0] != AUCUNE || !cand.get(k0 % N, k0 / N) {
            continue;
        }
        let e = tailles.len() as u32;
        lab[k0] = e;
        pile.push(k0 as u32);
        let mut t = 0u32;
        while let Some(k) = pile.pop() {
            t += 1;
            let k = k as usize;
            let (i, j) = (k % N, k / N);
            let mut v = |kk: usize, ii: usize, jj: usize| {
                if lab[kk] == AUCUNE && cand.get(ii, jj) {
                    lab[kk] = e;
                    pile.push(kk as u32);
                }
            };
            if i > 0 {
                v(k - 1, i - 1, j);
            }
            if i + 1 < N {
                v(k + 1, i + 1, j);
            }
            if j > 0 {
                v(k - N, i, j - 1);
            }
            if j + 1 < N {
                v(k + N, i, j + 1);
            }
        }
        tailles.push(t);
    }
    Etiquettes { lab, tailles }
}

/// Ce qu'une tile garde après la passe 1.
#[derive(Clone, Default)]
pub struct Resume {
    pub tailles: Vec<u32>,
    pub route: Vec<bool>,
    /// Étiquettes des bords : ouest (i = 0), est (i = N-1), sud (j = 0),
    /// nord (j = N-1), chacune de N cases.
    pub ouest: Vec<u32>,
    pub est: Vec<u32>,
    pub sud: Vec<u32>,
    pub nord: Vec<u32>,
}

pub fn resumer(et: &Etiquettes, routes: &Bitmap) -> Resume {
    let mut route = vec![false; et.tailles.len()];
    for k in 0..N * N {
        let l = et.lab[k];
        if l != AUCUNE && routes.get(k % N, k / N) {
            route[l as usize] = true;
        }
    }
    Resume {
        tailles: et.tailles.clone(),
        route,
        ouest: (0..N).map(|j| et.lab[j * N]).collect(),
        est: (0..N).map(|j| et.lab[j * N + N - 1]).collect(),
        sud: (0..N).map(|i| et.lab[i]).collect(),
        nord: (0..N).map(|i| et.lab[(N - 1) * N + i]).collect(),
    }
}

pub struct UnionFind {
    pub p: Vec<u32>,
}

impl UnionFind {
    pub fn new(n: usize) -> Self {
        UnionFind { p: (0..n as u32).collect() }
    }
    pub fn trouver(&mut self, mut x: u32) -> u32 {
        while self.p[x as usize] != x {
            let g = self.p[self.p[x as usize] as usize];
            self.p[x as usize] = g;
            x = g;
        }
        x
    }
    /// La plus petite racine gagne : résultat indépendant de l'ordre.
    pub fn unir(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.trouver(a), self.trouver(b));
        if ra != rb {
            let (lo, hi) = (ra.min(rb), ra.max(rb));
            self.p[hi as usize] = lo;
        }
    }
}

/// Passages à percer (B-02) dans une tile : pour chaque cour fermée (composante
/// hors zone d'au moins `taille_min` cases), le plus court passage à travers le
/// bâti vers la zone, d'au plus `long_max` cases. Parcours en largeur depuis la
/// zone, dans le bâti seulement (jamais dans la mer), ordre fixe.
/// Renvoie les cases à percer et, pour chaque composante fermée, si elle a
/// reçu un passage.
pub fn percer(
    lab: &[u32],
    dans_zone: &[bool],
    taille: &[u32],
    bati: &Bitmap,
    taille_min: u32,
    long_max: u32,
) -> (Vec<u32>, Vec<bool>) {
    let nb = taille.len();
    let mut relie = vec![false; nb];
    // Distance et prédécesseur dans le bâti (cases de bâti seulement).
    let mut dist = vec![u32::MAX; N * N];
    let mut pred = vec![u32::MAX; N * N];
    let mut file = std::collections::VecDeque::new();
    for k in 0..N * N {
        let l = lab[k];
        if l != AUCUNE && dans_zone[l as usize] {
            dist[k] = 0;
            file.push_back(k as u32);
        }
    }
    let mut cases = Vec::new();
    while let Some(k) = file.pop_front() {
        let k = k as usize;
        let d = dist[k];
        let (i, j) = (k % N, k / N);
        let vois = [
            (i > 0).then(|| k - 1),
            (i + 1 < N).then(|| k + 1),
            (j > 0).then(|| k - N),
            (j + 1 < N).then(|| k + N),
        ];
        for kk in vois.into_iter().flatten() {
            let l = lab[kk];
            if l != AUCUNE {
                // Une case candidate : soit la zone (déjà à 0), soit une cour.
                let l = l as usize;
                if !dans_zone[l] && !relie[l] && taille[l] >= taille_min && d > 0 {
                    relie[l] = true;
                    let mut c = k;
                    while dist[c] > 0 {
                        cases.push(c as u32);
                        c = pred[c] as usize;
                    }
                }
                continue;
            }
            if dist[kk] != u32::MAX || !bati.get(kk % N, kk / N) || d + 1 > long_max {
                continue;
            }
            dist[kk] = d + 1;
            pred[kk] = k as u32;
            file.push_back(kk as u32);
        }
    }
    cases.sort_unstable();
    cases.dedup();
    (cases, relie)
}
