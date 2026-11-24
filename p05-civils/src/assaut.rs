//! Assaut d'une colonie isolée (scénario 3) : la simulation des zombies de
//! P-04 sur le bloc réel, une colonie dans une tile vidée de ses zombies,
//! entourée de tiles pleines.
//!
//! - Un bâtiment occupé est perçu par les zombies comme **un point à sa
//!   porte** (variante : un point par occupant) dans l'arbre k-d des humains
//!   de P-02 ; ses occupants armés tirent par les fenêtres (tir de P-04).
//! - Un zombie arrivé à moins de 2 m de la porte d'un bâtiment occupé y
//!   **entre** : il sort de la grille et ne bouge plus (état `DEDANS`).
//! - Chaque seconde, le combat intérieur est **auto-résolu** (B-02) :
//!   chaque occupant armé peut abattre un zombie, chaque zombie peut exposer
//!   un occupant. Un exposé devient zombie 10 s plus tard : il quitte le
//!   dossier et naît comme entité zombie dans le bâtiment.
//! - Un bâtiment sans humain libère aussitôt ses zombies. La chute de la
//!   colonie n'est déclenchée par aucune règle.
//!
//! Paramètres : hypothèses de prototype.

use std::path::Path;
use std::time::{Duration, Instant};

use bevy_ecs::prelude::*;

use crate::alea::{hacher, unite};
use crate::carte::Carte;
use crate::civils::{Civils, Commun, Dossiers};
use crate::colon::{ARME, EXPOSE, TPS};
use crate::humains::Humains;
use crate::lieux::Genre;
use crate::sim::{self, Cerveau, Corps, DEDANS, ERRANCE, EtatSlot, EtatTuile, Ident, NB_PHASES, Simulation, Slots};

pub struct ParamsAssaut {
    /// Occupant armé : chance d'abattre un zombie par seconde.
    pub p_tue: f32,
    /// Zombie à l'intérieur : chance d'exposer un occupant par seconde.
    pub p_expose: f32,
    /// Délai de zombification (ticks).
    pub delai_zombie: u32,
    pub rayon_entree: f32,
    /// Un point par occupant (sinon : un point pondéré par bâtiment).
    pub points_occupants: bool,
}

impl Default for ParamsAssaut {
    fn default() -> Self {
        ParamsAssaut { p_tue: 0.3, p_expose: 0.25, delai_zombie: 10 * TPS, rayon_entree: 2.0, points_occupants: false }
    }
}

pub struct Bat {
    pub lieu: u32,
    pub porte: [f64; 2],
    pub tile: usize,
    /// Points de ce bâtiment dans `Humains` : [h0, h0 + nh).
    pub h0: usize,
    pub nh: usize,
    pub dedans: Vec<Entity>,
    /// Exposés : (identifiant, tick de zombification).
    pub exposes: Vec<(u32, u32)>,
    pub tombe: bool,
}

#[derive(Default, Debug, Clone)]
pub struct StatsAssaut {
    pub entrees: u64,
    pub abattus_dedans: u64,
    pub expositions: u64,
    pub zombifies: u64,
    pub sorties: u64,
    pub batiments_tombes: u64,
    pub ns_entrees: u64,
    pub ns_resolution: u64,
    pub ns_zombification: u64,
    pub ns_civils: u64,
    pub ns_sim: u64,
    pub phases: [Duration; NB_PHASES],
    pub chute_tick: Option<u32>,
}

pub struct Assaut {
    pub sim: Simulation,
    pub civils: Civils,
    pub bats: Vec<Bat>,
    pub p: ParamsAssaut,
    pub stats: StatsAssaut,
    pub tick: u32,
    pub humains_initiaux: u64,
}

/// Points de perception d'un bâtiment : sa porte, ou un point par occupant
/// (porte décalée d'au plus 1 m, sur une case accessible).
fn points(carte: &Carte, porte: [f64; 2], n: usize, occupants: bool, graine: u64) -> Vec<[f64; 2]> {
    if !occupants {
        return vec![porte];
    }
    (0..n)
        .map(|k| {
            let h = hacher(graine, k as u64);
            let q = [porte[0] + unite(h) as f64 * 2.0 - 1.0, porte[1] + unite(hacher(h, 1)) as f64 * 2.0 - 1.0];
            if carte.accessible_monde(q) { q } else { porte }
        })
        .collect()
}

impl Assaut {
    /// `civils` : colonie déjà installée (B) ; `carte` : bloc de P-04 dont la
    /// tile de la colonie a été vidée de ses zombies.
    pub fn new(civils: Civils, carte: Carte, p: ParamsAssaut, cfg: &sim::ConfigSim, part_armes: f32) -> Assaut {
        let mut civils = civils;
        // Miliciens : une part des colons est armée.
        {
            let mut ds = civils.world.resource_mut::<Dossiers>();
            for d in ds.0.iter_mut() {
                for c in d.iter_mut() {
                    if unite(hacher(0xA53E, c.id as u64)) < part_armes {
                        c.drapeaux |= ARME;
                    }
                }
            }
        }
        let geo = carte.geo;
        let mut h = Humains::new([geo.x0 as f64 * 1000.0, geo.y0 as f64 * 1000.0]);
        let mut bats = vec![];
        {
            let cm = civils.world.resource::<Commun>();
            let ds = civils.world.resource::<Dossiers>();
            for (l, li) in cm.lieux.iter().enumerate() {
                let d = &ds.0[l];
                if li.genre == Genre::Chantier || d.is_empty() {
                    continue;
                }
                let Some(porte) = crate::marche::porte(&carte, li.pos) else { continue };
                let Some(tile) = geo.tile_de(porte) else { continue };
                let armes = d.iter().filter(|c| c.drapeaux & ARME != 0).count() as u16;
                let pts = points(&carte, porte, d.len(), p.points_occupants, l as u64);
                let mut tir = vec![0u16; pts.len()];
                tir[0] = armes;
                let h0 = h.ajouter_points(&pts, &tir);
                bats.push(Bat {
                    lieu: l as u32,
                    porte,
                    tile,
                    h0,
                    nh: pts.len(),
                    dedans: vec![],
                    exposes: vec![],
                    tombe: false,
                });
            }
        }
        let humains_initiaux = civils.vivants();
        let sim = Simulation::new(cfg, carte, h);
        Assaut { sim, civils, bats, p, stats: StatsAssaut::default(), tick: 0, humains_initiaux }
    }

    fn slot_de(&self, tile: usize) -> Option<usize> {
        match self.sim.topo().etat[tile] {
            EtatTuile::Actif(s) => Some(s as usize),
            _ => None,
        }
    }

    pub fn tick(&mut self) {
        let t = self.tick;
        let t0 = Instant::now();
        let d = self.sim.tick();
        for (a, b) in self.stats.phases.iter_mut().zip(d) {
            *a += b;
        }
        let t1 = Instant::now();
        self.entrees();
        let t2 = Instant::now();
        if t % TPS == 0 {
            self.resolution(t);
        }
        let t3 = Instant::now();
        self.zombifications(t);
        let t4 = Instant::now();
        self.civils.tick();
        let t5 = Instant::now();
        self.stats.ns_sim += (t1 - t0).as_nanos() as u64;
        self.stats.ns_entrees += (t2 - t1).as_nanos() as u64;
        self.stats.ns_resolution += (t3 - t2).as_nanos() as u64;
        self.stats.ns_zombification += (t4 - t3).as_nanos() as u64;
        self.stats.ns_civils += (t5 - t4).as_nanos() as u64;
        if self.stats.chute_tick.is_none() && self.bats.iter().all(|b| b.tombe) {
            self.stats.chute_tick = Some(t);
        }
        self.tick += 1;
    }

    /// Zombies à la porte d'un bâtiment occupé : ils entrent.
    fn entrees(&mut self) {
        let r2 = self.p.rayon_entree * self.p.rayon_entree;
        let mut entrants: Vec<(usize, Entity, u32, u32)> = vec![];
        {
            let slots = &self.sim.world.resource::<Slots>().0;
            for (bi, b) in self.bats.iter().enumerate() {
                if b.tombe {
                    continue;
                }
                let Some(s) = self.slot_de(b.tile) else { continue };
                let sl = &slots[s];
                if sl.etat != EtatSlot::Actif {
                    continue;
                }
                let loc = [(b.porte[0] - sl.origine[0]) as f32, (b.porte[1] - sl.origine[1]) as f32];
                let (cx, cy) = sl.vois.coord(loc);
                let m = sl.vois.cote as i32 - 1;
                for y in (cy - 1).max(0)..=(cy + 1).min(m) {
                    for x in (cx - 1).max(0)..=(cx + 1).min(m) {
                        for e in sl.vois.case(x, y) {
                            let r = e.id as usize;
                            if sl.abattu[r].load(std::sync::atomic::Ordering::Relaxed) != 0 {
                                continue;
                            }
                            let Some(ent) = sl.entites.get(r).copied().flatten() else { continue };
                            let p = sim::decoder(sl.inst[r].load(std::sync::atomic::Ordering::Relaxed));
                            if sim::d2(p, loc) <= r2 {
                                entrants.push((bi, ent, s as u32, r as u32));
                            }
                        }
                    }
                }
            }
        }
        entrants.sort_unstable_by_key(|x| (x.2, x.3));
        entrants.dedup_by_key(|x| (x.2, x.3));
        for (bi, e, s, r) in entrants {
            let Some(mut c) = self.sim.world.get_mut::<Cerveau>(e) else { continue };
            if c.etat == DEDANS {
                continue;
            }
            c.etat = DEDANS;
            self.sim.world.resource::<Slots>().0[s as usize].indexe[r as usize]
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.bats[bi].dedans.push(e);
            self.stats.entrees += 1;
        }
    }

    /// Combat auto-résolu, une fois par seconde, dans chaque bâtiment qui
    /// contient des zombies.
    fn resolution(&mut self, t: u32) {
        let graine = hacher(0xB02, t as u64);
        for bi in 0..self.bats.len() {
            if self.bats[bi].tombe || self.bats[bi].dedans.is_empty() {
                continue;
            }
            let l = self.bats[bi].lieu as usize;
            let mut a_abattre = vec![];
            {
                let mut ds = self.civils.world.resource_mut::<Dossiers>();
                let d = &mut ds.0[l];
                let b = &mut self.bats[bi];
                let hb = hacher(graine, l as u64);
                // Tirs des occupants armés non exposés.
                let mut k = 0u64;
                for c in d.iter() {
                    if c.drapeaux & ARME != 0 && c.drapeaux & EXPOSE == 0 {
                        k += 1;
                        if !b.dedans.is_empty() && unite(hacher(hb, k)) < self.p.p_tue {
                            a_abattre.push(b.dedans.pop().unwrap());
                        }
                    }
                }
                // Expositions par les zombies restants.
                for z in 0..b.dedans.len() as u64 {
                    if unite(hacher(hb, 1 << 32 | z)) >= self.p.p_expose {
                        continue;
                    }
                    let sains: Vec<usize> = (0..d.len()).filter(|&i| d[i].drapeaux & EXPOSE == 0).collect();
                    if sains.is_empty() {
                        break;
                    }
                    let i = sains[(hacher(hb, 2 << 32 | z) % sains.len() as u64) as usize];
                    d[i].drapeaux |= EXPOSE;
                    b.exposes.push((d[i].id, t + self.p.delai_zombie));
                    self.stats.expositions += 1;
                }
            }
            for e in a_abattre {
                let id = *self.sim.world.get::<Ident>(e).unwrap();
                self.sim.abattre(id.slot, id.rang);
                self.stats.abattus_dedans += 1;
            }
        }
    }

    /// Exposés arrivés au terme : zombies. Bâtiments vides : les zombies
    /// sortent.
    fn zombifications(&mut self, t: u32) {
        for bi in 0..self.bats.len() {
            if self.bats[bi].tombe {
                continue;
            }
            let l = self.bats[bi].lieu as usize;
            let dus: Vec<u32> = {
                let b = &mut self.bats[bi];
                let (dus, reste): (Vec<_>, Vec<_>) = b.exposes.iter().partition(|x| x.1 <= t);
                b.exposes = reste;
                dus.into_iter().map(|x| x.0).collect()
            };
            if !dus.is_empty() {
                {
                    let mut ds = self.civils.world.resource_mut::<Dossiers>();
                    ds.0[l].retain(|c| !dus.contains(&c.id));
                }
                {
                    let mut cm = self.civils.world.resource_mut::<Commun>();
                    for _ in &dus {
                        cm.occ[l] -= 1;
                        cm.stats.zombifies += 1;
                        let pos = cm.lieux[l].pos;
                        cm.evenements.push((pos, t));
                    }
                }
                let b = &self.bats[bi];
                if let Some(s) = self.slot_de(b.tile) {
                    let o = self.sim.slots()[s].origine;
                    let loc = [(b.porte[0] - o[0]) as f32, (b.porte[1] - o[1]) as f32];
                    for _ in &dus {
                        let e = self.sim.zombifier(s, loc, DEDANS);
                        self.bats[bi].dedans.push(e);
                        self.stats.zombifies += 1;
                    }
                }
            }
            // Occupants restants, tireurs, points actifs.
            let (n, armes) = {
                let ds = self.civils.world.resource::<Dossiers>();
                let d = &ds.0[l];
                (d.len(), d.iter().filter(|c| c.drapeaux & ARME != 0 && c.drapeaux & EXPOSE == 0).count())
            };
            let b = &mut self.bats[bi];
            let mut h = self.sim.world.resource_mut::<Humains>();
            h.tireurs[b.h0] = armes as u16;
            h.tir[b.h0] = armes > 0;
            if b.nh > 1 {
                for k in 0..b.nh {
                    h.actif[b.h0 + k] = k < n;
                }
            }
            if n == 0 {
                for k in 0..b.nh {
                    h.actif[b.h0 + k] = false;
                }
                b.tombe = true;
                self.stats.batiments_tombes += 1;
                let sortants = std::mem::take(&mut b.dedans);
                drop(h);
                for e in sortants {
                    let id = *self.sim.world.get::<Ident>(e).unwrap();
                    self.sim.world.get_mut::<Cerveau>(e).unwrap().etat = ERRANCE;
                    self.sim.world.resource::<Slots>().0[id.slot as usize].indexe[id.rang as usize]
                        .store(1, std::sync::atomic::Ordering::Relaxed);
                    self.stats.sorties += 1;
                }
            }
        }
    }

    /// Humains : vivants + morts + zombifiés = initial.
    pub fn conservation_humains(&mut self) -> bool {
        self.civils.conservation()
    }

    pub fn zombies_dedans(&self) -> usize {
        self.bats.iter().map(|b| b.dedans.len()).sum()
    }

    pub fn exposes(&self) -> usize {
        self.bats.iter().map(|b| b.exposes.len()).sum()
    }

    pub fn tenus(&self) -> usize {
        self.bats.iter().filter(|b| !b.tombe).count()
    }

    pub fn zombies_vivants(&mut self) -> u64 {
        self.sim.world.query::<&Corps>().iter(&self.sim.world).count() as u64
    }
}

/// Carte du bloc, la tile de la colonie vidée de ses zombies.
pub fn carte_colonie(dossier: &Path, zones: &Path, geo: crate::carte::Geo, densite: f32, tile: usize) -> Carte {
    let (mut c, _) = Carte::reelle(dossier, geo, Some(zones), densite).expect("carte du bloc");
    c.tuiles[tile].population = 0;
    c
}
