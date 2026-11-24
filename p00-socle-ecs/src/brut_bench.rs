//! Référence « Rust brut » : tableaux contigus par champ (SoA) + rayon.
//! C'est le plafond théorique : l'écart avec Bevy mesure le coût de l'ECS.

use rayon::prelude::*;

use crate::logique::{deplacer, empreinte, etat_initial, maj_jauges};
use crate::{Banc, Disposition, Mode};

/// Taille minimale d'un lot parallèle, pour amortir le coût d'ordonnancement.
const LOT_MIN: usize = 4096;

enum Jauges {
    Separees {
        fatigue: Vec<f32>,
        moral: Vec<f32>,
        faim: Vec<f32>,
        soif: Vec<f32>,
    },
    Groupees(Vec<[f32; 4]>),
}

pub struct BancBrut {
    pos: Vec<[f64; 3]>,
    vit: Vec<[f32; 3]>,
    jauges: Jauges,
    pool: Option<rayon::ThreadPool>,
}

impl BancBrut {
    pub fn new(n: u64, disposition: Disposition, mode: Mode, threads: usize) -> Self {
        let n_us = n as usize;
        let mut pos = Vec::with_capacity(n_us);
        let mut vit = Vec::with_capacity(n_us);
        let mut js: [Vec<f32>; 4] = Default::default();
        let mut jg = Vec::new();
        match disposition {
            Disposition::Separees => js.iter_mut().for_each(|v| v.reserve_exact(n_us)),
            Disposition::Groupees => jg.reserve_exact(n_us),
        }
        for i in 0..n {
            let (p, v, j) = etat_initial(i);
            pos.push(p);
            vit.push(v);
            match disposition {
                Disposition::Separees => (0..4).for_each(|k| js[k].push(j[k])),
                Disposition::Groupees => jg.push(j),
            }
        }
        let jauges = match disposition {
            Disposition::Separees => {
                let [fatigue, moral, faim, soif] = js;
                Jauges::Separees {
                    fatigue,
                    moral,
                    faim,
                    soif,
                }
            }
            Disposition::Groupees => Jauges::Groupees(jg),
        };
        let pool = match mode {
            Mode::Mono => None,
            Mode::Par => Some(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .expect("pool rayon"),
            ),
        };
        BancBrut {
            pos,
            vit,
            jauges,
            pool,
        }
    }
}

fn deplacement_seq(pos: &mut [[f64; 3]], vit: &[[f32; 3]]) {
    for (p, v) in pos.iter_mut().zip(vit) {
        *p = deplacer(*p, *v);
    }
}

fn deplacement_par(pos: &mut [[f64; 3]], vit: &[[f32; 3]]) {
    pos.par_iter_mut()
        .zip(vit.par_iter())
        .with_min_len(LOT_MIN)
        .for_each(|(p, v)| *p = deplacer(*p, *v));
}

fn jauges_seq(j: &mut Jauges, vit: &[[f32; 3]]) {
    match j {
        Jauges::Separees {
            fatigue,
            moral,
            faim,
            soif,
        } => {
            let n = vit.len();
            let (fatigue, moral, faim, soif) = (
                &mut fatigue[..n],
                &mut moral[..n],
                &mut faim[..n],
                &mut soif[..n],
            );
            for i in 0..n {
                let r = maj_jauges(vit[i], [fatigue[i], moral[i], faim[i], soif[i]]);
                fatigue[i] = r[0];
                moral[i] = r[1];
                faim[i] = r[2];
                soif[i] = r[3];
            }
        }
        Jauges::Groupees(g) => {
            for (x, v) in g.iter_mut().zip(vit) {
                *x = maj_jauges(*v, *x);
            }
        }
    }
}

fn jauges_par(j: &mut Jauges, vit: &[[f32; 3]]) {
    match j {
        Jauges::Separees {
            fatigue,
            moral,
            faim,
            soif,
        } => {
            (
                vit.par_iter(),
                fatigue.par_iter_mut(),
                moral.par_iter_mut(),
                faim.par_iter_mut(),
                soif.par_iter_mut(),
            )
                .into_par_iter()
                .with_min_len(LOT_MIN)
                .for_each(|(v, f, m, fa, s)| {
                    let r = maj_jauges(*v, [*f, *m, *fa, *s]);
                    *f = r[0];
                    *m = r[1];
                    *fa = r[2];
                    *s = r[3];
                });
        }
        Jauges::Groupees(g) => {
            g.par_iter_mut()
                .zip(vit.par_iter())
                .with_min_len(LOT_MIN)
                .for_each(|(x, v)| *x = maj_jauges(*v, *x));
        }
    }
}

impl Banc for BancBrut {
    fn tick(&mut self) {
        let BancBrut {
            pos,
            vit,
            jauges,
            pool,
        } = self;
        match pool {
            None => {
                deplacement_seq(pos, vit);
                jauges_seq(jauges, vit);
            }
            // Comme l'exécuteur multi-thread de Bevy : les deux « systèmes »
            // tournent en même temps, chacun découpé en lots.
            Some(pool) => pool.install(|| {
                rayon::join(|| deplacement_par(pos, vit), || jauges_par(jauges, vit));
            }),
        }
    }

    fn somme_controle(&mut self) -> u64 {
        let mut h = 0u64;
        for i in 0..self.pos.len() {
            let j = match &self.jauges {
                Jauges::Separees {
                    fatigue,
                    moral,
                    faim,
                    soif,
                } => [fatigue[i], moral[i], faim[i], soif[i]],
                Jauges::Groupees(g) => g[i],
            };
            h = h.wrapping_add(empreinte(self.pos[i], j));
        }
        h
    }
}
