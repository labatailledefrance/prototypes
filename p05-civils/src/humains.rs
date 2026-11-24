//! Humains du monde : groupes qui suivent un plan (marcher en colonne le
//! long d'un chemin, attendre en se promenant dans un disque), en boucle
//! ou non. Positions en mètres Lambert-93 (f64), pour tout le bloc.

use std::sync::atomic::{AtomicU32, AtomicU64};

use crate::alea::{hacher, unite};
use crate::carte::Carte;
use crate::sim::{DT, coder};

pub enum Etape {
    /// Promenade aléatoire dans un disque, pendant `ticks`.
    Attendre { ticks: u32, centre: [f64; 2], rayon: f64 },
    /// Colonne le long d'un chemin : le membre j est à `ecart × j` mètres
    /// derrière la tête.
    Marcher { chemin: Vec<[f64; 2]>, cumul: Vec<f64>, vitesse: f64, ecart: f64 },
}

impl Etape {
    pub fn marcher(chemin: Vec<[f64; 2]>, vitesse: f64, ecart: f64) -> Etape {
        let mut cumul = vec![0.0];
        for w in chemin.windows(2) {
            let d = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
            cumul.push(cumul.last().unwrap() + d);
        }
        Etape::Marcher { chemin, cumul, vitesse, ecart }
    }
    pub fn fin(&self) -> [f64; 2] {
        match self {
            Etape::Attendre { centre, .. } => *centre,
            Etape::Marcher { chemin, .. } => *chemin.last().unwrap(),
        }
    }
}

fn point(chemin: &[[f64; 2]], cumul: &[f64], s: f64) -> [f64; 2] {
    let l = *cumul.last().unwrap();
    let s = s.clamp(0.0, l);
    let i = cumul.partition_point(|&c| c <= s).clamp(1, cumul.len() - 1);
    let (a, b) = (chemin[i - 1], chemin[i]);
    let seg = cumul[i] - cumul[i - 1];
    let f = if seg > 0.0 { (s - cumul[i - 1]) / seg } else { 0.0 };
    [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
}

pub struct Groupe {
    pub debut: usize,
    pub n: usize,
    pub etapes: Vec<Etape>,
    pub boucle: bool,
    /// Les membres tirent sur les zombies proches.
    pub tir: bool,
    etape: usize,
    t: u32,
    tete: f64,
}

impl Groupe {
    pub fn new(debut: usize, n: usize, etapes: Vec<Etape>, boucle: bool, tir: bool) -> Groupe {
        let tete = match &etapes[0] {
            Etape::Marcher { ecart, .. } => ecart * (n - 1) as f64,
            _ => 0.0,
        };
        Groupe { debut, n, etapes, boucle, tir, etape: 0, t: 0, tete }
    }
    pub fn fini(&self) -> bool {
        self.etape >= self.etapes.len()
    }
}

#[derive(bevy_ecs::prelude::Resource)]
pub struct Humains {
    pub pos: Vec<[f64; 2]>,
    pub cap: Vec<f32>,
    /// Position canonique de ce tick, relative à l'origine du bloc (f32
    /// codé) : la même valeur pour tous les observateurs, toutes tiles
    /// confondues (déterminisme).
    pub code: Vec<u64>,
    pub tir: Vec<bool>,
    /// Tick (+1) de la dernière observation directe, et position observée
    /// (relative au bloc) : sources des champs « proies ».
    pub obs_tick: Vec<AtomicU32>,
    pub obs_pos: Vec<AtomicU64>,
    pub groupes: Vec<Groupe>,
    pub origine: [f64; 2],
    /// P-05 : point actif (un bâtiment tombé n'est plus perçu) et nombre de
    /// tireurs (occupants armés d'un bâtiment, qui tirent par les fenêtres).
    pub actif: Vec<bool>,
    pub tireurs: Vec<u16>,
}

impl Humains {
    pub fn new(origine: [f64; 2]) -> Humains {
        Humains {
            pos: vec![],
            cap: vec![],
            code: vec![],
            tir: vec![],
            obs_tick: vec![],
            obs_pos: vec![],
            groupes: vec![],
            origine,
            actif: vec![],
            tireurs: vec![],
        }
    }

    /// P-05 : points fixes (portes des bâtiments occupés, ou occupants), sans
    /// groupe ni déplacement.
    pub fn ajouter_points(&mut self, pts: &[[f64; 2]], tireurs: &[u16]) -> usize {
        let debut = self.pos.len();
        for (p, &t) in pts.iter().zip(tireurs) {
            self.pos.push(*p);
            self.cap.push(0.0);
            self.code.push(0);
            self.tir.push(t > 0);
            self.tireurs.push(t);
            self.actif.push(true);
            self.obs_tick.push(AtomicU32::new(0));
            self.obs_pos.push(AtomicU64::new(0));
        }
        self.coder();
        debut
    }

    /// Ajoute un groupe de `n` humains ; positions initiales : début du plan.
    pub fn ajouter(&mut self, carte: &Carte, n: usize, etapes: Vec<Etape>, boucle: bool, tir: bool) {
        let debut = self.pos.len();
        let g = Groupe::new(debut, n, etapes, boucle, tir);
        for j in 0..n {
            let p = match &g.etapes[0] {
                Etape::Marcher { chemin, cumul, ecart, .. } => point(chemin, cumul, g.tete - ecart * j as f64),
                Etape::Attendre { centre, rayon, .. } => {
                    let mut k = 0u64;
                    loop {
                        let h = hacher(0x4855_4D41, (debut + j) as u64 * 1000 + k);
                        let p = [
                            centre[0] + (unite(h) as f64 * 2.0 - 1.0) * rayon,
                            centre[1] + (unite(hacher(h, 1)) as f64 * 2.0 - 1.0) * rayon,
                        ];
                        k += 1;
                        if carte.accessible_monde(p) || k > 10_000 {
                            break p;
                        }
                    }
                }
            };
            self.pos.push(p);
            self.cap.push(unite(hacher(0xCA9, (debut + j) as u64)) * std::f32::consts::TAU);
            self.code.push(0);
            self.tir.push(tir);
            self.tireurs.push(tir as u16);
            self.actif.push(true);
            self.obs_tick.push(AtomicU32::new(0));
            self.obs_pos.push(AtomicU64::new(0));
        }
        self.groupes.push(g);
        self.coder();
    }

    fn coder(&mut self) {
        let o = self.origine;
        for (c, p) in self.code.iter_mut().zip(&self.pos) {
            *c = coder([(p[0] - o[0]) as f32, (p[1] - o[1]) as f32]);
        }
    }

    /// Un tick de déplacement (séquentiel : quelques centaines d'humains).
    pub fn deplacer(&mut self, carte: &Carte, tick: u64, vitesse_promenade: f64) {
        for g in &mut self.groupes {
            if g.fini() {
                continue;
            }
            let suivante;
            match &g.etapes[g.etape] {
                Etape::Marcher { chemin, cumul, vitesse, ecart } => {
                    g.tete += vitesse * DT as f64;
                    let l = *cumul.last().unwrap();
                    for j in 0..g.n {
                        let i = g.debut + j;
                        let s = g.tete - ecart * j as f64;
                        if s >= 0.0 {
                            self.pos[i] = point(chemin, cumul, s);
                        } else {
                            // Pas encore sur le chemin : rejoint son début.
                            let (a, p) = (chemin[0], self.pos[i]);
                            let d = ((a[0] - p[0]).powi(2) + (a[1] - p[1]).powi(2)).sqrt();
                            if d > 0.01 {
                                let k = (vitesse * DT as f64 / d).min(1.0);
                                let q = [p[0] + (a[0] - p[0]) * k, p[1] + (a[1] - p[1]) * k];
                                if carte.accessible_monde(q) {
                                    self.pos[i] = q;
                                }
                            }
                        }
                    }
                    suivante = g.tete - ecart * (g.n - 1) as f64 >= l;
                }
                Etape::Attendre { ticks, centre, rayon } => {
                    for j in 0..g.n {
                        let i = g.debut + j;
                        let h = hacher(i as u64, tick);
                        let mut cap = self.cap[i];
                        if (tick + i as u64) % 40 == 0 {
                            cap += (unite(h) - 0.5) * 2.0;
                        }
                        let p = self.pos[i];
                        let pas = vitesse_promenade * DT as f64;
                        let q = [p[0] + cap.cos() as f64 * pas, p[1] + cap.sin() as f64 * pas];
                        let dans = (q[0] - centre[0]).powi(2) + (q[1] - centre[1]).powi(2) <= rayon * rayon;
                        if dans && carte.accessible_monde(q) {
                            self.pos[i] = q;
                        } else {
                            cap += std::f32::consts::FRAC_PI_2 + unite(hacher(h, 1)) * std::f32::consts::PI;
                        }
                        self.cap[i] = cap;
                    }
                    g.t += 1;
                    suivante = g.t >= *ticks;
                }
            }
            if suivante {
                g.etape += 1;
                g.t = 0;
                if g.etape >= g.etapes.len() && g.boucle {
                    g.etape = 0;
                }
                if let Some(Etape::Marcher { .. }) = g.etapes.get(g.etape) {
                    g.tete = 0.0;
                }
            }
        }
        self.coder();
    }
}
