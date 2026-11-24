//! Génération des bâtiments d'une tile.
//!
//! Une tile génère les bâtiments qu'elle **possède** (règle de P-07 : le
//! centroïde est dans la tile), en entier, même s'ils débordent ; ses voisines
//! ne les génèrent pas. Chaque bâtiment ne dépend que de ses attributs, de sa
//! graine (son identifiant) et de données fixes de la tile et de ses
//! voisines : on peut donc le générer dans n'importe quel ordre, sur n'importe
//! quel thread, et la tile fusionnée est rangée par identifiant.
//!
//! Analogie : un `map` sans état partagé suivi d'un `reduce` ordonné, comme
//! un build parallèle dont l'éditeur de liens range les objets par nom.

use crate::batiment::{self, Entree, Etat, Lod, Maillage, Params, Porte, Repli, Sortie};
use crate::donnees::{Bat, Contexte};
use crate::porte::{self, Choix, Grille};
use rayon::prelude::*;
use std::collections::HashMap;
use std::time::Instant;

pub fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// Ce qui se calcule une fois par tile avant les bâtiments.
pub struct Prep {
    pub repli: Repli,
    pub modeles: Vec<batiment::Modele>,
    pub portes: Vec<Option<Choix>>,
    /// Cases percées du contexte dont le centre tombe dans la boîte de chaque
    /// bâtiment (filtrées ensuite polygone par polygone).
    pub porches: Vec<Vec<(i32, i32)>>,
    pub grille: Option<Grille>,
    pub ms_repli: f64,
    pub ms_grille: f64,
    pub ms_portes: f64,
    pub ms_porches: f64,
}

pub fn preparer(ctx: &Contexte, p: &Params, avec_portes: bool) -> Prep {
    let bats = &ctx.tile().bats;
    let t0 = Instant::now();
    let repli = batiment::repli(bats, None);
    let modeles: Vec<_> = bats.iter().map(|b| batiment::modele(b, &repli, p)).collect();
    let ms_repli = ms(t0);
    let t0 = Instant::now();
    let grille = avec_portes.then(|| porte::grille(ctx));
    let ms_grille = ms(t0);
    let t0 = Instant::now();
    let portes = match &grille {
        Some(g) => bats.iter().map(|b| Some(porte::choisir(b, g))).collect(),
        None => vec![None; bats.len()],
    };
    let ms_portes = ms(t0);
    let t0 = Instant::now();
    let perc = ctx.percees();
    let porches = bats
        .iter()
        .map(|b| {
            let bb = b.bbox();
            let (gi0, gj0) = ((bb[0] + 8000).div_euclid(32), (bb[1] + 8000).div_euclid(32));
            let (gi1, gj1) = ((bb[2] + 8000).div_euclid(32), (bb[3] + 8000).div_euclid(32));
            // Liste triée (gi, gj) : recherche par plage de gi.
            let lo = perc.partition_point(|c| c.0 < gi0);
            perc[lo..].iter().take_while(|c| c.0 <= gi1).filter(|c| c.1 >= gj0 && c.1 <= gj1).copied().collect()
        })
        .collect();
    let ms_porches = ms(t0);
    Prep { repli, modeles, portes, porches, grille, ms_repli, ms_grille, ms_portes, ms_porches }
}

pub struct Gen<'a> {
    pub ctx: &'a Contexte,
    pub prep: &'a Prep,
    pub p: &'a Params,
    pub lod: Lod,
    pub etats: &'a HashMap<u64, Etat>,
}

impl Gen<'_> {
    pub fn bat(&self, k: usize) -> Sortie {
        let b: &Bat = &self.ctx.tile().bats[k];
        let sol = |x: f64, y: f64| self.ctx.sol(x, y);
        let porte: Option<Porte> = self.prep.portes[k].and_then(|c| c.porte);
        let e = Entree {
            b,
            m: &self.prep.modeles[k],
            sol: &sol,
            porte,
            porches: &self.prep.porches[k],
            etat: self.etats.get(&b.id).copied().unwrap_or(Etat::Intact),
            lod: self.lod,
            p: self.p,
        };
        batiment::generer(&e)
    }
    pub fn n(&self) -> usize {
        self.ctx.tile().bats.len()
    }
}

/// Ordre de génération (pour le contrôle de déterminisme).
#[derive(Clone, Copy, Debug)]
pub enum Ordre {
    Direct,
    Inverse,
    Melange(u64),
}

pub fn ordre(n: usize, o: Ordre) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).collect();
    match o {
        Ordre::Direct => {}
        Ordre::Inverse => v.reverse(),
        Ordre::Melange(g) => {
            // Fisher-Yates sur la graine.
            let mut s = g;
            for i in (1..n).rev() {
                s = batiment::hacher(s);
                v.swap(i, (s % (i as u64 + 1)) as usize);
            }
        }
    }
    v
}

/// Génère tous les bâtiments : séquentiel (avec le temps de chacun) ou
/// parallèle (rayon, sur les bâtiments). Résultat rangé par indice.
pub fn tout(g: &Gen, par: bool, o: Ordre) -> (Vec<Sortie>, Vec<f64>) {
    let n = g.n();
    let ord = ordre(n, o);
    let mut res: Vec<Option<(Sortie, f64)>> = (0..n).map(|_| None).collect();
    if par {
        let v: Vec<(usize, Sortie, f64)> = ord
            .par_iter()
            .with_min_len(16)
            .map(|&k| {
                let t = Instant::now();
                let s = g.bat(k);
                (k, s, t.elapsed().as_secs_f64() * 1e6)
            })
            .collect();
        for (k, s, us) in v {
            res[k] = Some((s, us));
        }
    } else {
        for &k in &ord {
            let t = Instant::now();
            let s = g.bat(k);
            res[k] = Some((s, t.elapsed().as_secs_f64() * 1e6));
        }
    }
    let (s, us) = res.into_iter().map(|x| x.unwrap()).unzip();
    (s, us)
}

/// Génération **étalée** : au plus `budget_ms` par tick (le bâtiment en
/// cours finit toujours). Renvoie les durées des ticks.
pub fn etale(g: &Gen, budget_ms: f64) -> (Vec<Sortie>, Vec<f64>) {
    let n = g.n();
    let mut out = Vec::with_capacity(n);
    let mut ticks = Vec::new();
    let mut k = 0;
    while k < n {
        let t = Instant::now();
        while k < n {
            out.push(g.bat(k));
            k += 1;
            if ms(t) >= budget_ms {
                break;
            }
        }
        ticks.push(ms(t));
    }
    (out, ticks)
}

/// Fusion en un maillage de tile, dans l'ordre des identifiants ; renvoie
/// aussi la plage d'indices de chaque bâtiment (pour la mise à jour d'un seul).
pub fn fusionner(s: &[Sortie]) -> (Maillage, Vec<(u32, u32)>) {
    let mut m = Maillage::default();
    let nv: usize = s.iter().map(|x| x.mesh.pos.len()).sum();
    let ni: usize = s.iter().map(|x| x.mesh.idx.len()).sum();
    m.pos.reserve(nv);
    m.nor.reserve(nv);
    m.uv.reserve(nv);
    m.col.reserve(nv);
    m.idx.reserve(ni);
    let mut plages = Vec::with_capacity(s.len());
    for x in s {
        let a = m.idx.len() as u32;
        m.ajouter(&x.mesh);
        plages.push((a, m.idx.len() as u32));
    }
    (m, plages)
}

/// Morceaux de `cote` m (fusion par sous-carré de la tile, selon le
/// centroïde) : la mise à jour d'un bâtiment ne refait qu'un morceau.
pub fn morceau_de(b: &Bat, cote: f64) -> (i32, i32) {
    let c = porte::centroide(b);
    let n = (1000.0 / cote) as i32;
    ((((c[0] + 500.0) / cote).floor() as i32).clamp(0, n - 1), (((c[1] + 500.0) / cote).floor() as i32).clamp(0, n - 1))
}

/// Empreinte d'une tile : BLAKE3 des octets canoniques du maillage fusionné.
pub fn empreinte(s: &[Sortie]) -> blake3::Hash {
    let mut h = blake3::Hasher::new();
    for x in s {
        h.update(x.mesh.empreinte().as_bytes());
    }
    h.finalize()
}
