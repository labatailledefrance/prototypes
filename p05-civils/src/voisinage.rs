//! Index des zombies d'une tile (« grille des zombies ») : séparation,
//! capacité par case et recherche de relais. Reconstruit à chaque tick.
//!
//! Quatre structures, interchangeables, qui rendent **exactement** les mêmes
//! voisins dans le même ordre (case par case, ligne par ligne, puis par rang
//! de zombie) ; seule la taille de case change le résultat.
//!
//! | Structure | Principe | Coût de reconstruction |
//! |---|---|---|
//! | `Dense32` (P-01) | tableau « case → début » de u32 + curseurs | ∝ cases (250 000) |
//! | `Dense16` | même tableau en u16, sans curseurs (tri par dénombrement à rebours) | ∝ cases, 3 fois moins d'octets |
//! | `Triee` | zombies triés par case (tri par base, 2 passes) + répertoire des lignes ; recherche dichotomique dans la ligne | ∝ zombies |
//! | `Creuse` | table de hachage des seules cases occupées, entrées marquées par génération (jamais remise à zéro) | ∝ zombies |
//!
//! Analogie : `Dense` est une table d'allocation complète, `Triee` un index
//! trié avec une table des matières, `Creuse` une table de hachage.

#[derive(Clone, Copy, Default)]
pub struct Elem {
    pub p: [f32; 2],
    pub id: u32,
    /// Alerte relayable : tick (+1) de la dernière observation directe de la
    /// cible ; 0 = aucune alerte.
    pub alerte: u32,
    pub cible: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Structure {
    Dense32,
    Dense16,
    Triee,
    Creuse,
}

#[derive(Clone, Copy, Default)]
struct Entree {
    cle: u32,
    generation: u32,
    debut: u32,
    nb: u32,
}

enum Repr {
    Dense32 { debut: Vec<u32>, curseur: Vec<u32> },
    Dense16 { debut: Vec<u16> },
    Triee {
        /// `lignes[y]..lignes[y + 1]` : occupants de la ligne y.
        lignes: Vec<u32>,
        /// Abscisse de case de chaque élément (trié par ligne puis par x).
        xs: Vec<u16>,
        tmp: Vec<u64>,
        tmp2: Vec<u64>,
        cx: Vec<u32>,
    },
    Creuse {
        table: Vec<Entree>,
        masque: u32,
        generation: u32,
        occupees: Vec<u32>,
        fente: Vec<u32>,
    },
}

pub struct Voisinage {
    pub cote: usize,
    inv_taille: f32,
    pub taille: f32,
    pub elems: Vec<Elem>,
    /// Un bit par case : un occupant porte une alerte (filtre du relais).
    alertes: Vec<u64>,
    repr: Repr,
    pub structure: Structure,
}

#[inline(always)]
fn hache(cle: u32) -> u32 {
    cle.wrapping_mul(0x9E37_79B1)
}

impl Voisinage {
    /// Couvre un carré de côté `etendue`, cases de côté `taille`, pour au
    /// plus `n_max` éléments.
    pub fn new(s: Structure, etendue: f32, taille: f32, n_max: usize) -> Self {
        let cote = (etendue / taille).ceil() as usize;
        let repr = match s {
            Structure::Dense32 => Repr::Dense32 {
                debut: vec![0; cote * cote + 1],
                curseur: vec![0; cote * cote],
            },
            Structure::Dense16 => {
                assert!(n_max < u16::MAX as usize);
                Repr::Dense16 {
                    debut: vec![0; cote * cote + 1],
                }
            }
            Structure::Triee => Repr::Triee {
                lignes: vec![0; cote + 1],
                xs: vec![],
                tmp: vec![],
                tmp2: vec![],
                cx: vec![0; cote + 1],
            },
            Structure::Creuse => {
                let taille_table = (2 * n_max).next_power_of_two().max(1024);
                Repr::Creuse {
                    table: vec![Entree::default(); taille_table],
                    masque: taille_table as u32 - 1,
                    generation: 0,
                    occupees: vec![],
                    fente: vec![],
                }
            }
        };
        Voisinage {
            cote,
            inv_taille: 1.0 / taille,
            taille,
            elems: vec![],
            alertes: vec![0; (cote * cote).div_ceil(64)],
            repr,
            structure: s,
        }
    }

    #[inline(always)]
    pub fn coord(&self, p: [f32; 2]) -> (i32, i32) {
        let m = self.cote as i32 - 1;
        (
            ((p[0] * self.inv_taille) as i32).clamp(0, m),
            ((p[1] * self.inv_taille) as i32).clamp(0, m),
        )
    }

    /// Reconstruit l'index à partir des points `i < n` tels que `garder(i)`.
    /// L'identifiant d'un élément est son rang. Ordre déterministe.
    pub fn reconstruire_avec(
        &mut self,
        n: usize,
        garder: impl Fn(usize) -> bool,
        point: impl Fn(usize) -> [f32; 2],
        alerte: impl Fn(usize) -> (u32, [f32; 2]),
    ) {
        self.alertes.fill(0);
        let cote = self.cote;
        let inv = self.inv_taille;
        let m = cote as i32 - 1;
        let coord = |p: [f32; 2]| {
            (
                ((p[0] * inv) as i32).clamp(0, m) as usize,
                ((p[1] * inv) as i32).clamp(0, m) as usize,
            )
        };
        let poser = |elems: &mut Vec<Elem>, alertes: &mut Vec<u64>, j: usize, i: usize, p: [f32; 2], k: usize| {
            let (a, cible) = alerte(i);
            if a != 0 {
                alertes[k >> 6] |= 1 << (k & 63);
            }
            elems[j] = Elem {
                p,
                id: i as u32,
                alerte: a,
                cible,
            };
        };
        match &mut self.repr {
            Repr::Dense32 { debut, curseur } => {
                // P-01/P-02, inchangé.
                debut.fill(0);
                let mut total = 0;
                for i in 0..n {
                    if !garder(i) {
                        continue;
                    }
                    total += 1;
                    let (x, y) = coord(point(i));
                    debut[y * cote + x + 1] += 1;
                }
                for c in 1..debut.len() {
                    debut[c] += debut[c - 1];
                }
                curseur.copy_from_slice(&debut[..debut.len() - 1]);
                self.elems.resize(total, Elem::default());
                for i in 0..n {
                    if !garder(i) {
                        continue;
                    }
                    let p = point(i);
                    let (x, y) = coord(p);
                    let k = y * cote + x;
                    let c = &mut curseur[k];
                    poser(&mut self.elems, &mut self.alertes, *c as usize, i, p, k);
                    *c += 1;
                }
            }
            Repr::Dense16 { debut } => {
                // Tri par dénombrement à rebours : sommes cumulées inclusives
                // (fin de chaque case), puis placement de n - 1 à 0 en
                // décrémentant ; à la fin, debut[k] est le début de la case k.
                // Pas de tableau de curseurs, et des u16 : 3 fois moins
                // d'octets à écrire que Dense32.
                debut.fill(0);
                let nc = cote * cote;
                let mut total = 0u16;
                for i in 0..n {
                    if !garder(i) {
                        continue;
                    }
                    total += 1;
                    let (x, y) = coord(point(i));
                    debut[y * cote + x] += 1;
                }
                let mut s = 0u16;
                for d in &mut debut[..nc] {
                    s += *d;
                    *d = s;
                }
                debut[nc] = total;
                self.elems.resize(total as usize, Elem::default());
                for i in (0..n).rev() {
                    if !garder(i) {
                        continue;
                    }
                    let p = point(i);
                    let (x, y) = coord(p);
                    let k = y * cote + x;
                    debut[k] -= 1;
                    let j = debut[k] as usize;
                    poser(&mut self.elems, &mut self.alertes, j, i, p, k);
                }
            }
            Repr::Triee {
                lignes,
                xs,
                tmp,
                tmp2,
                cx,
            } => {
                // Tri par base en deux passes stables (x, puis y) sur des
                // clés de 64 bits : y (16) | x (16) | rang (32).
                tmp.clear();
                cx.fill(0);
                lignes.fill(0);
                for i in 0..n {
                    if !garder(i) {
                        continue;
                    }
                    let (x, y) = coord(point(i));
                    cx[x + 1] += 1;
                    lignes[y + 1] += 1;
                    tmp.push((y as u64) << 48 | (x as u64) << 32 | i as u64);
                }
                for c in 1..=cote {
                    cx[c] += cx[c - 1];
                    lignes[c] += lignes[c - 1];
                }
                let total = tmp.len();
                tmp2.resize(total, 0);
                for &k in tmp.iter() {
                    let x = (k >> 32 & 0xFFFF) as usize;
                    tmp2[cx[x] as usize] = k;
                    cx[x] += 1;
                }
                // Deuxième passe : par ligne ; `cx` sert de curseur.
                cx[..cote].copy_from_slice(&lignes[..cote]);
                for &k in tmp2.iter() {
                    let y = (k >> 48) as usize;
                    tmp[cx[y] as usize] = k;
                    cx[y] += 1;
                }
                self.elems.resize(total, Elem::default());
                xs.resize(total, 0);
                for (j, &k) in tmp.iter().enumerate() {
                    let i = (k & 0xFFFF_FFFF) as usize;
                    let x = (k >> 32 & 0xFFFF) as usize;
                    let y = (k >> 48) as usize;
                    xs[j] = x as u16;
                    poser(&mut self.elems, &mut self.alertes, j, i, point(i), y * cote + x);
                }
            }
            Repr::Creuse {
                table,
                masque,
                generation,
                occupees,
                fente,
            } => {
                // Grille creuse (hachage spatial) : seules les cases occupées
                // ont une entrée. Une entrée d'une génération passée est
                // vide : la table n'est jamais remise à zéro.
                *generation = generation.wrapping_add(1);
                if *generation == 0 {
                    table.fill(Entree::default());
                    *generation = 1;
                }
                let g = *generation;
                occupees.clear();
                fente.clear();
                fente.resize(n, u32::MAX);
                for i in 0..n {
                    if !garder(i) {
                        continue;
                    }
                    let (x, y) = coord(point(i));
                    let cle = (y * cote + x) as u32;
                    let mut s = hache(cle) & *masque;
                    loop {
                        let e = &mut table[s as usize];
                        if e.generation != g {
                            *e = Entree {
                                cle,
                                generation: g,
                                debut: 0,
                                nb: 1,
                            };
                            occupees.push(s);
                            break;
                        }
                        if e.cle == cle {
                            e.nb += 1;
                            break;
                        }
                        s = (s + 1) & *masque;
                    }
                    fente[i] = s;
                }
                // Débuts dans l'ordre de première apparition ; `nb` devient
                // le curseur.
                let mut total = 0u32;
                for &s in occupees.iter() {
                    let e = &mut table[s as usize];
                    e.debut = total;
                    total += e.nb;
                    e.nb = 0;
                }
                self.elems.resize(total as usize, Elem::default());
                for i in 0..n {
                    let s = fente[i];
                    if s == u32::MAX {
                        continue;
                    }
                    let e = &mut table[s as usize];
                    let j = (e.debut + e.nb) as usize;
                    e.nb += 1;
                    let k = e.cle as usize;
                    poser(&mut self.elems, &mut self.alertes, j, i, point(i), k);
                }
            }
        }
    }

    /// Occupants de la case (x, y), supposée dans la grille, par rang.
    #[inline(always)]
    pub fn case(&self, x: i32, y: i32) -> &[Elem] {
        self.plage(y, x, x)
    }

    /// Occupants des cases x0..=x1 de la ligne y (bornes dans la grille),
    /// case par case puis par rang : le même ordre pour toutes les
    /// structures. Pour Dense et Triee, c'est une seule tranche contiguë.
    #[inline(always)]
    pub fn plage(&self, y: i32, x0: i32, x1: i32) -> &[Elem] {
        let (y, x0, x1) = (y as usize, x0 as usize, x1 as usize);
        let cote = self.cote;
        match &self.repr {
            Repr::Dense32 { debut, .. } => {
                &self.elems[debut[y * cote + x0] as usize..debut[y * cote + x1 + 1] as usize]
            }
            Repr::Dense16 { debut } => {
                &self.elems[debut[y * cote + x0] as usize..debut[y * cote + x1 + 1] as usize]
            }
            Repr::Triee { lignes, xs, .. } => {
                let (a, b) = (lignes[y] as usize, lignes[y + 1] as usize);
                let r = &xs[a..b];
                // Lignes courtes : parcours linéaire ; sinon dichotomie.
                let (i0, i1) = if r.len() <= 16 {
                    let i0 = r.iter().position(|&x| x as usize >= x0).unwrap_or(r.len());
                    let i1 = i0 + r[i0..].iter().position(|&x| x as usize > x1).unwrap_or(r.len() - i0);
                    (i0, i1)
                } else {
                    let i0 = r.partition_point(|&x| (x as usize) < x0);
                    let i1 = i0 + r[i0..].partition_point(|&x| (x as usize) <= x1);
                    (i0, i1)
                };
                &self.elems[a + i0..a + i1]
            }
            Repr::Creuse { .. } => {
                // Cases non contiguës en mémoire : on ne rend qu'une case à
                // la fois (voir `pour_plage`).
                debug_assert_eq!(x0, x1);
                self.creuse_case(x0, y)
            }
        }
    }

    #[inline(always)]
    fn creuse_case(&self, x: usize, y: usize) -> &[Elem] {
        let Repr::Creuse {
            table,
            masque,
            generation,
            ..
        } = &self.repr
        else {
            unreachable!()
        };
        let cle = (y * self.cote + x) as u32;
        let mut s = hache(cle) & masque;
        loop {
            let e = &table[s as usize];
            if e.generation != *generation {
                return &[];
            }
            if e.cle == cle {
                return &self.elems[e.debut as usize..(e.debut + e.nb) as usize];
            }
            s = (s + 1) & masque;
        }
    }

    /// Appelle `f` sur les occupants des cases x0..=x1 de la ligne y, dans
    /// l'ordre commun ; s'arrête dès que `f` renvoie `false`. Renvoie
    /// `false` si l'on s'est arrêté.
    #[inline(always)]
    pub fn pour_plage(&self, y: i32, x0: i32, x1: i32, mut f: impl FnMut(&Elem) -> bool) -> bool {
        if let Repr::Creuse { .. } = self.repr {
            for x in x0..=x1 {
                for e in self.creuse_case(x as usize, y as usize) {
                    if !f(e) {
                        return false;
                    }
                }
            }
            return true;
        }
        for e in self.plage(y, x0, x1) {
            if !f(e) {
                return false;
            }
        }
        true
    }

    /// Nombre d'occupants de la case (x, y).
    #[inline(always)]
    pub fn occupants(&self, x: i32, y: i32) -> u32 {
        if let Repr::Creuse { .. } = self.repr {
            return self.creuse_case(x as usize, y as usize).len() as u32;
        }
        self.plage(y, x, x).len() as u32
    }

    /// Un occupant de la case (x, y) porte-t-il une alerte ?
    #[inline(always)]
    pub fn alerte_dans(&self, x: i32, y: i32) -> bool {
        let k = y as usize * self.cote + x as usize;
        (self.alertes[k >> 6] >> (k & 63)) & 1 == 1
    }

    /// Octets touchés par une reconstruction, hors éléments (indicatif).
    pub fn octets_repertoire(&self) -> usize {
        let a = self.alertes.len() * 8;
        a + match &self.repr {
            Repr::Dense32 { debut, curseur } => (debut.len() + curseur.len()) * 4,
            Repr::Dense16 { debut } => debut.len() * 2,
            Repr::Triee { lignes, xs, tmp, .. } => lignes.len() * 4 + xs.len() * 2 + tmp.len() * 16,
            Repr::Creuse { occupees, fente, .. } => occupees.len() * 16 + fente.len() * 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alea::Alea;

    /// Toutes les structures rendent les mêmes occupants, dans le même
    /// ordre, pour toutes les plages, y compris avec des cases très pleines.
    #[test]
    fn structures_equivalentes() {
        let mut a = Alea::new(11);
        let n = 5000;
        let pts: Vec<[f32; 2]> = (0..n)
            .map(|i| {
                if i % 3 == 0 {
                    // Une foule dense autour d'un point.
                    [500.0 + a.f32() * 6.0, 500.0 + a.f32() * 6.0]
                } else {
                    [a.f32() * 1000.0, a.f32() * 1000.0]
                }
            })
            .collect();
        let garder = |i: usize| i % 7 != 0;
        let alerte = |i: usize| ((i % 5 == 0) as u32 * i as u32, [i as f32, 0.0]);
        let mut vs: Vec<Voisinage> = [Structure::Dense32, Structure::Dense16, Structure::Triee, Structure::Creuse]
            .into_iter()
            .map(|s| Voisinage::new(s, 1000.0, 2.0, n))
            .collect();
        for _ in 0..3 {
            for v in &mut vs {
                v.reconstruire_avec(n, garder, |i| pts[i], alerte);
            }
        }
        let cle = |v: &Voisinage, y: i32, x0: i32, x1: i32| {
            let mut r = vec![];
            v.pour_plage(y, x0, x1, |e| {
                r.push((e.id, e.alerte));
                true
            });
            r
        };
        for _ in 0..20_000 {
            let p = if a.f32() < 0.3 {
                [500.0 + a.f32() * 8.0 - 1.0, 500.0 + a.f32() * 8.0 - 1.0]
            } else {
                [a.f32() * 1000.0, a.f32() * 1000.0]
            };
            let (x, y) = vs[0].coord(p);
            let (x0, x1) = ((x - 1).max(0), (x + 1).min(499));
            let r = cle(&vs[0], y, x0, x1);
            for v in &vs[1..] {
                assert_eq!(r, cle(v, y, x0, x1), "{:?}", v.structure);
                assert_eq!(vs[0].occupants(x, y), v.occupants(x, y));
                assert_eq!(vs[0].alerte_dans(x, y), v.alerte_dans(x, y));
            }
        }
    }
}

#[cfg(test)]
mod banc {
    use super::*;
    use crate::alea::Alea;

    /// `cargo test --release banc_isole -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn banc_isole() {
        let mut a = Alea::new(3);
        let n = 20_000;
        let pts: Vec<[f32; 2]> = (0..n).map(|_| [a.f32() * 1000.0, a.f32() * 1000.0]).collect();
        let al: Vec<(u32, [f32; 2])> = (0..n).map(|i| ((i % 5 == 0) as u32, [0.0; 2])).collect();
        for s in [Structure::Dense32, Structure::Dense16, Structure::Triee, Structure::Creuse] {
            let mut v = Voisinage::new(s, 1000.0, 2.0, n);
            v.reconstruire_avec(n, |_| true, |i| pts[i], |i| al[i]);
            let t0 = std::time::Instant::now();
            for _ in 0..200 {
                v.reconstruire_avec(n, |_| true, |i| pts[i], |i| al[i]);
            }
            eprintln!("{s:?} : {:.1} µs", t0.elapsed().as_secs_f64() * 1e6 / 200.0);
        }
        let mut b = vec![0u64; 3907];
        let t0 = std::time::Instant::now();
        for _ in 0..200 {
            b.fill(0);
            std::hint::black_box(&b);
        }
        eprintln!("fill 31 Ko : {:.1} µs", t0.elapsed().as_secs_f64() * 1e6 / 200.0);
    }
}
