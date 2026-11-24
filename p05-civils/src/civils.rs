//! Le monde civil : lieux, stocks des colonies, carte de risque, carte des
//! corps, trajets en cours ; et les représentations comparées.
//!
//! - **A1** : une entité ECS par colon, phase en champ ; chaque tick parcourt
//!   toutes les entités et ne met à jour que celles de la phase.
//! - **A2** : une entité ECS par colon, rangée dans l'une de 20 tables par
//!   un composant marqueur de partition (phase modulo 20) ; chaque tick ne
//!   parcourt qu'une table.
//! - **B** : un dossier compact par lieu (tableau de `Colon` de 32 octets),
//!   hors de l'ECS ; chaque tick ne parcourt que les lieux de sa phase.
//! - **C** : un agrégat statistique par logement (effectifs et moyennes).
//!
//! A1, A2 et B appellent la même fonction `colon::vivre` : ils doivent avoir
//! la même somme de contrôle. Analogie : A est un processus par client, B un
//! pool de workers qui lisent l'état des clients en base, C des compteurs
//! agrégés façon métriques.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use bevy_ecs::batching::BatchingStrategy;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::MultiThreadedExecutor;
use bevy_tasks::ComputeTaskPool;
use bevy_utils::Parallel;

use crate::alea::{hacher, unite};
use crate::colon::*;
use crate::lieux::{Genre, Implantation, Lieu};

#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Repr {
    A1,
    A2,
    B,
    C,
}

pub const PARTITIONS: u32 = 20;
/// Carte de risque : mise à jour de chaque tile une fois par minute de jeu.
pub const PERIODE_RISQUE: u32 = 60 * TPS;
/// Carte des corps : cases de 10 m, 100 × 100 par tile (A-16).
pub const COTE_CORPS: usize = 100;

// ---------------------------------------------------------------- Composants (A)

#[derive(Component)]
pub struct Fiche(pub Colon);

/// Lieu du colon (sa destination s'il est en trajet), phase de ce lieu,
/// tick d'arrivée.
#[derive(Component, Clone, Copy)]
pub struct Place {
    pub lieu: u32,
    pub phase: u32,
    pub arrivee: u32,
}

#[derive(Component)]
pub struct Position(pub [f64; 2]);

#[derive(Component)]
pub struct Part<const K: u8>;

// ---------------------------------------------------------------- État commun

/// Un trajet en cours : le colon (B) ou seulement son arrivée (A).
pub struct Transit {
    pub arrivee: u32,
    pub id: u32,
    pub vers: u32,
    pub colon: Option<Colon>,
}
impl PartialEq for Transit {
    fn eq(&self, o: &Self) -> bool {
        (self.arrivee, self.id) == (o.arrivee, o.id)
    }
}
impl Eq for Transit {}
impl PartialOrd for Transit {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Transit {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.arrivee, self.id).cmp(&(o.arrivee, o.id))
    }
}

#[derive(Default, Clone, Debug)]
pub struct StatsCivils {
    pub morts: u64,
    pub zombifies: u64,
    pub malades: i64,
    pub nouveaux_malades: u64,
    pub gueris: u64,
    pub travail_ticks: u64,
    pub mises_a_jour: u64,
    pub departs: u64,
    pub deficit: [u64; NB_RES],
    pub consomme: [u64; NB_RES],
    pub demande: [u64; NB_RES],
}

#[derive(Resource)]
pub struct Commun {
    pub p: ParamsCivils,
    pub lieux: Vec<Lieu>,
    pub pos_lieux: Vec<[f64; 2]>,
    /// Début de chaque phase dans `lieux` (lieux triés par phase).
    pub debut_phase: Vec<u32>,
    pub colonies: usize,
    pub tiles: Vec<(i64, i64)>,
    pub lieux_tile: Vec<Vec<u32>>,
    pub stocks: Vec<[u64; NB_RES]>,
    pub demande_fen: Vec<[u64; NB_RES]>,
    pub servi_fen: Vec<[u64; NB_RES]>,
    pub taux: Vec<[f32; NB_RES]>,
    /// Livraisons par jour et par colonie (la logistique est hors périmètre).
    pub livraison_jour: Vec<[u64; NB_RES]>,
    livraison_reste: Vec<[u64; NB_RES]>,
    pub occ: Vec<u32>,
    pub deces: Vec<u16>,
    pub risque: Vec<[f32; 100]>,
    pub eau_np: Vec<[u32; 100]>,
    pub corps: Vec<Vec<u16>>,
    pub transit: BinaryHeap<Reverse<Transit>>,
    /// Morts de civils : (position, tick) ; lus par le moral des soldats.
    pub evenements: Vec<([f64; 2], u32)>,
    pub stats: StatsCivils,
    pub initial: u64,
}

impl Commun {
    pub fn new(imp: &Implantation, p: ParamsCivils, stocks: [u64; NB_RES], livraison: [u64; NB_RES]) -> Commun {
        let n = imp.lieux.len();
        let mut debut_phase = vec![0u32; p.periode as usize + 1];
        for l in &imp.lieux {
            debut_phase[l.phase as usize + 1] += 1;
        }
        for k in 0..p.periode as usize {
            debut_phase[k + 1] += debut_phase[k];
        }
        let mut lieux_tile = vec![vec![]; imp.tiles.len()];
        for (i, l) in imp.lieux.iter().enumerate() {
            lieux_tile[l.tile as usize].push(i as u32);
        }
        let k = imp.colonies;
        // Stocks : `stocks` est donné pour 100 000 colons, à l'échelle de la
        // colonie.
        let mut habitants = vec![0u64; k];
        for (l, _) in &imp.colons {
            habitants[imp.lieux[*l as usize].colonie as usize] += 1;
        }
        // Médicaments : donnés en doses, comptés en ticks de dose.
        let echelle = |c: usize, v: [u64; NB_RES]| {
            let mut r = v.map(|x| x * habitants[c] / 100_000);
            r[MEDICAMENTS] *= DOSE;
            r
        };
        let stocks_col: Vec<[u64; NB_RES]> = (0..k).map(|c| echelle(c, stocks)).collect();
        // Taux de service avant le premier bilan : rien n'est servi d'un
        // stock vide.
        let taux = stocks_col
            .iter()
            .map(|s| {
                let mut t = [1.0f32; NB_RES];
                for r in 0..EAU_NP {
                    if s[r] == 0 {
                        t[r] = 0.0;
                    }
                }
                t
            })
            .collect();
        Commun {
            pos_lieux: imp.lieux.iter().map(|l| l.pos).collect(),
            lieux: imp.lieux.clone(),
            debut_phase,
            colonies: k,
            tiles: imp.tiles.clone(),
            lieux_tile,
            stocks: stocks_col,
            demande_fen: vec![[0; NB_RES]; k],
            servi_fen: vec![[0; NB_RES]; k],
            taux,
            livraison_jour: (0..k).map(|c| echelle(c, livraison)).collect(),
            livraison_reste: vec![[0; NB_RES]; k],
            occ: vec![0; n],
            deces: vec![0; n],
            risque: vec![[0.5; 100]; imp.tiles.len()],
            eau_np: vec![[0; 100]; imp.tiles.len()],
            corps: vec![vec![0; COTE_CORPS * COTE_CORPS]; imp.tiles.len()],
            transit: BinaryHeap::new(),
            evenements: vec![],
            stats: StatsCivils::default(),
            initial: imp.colons.len() as u64,
            p,
        }
    }

    #[inline(always)]
    pub fn ctx_lieu(&self, l: u32) -> CtxLieu {
        let li = &self.lieux[l as usize];
        CtxLieu {
            lieu: l,
            colonie: li.colonie,
            risque: self.risque[li.tile as usize][li.case as usize],
            chauffe: li.genre != Genre::Chantier && self.p.chauffage,
            exterieur: li.genre == Genre::Chantier,
            deces: self.deces[l as usize],
        }
    }

    pub fn ctx_global(&self, tick: u32) -> CtxGlobal<'_> {
        let s = CtxGlobal::heure(&self.p, tick);
        CtxGlobal {
            p: &self.p,
            k: self.p.coefs(),
            maintenant: tick,
            seconde_jour: s,
            temp_air: CtxGlobal::temperature(&self.p, s),
            taux: &self.taux,
            pos_lieux: &self.pos_lieux,
            graine: hacher(self.p.graine, tick as u64),
        }
    }

    pub fn phase_de(&self, tick: u32) -> std::ops::Range<usize> {
        let k = (tick % self.p.periode) as usize;
        self.debut_phase[k] as usize..self.debut_phase[k + 1] as usize
    }

    pub fn periode_eco(&self) -> u32 {
        self.p.periode.max(TPS)
    }

    /// Un décès : corps dans la carte, événement pour le moral des soldats,
    /// effet sur les occupants du lieu à leur prochaine mise à jour.
    pub fn deces_a(&mut self, lieu: u32, tick: u32) {
        let l = &self.lieux[lieu as usize];
        let (tx, ty) = self.tiles[l.tile as usize];
        let (cx, cy) = (
            ((l.pos[0] - tx as f64 * 1000.0) / 10.0).floor().clamp(0.0, 99.0) as usize,
            ((l.pos[1] - ty as f64 * 1000.0) / 10.0).floor().clamp(0.0, 99.0) as usize,
        );
        let c = &mut self.corps[l.tile as usize][cy * COTE_CORPS + cx];
        *c = c.saturating_add(1);
        self.evenements.push((l.pos, tick));
        self.deces[lieu as usize] = self.deces[lieu as usize].saturating_add(1);
        self.occ[lieu as usize] -= 1;
        self.stats.morts += 1;
        self.stats.malades -= 1;
    }

    /// Sorties d'un tick : départs, décès, compteurs (ordre fixe).
    pub fn appliquer(&mut self, s: &mut Sorties, tick: u32) {
        let phase = self.phase_de(tick);
        for l in phase {
            self.deces[l] = 0;
        }
        for c in 0..self.colonies.min(s.demande.len()) {
            for r in 0..NB_RES {
                self.demande_fen[c][r] += s.demande[c][r];
                self.servi_fen[c][r] += s.servi[c][r];
            }
            self.stats.travail_ticks += s.travail[c];
            self.stats.nouveaux_malades += s.nouveaux_malades[c];
            self.stats.gueris += s.gueris[c];
            self.stats.malades += s.nouveaux_malades[c] as i64 - s.gueris[c] as i64;
        }
        self.stats.mises_a_jour += s.mises_a_jour;
        s.morts.sort_unstable();
        for &(_, lieu) in &s.morts {
            self.deces_a(lieu, tick);
        }
        for &(lieu, n) in &s.eau_np {
            let l = &self.lieux[lieu as usize];
            self.eau_np[l.tile as usize][l.case as usize] += n;
        }
        s.departs.sort_unstable_by_key(|d| d.c.id);
        self.stats.departs += s.departs.len() as u64;
    }

    pub fn partir(&mut self, d: &Depart, avec_colon: bool) {
        self.occ[d.de as usize] -= 1;
        self.transit.push(Reverse(Transit {
            arrivee: d.arrivee,
            id: d.c.id,
            vers: d.vers,
            colon: avec_colon.then_some(d.c),
        }));
    }

    /// Économie : consommation, livraisons, taux de service de la fenêtre
    /// suivante.
    pub fn economie(&mut self, tick: u32) {
        let pe = self.periode_eco();
        if tick % pe != 0 {
            return;
        }
        for c in 0..self.colonies {
            for r in 0..NB_RES {
                let servi = self.servi_fen[c][r];
                let demande = self.demande_fen[c][r];
                self.stats.demande[r] += demande;
                if r == EAU_NP {
                    self.stats.consomme[r] += servi;
                    self.taux[c][r] = 1.0;
                } else {
                    let pris = servi.min(self.stocks[c][r]);
                    self.stocks[c][r] -= pris;
                    self.stats.consomme[r] += pris;
                    self.stats.deficit[r] += servi - pris;
                    // Livraison du dépôt, au prorata de la fenêtre.
                    let l = self.livraison_jour[c][r] * pe as u64 + self.livraison_reste[c][r];
                    self.stocks[c][r] += l / TICKS_JOUR as u64;
                    self.livraison_reste[c][r] = l % TICKS_JOUR as u64;
                    self.taux[c][r] =
                        if demande == 0 {
                            if self.stocks[c][r] > 0 { 1.0 } else { 0.0 }
                        } else {
                            (self.stocks[c][r] as f64 / demande as f64).min(1.0) as f32
                        };
                }
                self.servi_fen[c][r] = 0;
                self.demande_fen[c][r] = 0;
            }
        }
    }

    /// Carte de risque (A-17) : une tile par tick, chacune une fois par
    /// minute. Corps non traités, eau non potable, surpopulation ; léger
    /// flou.
    pub fn risque(&mut self, tick: u32) {
        let mut t = (tick % PERIODE_RISQUE) as usize;
        while t < self.tiles.len() {
            let mut brut = [0.5f32; 100];
            let corps = &self.corps[t];
            for (k, b) in brut.iter_mut().enumerate() {
                let (cx, cy) = (k % 10, k / 10);
                let mut s = 0u32;
                for y in cy * 10..cy * 10 + 10 {
                    for x in cx * 10..cx * 10 + 10 {
                        s += corps[y * COTE_CORPS + x] as u32;
                    }
                }
                *b += s as f32 * 0.05 + self.eau_np[t][k] as f32 * 0.002;
            }
            let mut exces = [0u32; 100];
            let mut cap = [0u32; 100];
            for &l in &self.lieux_tile[t] {
                let li = &self.lieux[l as usize];
                if li.genre == Genre::Chantier {
                    continue;
                }
                exces[li.case as usize] += self.occ[l as usize].saturating_sub(li.capacite);
                cap[li.case as usize] += li.capacite;
            }
            for k in 0..100 {
                if cap[k] > 0 {
                    brut[k] += 4.0 * exces[k] as f32 / cap[k] as f32;
                }
            }
            let r = &mut self.risque[t];
            for k in 0..100 {
                let (cx, cy) = ((k % 10) as i32, (k / 10) as i32);
                let (mut s, mut n) = (0.0, 0.0);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (x, y) = (cx + dx, cy + dy);
                        if (dx, dy) != (0, 0) && (0..10).contains(&x) && (0..10).contains(&y) {
                            s += brut[(y * 10 + x) as usize];
                            n += 1.0;
                        }
                    }
                }
                r[k] = (0.7 * brut[k] + 0.3 * s / n).min(20.0);
            }
            self.eau_np[t] = [0; 100];
            t += PERIODE_RISQUE as usize;
        }
    }

    pub fn somme(&self) -> u64 {
        let mut h = 0u64;
        for (c, s) in self.stocks.iter().enumerate() {
            for (r, v) in s.iter().enumerate() {
                h = h.wrapping_add(hacher(hacher(c as u64, r as u64), *v));
            }
        }
        for (t, c) in self.corps.iter().enumerate() {
            for (k, &v) in c.iter().enumerate() {
                if v != 0 {
                    h = h.wrapping_add(hacher(t as u64, (k as u64) << 16 | v as u64));
                }
            }
        }
        for (t, r) in self.risque.iter().enumerate() {
            for (k, v) in r.iter().enumerate() {
                h = h.wrapping_add(hacher(hacher(t as u64, k as u64), v.to_bits() as u64));
            }
        }
        h = h.wrapping_add(hacher(self.stats.morts, self.stats.zombifies));
        h
    }
}

#[derive(Resource)]
pub struct Horloge(pub u32);

#[derive(Resource)]
pub struct Reglage {
    pub lots: BatchingStrategy,
    pub lot_min: usize,
    pub threads: usize,
}

/// Sorties fusionnées du tick.
#[derive(Resource, Default)]
pub struct Resultat(pub Sorties);

// ---------------------------------------------------------------- B : dossiers

#[derive(Resource)]
pub struct Dossiers(pub Vec<Vec<Colon>>);

/// Met à jour les colons d'un dossier ; ceux qui partent ou meurent en
/// sortent (ordre conservé).
#[inline(always)]
pub fn vivre_dossier(d: &mut Vec<Colon>, l: u32, cm: &Commun, g: &CtxGlobal, s: &mut Sorties) {
    let ctx = cm.ctx_lieu(l);
    d.retain_mut(|c| match vivre(c, &ctx, g, s) {
        Issue::Reste => true,
        Issue::Part { dest, arrivee } => {
            s.departs.push(Depart { c: *c, de: l, vers: dest, arrivee, e: SANS_ENTITE });
            false
        }
        Issue::Meurt => {
            s.morts.push((c.id, l));
            false
        }
    });
    if s.eau_np_n > 0 {
        s.eau_np.push((l, s.eau_np_n));
        s.eau_np_n = 0;
    }
}

/// Découpe une plage de dossiers en lots d'au moins `lot` colons.
pub fn lots_de(d: &[Vec<Colon>], lot: usize) -> Vec<usize> {
    let mut coupes = vec![0];
    let mut n = 0;
    for (i, x) in d.iter().enumerate() {
        n += x.len();
        if n >= lot {
            coupes.push(i + 1);
            n = 0;
        }
    }
    if *coupes.last().unwrap() != d.len() {
        coupes.push(d.len());
    }
    coupes
}

fn maj_b(mut ds: ResMut<Dossiers>, cm: Res<Commun>, h: Res<Horloge>, reg: Res<Reglage>, mut res: ResMut<Resultat>) {
    let t = h.0;
    let plage = cm.phase_de(t);
    let g = cm.ctx_global(t);
    let debut = plage.start;
    let tranche = &mut ds.0[plage];
    let total: usize = tranche.iter().map(|d| d.len()).sum();
    let lot = reg.lot_min.max(total / (4 * reg.threads.max(1)));
    let coupes = lots_de(tranche, lot);
    let colonies = cm.colonies;
    res.0.preparer(colonies);
    if coupes.len() <= 2 {
        let s = &mut res.0;
        for (i, d) in tranche.iter_mut().enumerate() {
            vivre_dossier(d, (debut + i) as u32, &cm, &g, s);
        }
        return;
    }
    let mut morceaux: Vec<(usize, &mut [Vec<Colon>])> = vec![];
    let mut reste = tranche;
    for w in coupes.windows(2) {
        let (a, b) = reste.split_at_mut(w[1] - w[0]);
        morceaux.push((debut + w[0], a));
        reste = b;
    }
    let (cm, g) = (&*cm, &g);
    let mut parts = ComputeTaskPool::get().scope(|sc| {
        for (d0, m) in morceaux {
            sc.spawn(async move {
                let mut s = Sorties::default();
                s.preparer(colonies);
                for (i, d) in m.iter_mut().enumerate() {
                    vivre_dossier(d, (d0 + i) as u32, cm, g, &mut s);
                }
                s
            });
        }
    });
    for p in &mut parts {
        res.0.fusionner(p);
    }
}

// ---------------------------------------------------------------- A : entités

/// Sorties propres à A : changements de partition et décès, par entité.
#[derive(Default)]
pub struct SortiesA {
    pub s: Sorties,
    /// (identifiant, entité, ancienne partition, nouvelle partition).
    pub changes: Vec<(u32, Entity, u8, u8)>,
    pub morts: Vec<(u32, Entity)>,
}

#[derive(Resource, Default)]
pub struct CollecteA(pub Parallel<SortiesA>);

#[derive(Resource, Default)]
pub struct ResultatA {
    pub changes: Vec<(u32, Entity, u8, u8)>,
    pub morts: Vec<(u32, Entity)>,
}

#[inline(always)]
pub fn vivre_entite(
    e: Entity,
    fiche: &mut Fiche,
    place: &mut Place,
    pos: &mut Position,
    cm: &Commun,
    g: &CtxGlobal,
    o: &mut SortiesA,
) {
    let ctx = cm.ctx_lieu(place.lieu);
    let c = &mut fiche.0;
    match vivre(c, &ctx, g, &mut o.s) {
        Issue::Reste => {}
        Issue::Part { dest, arrivee } => {
            o.s.departs.push(Depart { c: *c, de: place.lieu, vers: dest, arrivee, e: e.to_bits() });
            let ancienne = (place.phase % PARTITIONS) as u8;
            place.lieu = dest;
            place.phase = cm.lieux[dest as usize].phase;
            place.arrivee = arrivee;
            pos.0 = cm.pos_lieux[dest as usize];
            let nouvelle = (place.phase % PARTITIONS) as u8;
            if ancienne != nouvelle {
                o.changes.push((c.id, e, ancienne, nouvelle));
            }
        }
        Issue::Meurt => {
            o.s.morts.push((c.id, place.lieu));
            o.morts.push((c.id, e));
        }
    }
    if o.s.eau_np_n > 0 {
        o.s.eau_np.push((place.lieu, o.s.eau_np_n));
        o.s.eau_np_n = 0;
    }
}

type RequeteA<'w, 's, F> = Query<'w, 's, (Entity, &'static mut Fiche, &'static mut Place, &'static mut Position), F>;

fn maj_a<F: bevy_ecs::query::QueryFilter>(
    q: &mut RequeteA<F>,
    cm: &Commun,
    t: u32,
    reg: &Reglage,
    col: &mut CollecteA,
    res: &mut Resultat,
    ra: &mut ResultatA,
) {
    let g = cm.ctx_global(t);
    let k = t % cm.p.periode;
    let colonies = cm.colonies;
    let par = &col.0;
    q.par_iter_mut().batching_strategy(reg.lots.clone()).for_each(|(e, mut fiche, mut place, mut pos)| {
        let pl = place.bypass_change_detection();
        if pl.phase != k || pl.arrivee > t {
            return;
        }
        let mut o = par.borrow_local_mut();
        o.s.preparer(colonies);
        vivre_entite(e, fiche.bypass_change_detection(), pl, pos.bypass_change_detection(), cm, &g, &mut o);
    });
    res.0.preparer(colonies);
    for o in col.0.iter_mut() {
        res.0.fusionner(&mut o.s);
        o.s.vider();
        ra.changes.append(&mut o.changes);
        ra.morts.append(&mut o.morts);
    }
}

#[allow(clippy::too_many_arguments)]
fn maj_a1(
    mut q: RequeteA<()>,
    cm: Res<Commun>,
    h: Res<Horloge>,
    reg: Res<Reglage>,
    mut col: ResMut<CollecteA>,
    mut res: ResMut<Resultat>,
    mut ra: ResMut<ResultatA>,
) {
    maj_a(&mut q, &cm, h.0, &reg, &mut col, &mut res, &mut ra);
}

macro_rules! partitions {
    ($($k:literal),*) => {
        #[allow(clippy::too_many_arguments)]
        fn maj_a2<const K: u8>(
            mut q: RequeteA<With<Part<K>>>,
            cm: Res<Commun>,
            h: Res<Horloge>,
            reg: Res<Reglage>,
            mut col: ResMut<CollecteA>,
            mut res: ResMut<Resultat>,
            mut ra: ResMut<ResultatA>,
        ) {
            if (h.0 % cm.p.periode) % PARTITIONS != K as u32 {
                return;
            }
            maj_a(&mut q, &cm, h.0, &reg, &mut col, &mut res, &mut ra);
        }
        fn systemes_a2(s: &mut Schedule) {
            $( s.add_systems(maj_a2::<$k>); )*
        }
        pub fn inserer_part(e: &mut EntityWorldMut, k: u8) {
            match k { $( $k => { e.insert(Part::<$k>); } )* _ => unreachable!() }
        }
        /// Crée les entités directement dans la table de leur partition.
        pub fn creer_partition(w: &mut World, k: u8, v: Vec<(Fiche, Place, Position)>) {
            match k { $( $k => { w.spawn_batch(v.into_iter().map(|(a, b, c)| (a, b, c, Part::<$k>))); } )* _ => unreachable!() }
        }
        pub fn retirer_part(e: &mut EntityWorldMut, k: u8) {
            match k { $( $k => { e.remove::<Part<$k>>(); } )* _ => unreachable!() }
        }
    };
}
partitions!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19);

// ---------------------------------------------------------------- C : agrégats

/// Agrégat d'un logement : effectifs et moyennes. L'identité, l'expérience
/// et les compétences de chacun sont perdues.
#[derive(Clone, Copy, Default, Debug)]
pub struct Agregat {
    pub n: u32,
    pub malades: u32,
    pub vetements: u32,
    pub couvertures: u32,
    pub actifs: u32,
    pub maj: u32,
    pub j: [f32; 5],
    pub experience: f32,
    /// Compétences : effectif par bit.
    pub competences: [u16; 6],
}

#[derive(Resource)]
pub struct Agregats(pub Vec<Agregat>);

/// Arrondi stochastique déterministe.
#[inline(always)]
fn arrondi(x: f32, h: u64) -> u32 {
    (x + unite(h)).max(0.0) as u32
}

/// Part des actifs en service à l'heure `s` (début uniforme sur
/// l'étalement des prises de poste).
fn part_en_service(p: &ParamsCivils, s: f32) -> f32 {
    let (a, e, d) = (6.0 * 3600.0, p.etalement_poste_h * 3600.0, p.poste_h * 3600.0);
    // Mesure des débuts u ∈ [a, a+e] tels que s ∈ [u, u + d) (modulo un jour).
    let mut tot = 0.0;
    for k in -1..=1 {
        let s2 = s + k as f32 * 86_400.0;
        let lo = (s2 - d).max(a);
        let hi = s2.min(a + e);
        tot += (hi - lo).max(0.0);
    }
    (tot / e).min(1.0)
}

fn vivre_agregat(a: &mut Agregat, l: u32, cm: &Commun, g: &CtxGlobal, s: &mut Sorties) {
    let p = g.p;
    let dt_ticks = g.maintenant.wrapping_sub(a.maj);
    a.maj = g.maintenant;
    if dt_ticks == 0 || a.n == 0 {
        return;
    }
    s.mises_a_jour += 1;
    let dt = dt_ticks as f32 / TPS as f32;
    let h0 = hacher(g.graine, l as u64 | 1 << 40);
    let ctx = cm.ctx_lieu(l);
    let col = ctx.colonie as usize;
    let sec = g.seconde_jour;
    let n = a.n as f32;
    let sains = (a.n - a.malades) as f32;
    let au_travail = a.actifs as f32 * (sains / n) * part_en_service(p, sec);
    let dort = endormi(sec);
    let eveilles = if dort { 0.0 } else { n };
    let mut faim = a.j[FAIM] + dt / (p.faim_h * 3600.0);
    let mut soif = a.j[SOIF] + dt / (p.soif_h * 3600.0);
    // Repas : chacun mange quand sa faim dépasse le seuil ; en moyenne, un
    // repas toutes les `faim_h × repas` heures d'éveil.
    let repas = arrondi(eveilles * dt / (p.faim_h * 3600.0 * p.repas), hacher(h0, 1));
    let servi = arrondi(repas as f32 * g.taux[col][VIVRES], hacher(h0, 2));
    s.demande[col][VIVRES] += repas as u64;
    s.servi[col][VIVRES] += servi as u64;
    faim -= servi as f32 * p.repas / n;
    let prises = arrondi(eveilles * dt / (p.soif_h * 3600.0 * p.boire), hacher(h0, 3));
    let eau = arrondi(prises as f32 * g.taux[col][EAU], hacher(h0, 4));
    s.demande[col][EAU] += prises as u64;
    s.servi[col][EAU] += eau as u64;
    s.demande[col][EAU_NP] += (prises - eau) as u64;
    s.servi[col][EAU_NP] += (prises - eau) as u64;
    soif -= prises as f32 * p.boire / n;
    let mut fatigue = a.j[FATIGUE];
    if dort {
        fatigue -= dt / (p.recup_h * 3600.0);
    } else {
        let part = au_travail / n;
        fatigue += dt * (part / (p.fatigue_travail_h * 3600.0) + (1.0 - part) / (p.fatigue_eveil_h * 3600.0));
    }
    // Froid moyen : part équipée.
    let mut t = g.temp_air + if ctx.chauffe { 15.0 } else { 5.0 };
    t += 7.0 * a.vetements as f32 / n;
    if dort {
        t += 8.0 * a.couvertures as f32 / n;
    }
    let fr = ((12.0 - t) / 20.0).clamp(0.0, 1.0);
    if fr > 0.0 && a.vetements < a.n {
        let d = a.n - a.vetements;
        let v = arrondi(d as f32 * g.taux[col][VETEMENTS], hacher(h0, 5)).min(d);
        s.demande[col][VETEMENTS] += d as u64;
        s.servi[col][VETEMENTS] += v as u64;
        a.vetements += v;
    }
    if dort && fr > 0.0 && a.couvertures < a.n {
        let d = a.n - a.couvertures;
        let v = arrondi(d as f32 * g.taux[col][COUVERTURES], hacher(h0, 6)).min(d);
        s.demande[col][COUVERTURES] += d as u64;
        s.servi[col][COUVERTURES] += v as u64;
        a.couvertures += v;
    }
    let jour = dt / 86_400.0;
    let chance = p.maladie_jour * jour * ctx.risque * (1.0 + p.poids_froid * fr) * (1.0 + faim + fatigue);
    let nouveaux = arrondi(sains * chance, hacher(h0, 7)).min(a.n - a.malades);
    let m = a.malades as f32;
    let soignes = arrondi(m * g.taux[col][MEDICAMENTS], hacher(h0, 8)).min(a.malades);
    s.demande[col][MEDICAMENTS] += a.malades as u64 * dt_ticks as u64;
    s.servi[col][MEDICAMENTS] += soignes as u64 * dt_ticks as u64;
    let non = (a.malades - soignes) as f32;
    let gueris = (arrondi(soignes as f32 * p.guerison_jour * jour, hacher(h0, 9))
        + arrondi(non * p.guerison_seul_jour * jour, hacher(h0, 10)))
    .min(a.malades);
    let morts = arrondi(non * p.mort_jour * jour, hacher(h0, 11)).min(a.malades - gueris);
    a.malades = a.malades + nouveaux - gueris - morts;
    s.nouveaux_malades[col] += nouveaux as u64;
    s.gueris[col] += gueris as u64;
    for k in 0..morts {
        s.morts.push((hacher(l as u64, (g.maintenant as u64) << 8 | k as u64) as u32, l));
    }
    if morts > 0 {
        let fv = a.vetements as f32 / n;
        let fc = a.couvertures as f32 / n;
        a.n -= morts;
        a.vetements = (a.n as f32 * fv).round() as u32;
        a.couvertures = (a.n as f32 * fc).round() as u32;
        a.actifs = a.actifs.min(a.n);
    }
    let cible = (1.0 - 0.4 * faim - 0.3 * soif - 0.2 * fatigue - 0.3 * fr - 0.2 * m / n).clamp(0.0, 1.0);
    let k = (dt / (p.moral_h * 3600.0)).min(1.0);
    a.j[MORAL] = (a.j[MORAL] + (cible - a.j[MORAL]) * k - p.moral_deces * ctx.deces as f32).clamp(0.0, 1.0);
    a.j[USURE] = (a.j[USURE] + p.usure_deces * ctx.deces as f32 - dt / (p.usure_recup_h * 3600.0)).clamp(0.0, 1.0);
    a.j[FAIM] = faim.clamp(0.0, 1.0);
    a.j[SOIF] = soif.clamp(0.0, 1.0);
    a.j[FATIGUE] = fatigue.clamp(0.0, 1.0);
    s.travail[col] += (au_travail * dt_ticks as f32) as u64;
    a.experience += au_travail / n * dt / 60.0;
}

fn maj_c(mut ag: ResMut<Agregats>, cm: Res<Commun>, h: Res<Horloge>, mut res: ResMut<Resultat>) {
    let t = h.0;
    let plage = cm.phase_de(t);
    let g = cm.ctx_global(t);
    res.0.preparer(cm.colonies);
    let debut = plage.start;
    for (i, a) in ag.0[plage].iter_mut().enumerate() {
        vivre_agregat(a, (debut + i) as u32, &cm, &g, &mut res.0);
    }
}

// ---------------------------------------------------------------- Le monde

pub struct Civils {
    pub world: World,
    pub repr: Repr,
    sched: Schedule,
    pub tick: u32,
    pub lot_min: usize,
    /// Temps cumulés : arrivées, mises à jour (schedule), après.
    pub chronos: [std::time::Duration; 3],
}

pub struct ConfigCivils {
    pub repr: Repr,
    /// Exécuteur multi-thread de Bevy (sinon : mono-thread ; le système de
    /// mise à jour lance lui-même ses tâches parallèles).
    pub multi: bool,
    /// Individualisation possible (B seulement) : ajoute la mise à jour des
    /// colons individualisés.
    pub individus: bool,
    pub threads: usize,
    pub lot_min: usize,
    pub stocks: [u64; NB_RES],
    pub livraison: [u64; NB_RES],
}

impl Civils {
    pub fn new(imp: &Implantation, p: ParamsCivils, c: &ConfigCivils) -> Civils {
        bevy_tasks::ComputeTaskPool::get_or_init(|| {
            bevy_tasks::TaskPoolBuilder::new().num_threads(c.threads).build()
        });
        let mut world = World::new();
        let mut cm = Commun::new(imp, p, c.stocks, c.livraison);
        for (l, _) in &imp.colons {
            cm.occ[*l as usize] += 1;
        }
        world.insert_resource(Horloge(0));
        world.insert_resource(Reglage {
            lots: BatchingStrategy::new().min_batch_size(c.lot_min.max(1)),
            lot_min: c.lot_min.max(1),
            threads: c.threads,
        });
        world.insert_resource(Resultat::default());
        world.insert_resource(ResultatA::default());
        world.insert_resource(CollecteA::default());
        let mut sched = Schedule::default();
        match c.repr {
            Repr::B => {
                let mut d: Vec<Vec<Colon>> = vec![vec![]; imp.lieux.len()];
                for (l, col) in &imp.colons {
                    d[*l as usize].push(*col);
                }
                world.insert_resource(Dossiers(d));
                sched.add_systems(maj_b);
                if c.individus {
                    world.insert_resource(crate::individus::CollecteInd::default());
                    world.insert_resource(crate::individus::ResultatInd::default());
                    sched.add_systems(crate::individus::maj_individus);
                }
            }
            Repr::A1 | Repr::A2 => {
                let lieux = &cm.lieux;
                if c.repr == Repr::A1 {
                    world.spawn_batch(imp.colons.iter().map(|(l, col)| {
                        (
                            Fiche(*col),
                            Place { lieu: *l, phase: lieux[*l as usize].phase, arrivee: 0 },
                            Position(lieux[*l as usize].pos),
                        )
                    }));
                    sched.add_systems(maj_a1);
                } else {
                    // Création directe dans la bonne table (une par partition).
                    for k in 0..PARTITIONS as u8 {
                        let v: Vec<(Fiche, Place, Position)> = imp
                            .colons
                            .iter()
                            .filter(|(l, _)| (lieux[*l as usize].phase % PARTITIONS) as u8 == k)
                            .map(|(l, col)| {
                                (
                                    Fiche(*col),
                                    Place { lieu: *l, phase: lieux[*l as usize].phase, arrivee: 0 },
                                    Position(lieux[*l as usize].pos),
                                )
                            })
                            .collect();
                        creer_partition(&mut world, k, v);
                    }
                    systemes_a2(&mut sched);
                }
            }
            Repr::C => {
                let mut ag = vec![Agregat::default(); imp.lieux.len()];
                for (l, col) in &imp.colons {
                    let a = &mut ag[*l as usize];
                    let n = a.n as f32;
                    for k in 0..5 {
                        a.j[k] = (a.j[k] * n + f(col.j[k])) / (n + 1.0);
                    }
                    a.experience = (a.experience * n + col.experience as f32) / (n + 1.0);
                    for b in 0..6 {
                        if col.competences & (1 << b) != 0 {
                            a.competences[b] += 1;
                        }
                    }
                    a.n += 1;
                    if col.travail != NUL {
                        a.actifs += 1;
                    }
                }
                world.insert_resource(Agregats(ag));
                sched.add_systems(maj_c);
            }
        }
        world.insert_resource(cm);
        if c.multi {
            sched.set_executor(MultiThreadedExecutor::new());
        } else {
            sched.set_executor(bevy_ecs::schedule::SingleThreadedExecutor::new());
        }
        sched.initialize(&mut world).expect("initialisation");
        Civils { world, repr: c.repr, sched, tick: 0, lot_min: c.lot_min, chronos: Default::default() }
    }

    pub fn commun(&self) -> &Commun {
        self.world.resource::<Commun>()
    }

    /// Un tick : arrivées, mises à jour de la phase, départs et décès,
    /// économie, carte de risque.
    pub fn tick(&mut self) {
        let t = self.tick;
        self.world.resource_mut::<Horloge>().0 = t;
        let t0 = std::time::Instant::now();
        if self.world.contains_resource::<crate::individus::Indiv>() {
            crate::individus::avant(self, t);
        }
        self.arrivees(t);
        let t1 = std::time::Instant::now();
        self.sched.run(&mut self.world);
        let t2 = std::time::Instant::now();
        self.apres(t);
        let t3 = std::time::Instant::now();
        self.chronos[0] += t1 - t0;
        self.chronos[1] += t2 - t1;
        self.chronos[2] += t3 - t2;
        self.tick += 1;
        self.world.clear_trackers();
    }

    pub fn arrivees(&mut self, t: u32) {
        let w = &mut self.world;
        let repr = self.repr;
        let mut a_creer: Vec<(Colon, u32)> = vec![];
        w.resource_scope(|w, mut cm: Mut<Commun>| {
            while cm.transit.peek().is_some_and(|x| x.0.arrivee <= t) {
                let Reverse(x) = cm.transit.pop().unwrap();
                cm.occ[x.vers as usize] += 1;
                if repr == Repr::B
                    && let Some(c) = x.colon
                {
                    let ind = w
                        .get_resource::<crate::individus::Indiv>()
                        .is_some_and(|i| i.accueille_individus(x.vers));
                    if ind {
                        a_creer.push((c, x.vers));
                    } else {
                        w.resource_mut::<Dossiers>().0[x.vers as usize].push(c);
                    }
                }
            }
        });
        if !a_creer.is_empty() {
            crate::individus::creer(w, &a_creer, t);
        }
    }

    pub fn apres(&mut self, t: u32) {
        let mut s = std::mem::take(&mut self.world.resource_mut::<Resultat>().0);
        let mut ra = std::mem::take(&mut *self.world.resource_mut::<ResultatA>());
        let mut morts_ind = vec![];
        if let Some(mut ri) = self.world.get_resource_mut::<crate::individus::ResultatInd>() {
            s.fusionner(&mut ri.s);
            ri.s.vider();
            morts_ind = std::mem::take(&mut ri.morts);
        }
        self.world.resource_mut::<Commun>().appliquer(&mut s, t);
        let avec = self.repr == Repr::B;
        for d in &s.departs {
            if d.e == SANS_ENTITE || self.repr != Repr::B {
                self.world.resource_mut::<Commun>().partir(d, avec);
            } else {
                crate::individus::depart(self, d, t);
            }
        }
        morts_ind.sort_unstable_by_key(|x: &(u32, Entity)| x.0);
        for (_, e) in morts_ind {
            crate::individus::mort(&mut self.world, e);
        }
        {
            let mut cm = self.world.resource_mut::<Commun>();
            cm.economie(t);
            cm.risque(t);
        }
        if matches!(self.repr, Repr::A1 | Repr::A2) {
            ra.changes.sort_unstable_by_key(|x| x.0);
            if self.repr == Repr::A2 {
                for &(_, e, a, b) in &ra.changes {
                    let mut em = self.world.entity_mut(e);
                    retirer_part(&mut em, a);
                    inserer_part(&mut em, b);
                }
            }
            ra.morts.sort_unstable_by_key(|x| x.0);
            for &(_, e) in &ra.morts {
                self.world.despawn(e);
            }
            ra.changes.clear();
            ra.morts.clear();
        }
        s.vider();
        self.world.resource_mut::<Resultat>().0 = s;
        *self.world.resource_mut::<ResultatA>() = ra;
    }

    /// Colons vivants (lieux, entités, trajets).
    pub fn vivants(&mut self) -> u64 {
        match self.repr {
            Repr::B => {
                let d: u64 = self.world.resource::<Dossiers>().0.iter().map(|x| x.len() as u64).sum();
                let e = self.world.query::<&Fiche>().iter(&self.world).count() as u64;
                d + e + self.commun().transit.iter().filter(|x| x.0.colon.is_some()).count() as u64
            }
            Repr::A1 | Repr::A2 => self.world.query::<&Fiche>().iter(&self.world).count() as u64,
            Repr::C => self.world.resource::<Agregats>().0.iter().map(|a| a.n as u64).sum(),
        }
    }

    /// Conservation : vivants + morts + zombifiés = initial.
    pub fn conservation(&mut self) -> bool {
        let v = self.vivants();
        let cm = self.commun();
        v + cm.stats.morts + cm.stats.zombifies == cm.initial
    }

    /// Aucun doublon : chaque identifiant présent au plus une fois (A, B).
    pub fn identifiants(&mut self) -> Vec<u32> {
        let mut v: Vec<u32> = match self.repr {
            Repr::B => {
                let mut v: Vec<u32> =
                    self.world.resource::<Dossiers>().0.iter().flat_map(|d| d.iter().map(|c| c.id)).collect();
                v.extend(self.commun().transit.iter().filter_map(|x| x.0.colon.map(|c| c.id)));
                v.extend(self.world.query::<&Fiche>().iter(&self.world).map(|f| f.0.id));
                v
            }
            Repr::A1 | Repr::A2 => self.world.query::<&Fiche>().iter(&self.world).map(|f| f.0.id).collect(),
            Repr::C => vec![],
        };
        v.sort_unstable();
        v
    }

    /// Somme de contrôle de tout l'état, insensible à l'ordre de stockage.
    pub fn somme(&mut self) -> u64 {
        let mut h = self.commun().somme();
        match self.repr {
            Repr::B => {
                for (l, d) in self.world.resource::<Dossiers>().0.iter().enumerate() {
                    for c in d {
                        h = h.wrapping_add(empreinte(c, l as u32));
                    }
                }
                for x in self.commun().transit.iter() {
                    if let Some(c) = &x.0.colon {
                        h = h.wrapping_add(empreinte(c, x.0.vers));
                    }
                }
                let mut q = self.world.query::<(&Fiche, &Place)>();
                for (f, p) in q.iter(&self.world) {
                    h = h.wrapping_add(empreinte(&f.0, p.lieu));
                }
            }
            Repr::A1 | Repr::A2 => {
                let mut q = self.world.query::<(&Fiche, &Place)>();
                for (f, p) in q.iter(&self.world) {
                    h = h.wrapping_add(empreinte(&f.0, p.lieu));
                }
            }
            Repr::C => {
                for (l, a) in self.world.resource::<Agregats>().0.iter().enumerate() {
                    let mut x = hacher(l as u64, (a.n as u64) << 32 | a.malades as u64);
                    for v in a.j {
                        x = hacher(x, v.to_bits() as u64);
                    }
                    h = h.wrapping_add(x);
                }
            }
        }
        h
    }

    pub fn malades(&mut self) -> u64 {
        match self.repr {
            Repr::B => {
                let d = self.world.resource::<Dossiers>().0.iter().flatten().filter(|c| c.malade != 0).count() as u64;
                let t = self.commun().transit.iter().filter(|x| x.0.colon.is_some_and(|c| c.malade != 0)).count() as u64;
                let e = self.world.query::<&Fiche>().iter(&self.world).filter(|f| f.0.malade != 0).count() as u64;
                d + t + e
            }
            Repr::A1 | Repr::A2 => {
                self.world.query::<&Fiche>().iter(&self.world).filter(|f| f.0.malade != 0).count() as u64
            }
            Repr::C => self.world.resource::<Agregats>().0.iter().map(|a| a.malades as u64).sum(),
        }
    }
}
