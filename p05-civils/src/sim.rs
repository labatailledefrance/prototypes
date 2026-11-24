//! Simulation d'un bloc de tiles avec activation et désactivation (P-04).
//!
//! Reprend la simulation de P-03 (arbre k-d des humains, relais filtré,
//! veille, veille profonde, flow fields de 2 m étalés, grille dense des
//! zombies en u16, réservation exacte partout) et la rend **dynamique** :
//!
//! - chaque tile du bloc est un **stock** (compteur jamais activé, ou
//!   sauvegarde sur disque) ou un **slot** actif, pris dans un pool réutilisé
//!   (aucune allocation en régime établi) ;
//! - une tile requise (humains dans elle ou ses 8 voisines) passe par un
//!   **chantier d'activation**, étalé sur plusieurs ticks dans la phase des
//!   champs : zone (si non précalculée), terrain de 2 m avec halo, placement
//!   ou restauration des zombies ; puis les zombies naissent (phase de
//!   gestion) ;
//! - une tile qui n'est plus requise depuis le délai est **sauvegardée**
//!   (instantané normalisé, écriture en tâche de fond) et ses entités
//!   détruites ;
//! - les zombies **changent de tile** (phase des frontières) ; la
//!   séparation, le relais, la vue, la capacité et la réservation
//!   traversent les frontières ; les champs échangent leurs bordures ;
//! - les tiles inertes **migrent** (journal rejoué à l'activation).
//!
//! Un tick = huit phases chronométrées : gestion (exclusive), humains,
//! grilles, champs (et chantiers), perception, déplacement, admission,
//! frontières (exclusive). Chaque phase parallèle ne lit que l'état de la
//! précédente ; les phases exclusives traitent tout dans un ordre fixe :
//! le résultat ne dépend ni du nombre de threads, ni du découpage en lots.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bevy_ecs::batching::BatchingStrategy;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::MultiThreadedExecutor;
use bevy_tasks::{ComputeTaskPool, TaskPoolBuilder};

use crate::alea::{hacher, unite};
use crate::carte::{self, Carte, Geo, bit};
use crate::champ::{self, Champ, ChantierTerrain, Dial, Echange, Graine, Regle, Terrain};
use crate::humains::Humains;
use crate::index::{Compte, Index, Proximite, RAYONS_MAX, Requete, Structure};
use crate::sauvegarde::{self, COTE_CORPS, Cote, Format, Mouvement, TuileSauvee, ZSauve};
use crate::ville::{COTE, COTE_F, Ville};
use crate::voisinage::{self, Elem, Voisinage};

/// Pas de temps : 20 ticks par seconde de jeu.
pub const DT: f32 = 0.05;
const CASE_PROXIMITE: f32 = 25.0;
const VOISINS_MAX: u32 = 16;
const APPROCHE: f32 = 15.0;
/// Humains des tiles voisines vus par une tile : jusqu'à 500 m au-delà du
/// bord (au moins la vue et le seuil de veille profonde).
pub const HALO: f32 = 500.0;
/// Zombies au plus par slot (rang sur 16 bits, grille en u16).
pub const CAP: usize = 65_000;
/// Slots au plus (6 bits dans la clé de réservation).
pub const SLOTS_MAX: usize = 64;
/// Événements du journal de migration rejoués par tick.
const REJEU_PAR_TICK: usize = 4;

pub const ERRANCE: u8 = 0;
pub const POURSUITE: u8 = 1;
pub const RELAIS: u8 = 3;
pub const PERDU: u8 = 4;
pub const PROFOND: u8 = 5;
/// P-05 : entré dans un bâtiment occupé (hors de la grille, immobile ;
/// combat auto-résolu).
pub const DEDANS: u8 = 6;

/// Paramètres : **hypothèses de prototype**, pas statistiques de jeu.
#[derive(Clone)]
pub struct Params {
    pub vitesse_course: f32,
    pub vitesse_errance: f32,
    pub rayon_ouie: f32,
    pub distance_vue: f32,
    pub demi_angle_vue_deg: f32,
    pub distance_separation: f32,
    pub capacite_case: u32,
    pub periode_perception: u32,
    pub rayons: usize,
    pub relais: bool,
    pub filtre_relais: bool,
    pub rayon_relais: f32,
    pub fraicheur_alerte: u32,
    pub seuil_source: u32,
    pub veille: bool,
    pub periode_veille: u32,
    pub pas_veille: u32,
    pub profonde: bool,
    pub seuil_profond: f32,
    pub duree_derniere_position: u32,
    pub periode_champ: u32,
    pub etalement: bool,
    // ---- P-04
    /// Délai de désactivation (ticks).
    pub delai: u64,
    /// Chantier d'activation étalé (sinon : tout dans un seul tick).
    pub etalement_activation: bool,
    /// Terrain : unités par tick (une unité = une case de 1 m lue).
    pub budget_terrain: u64,
    /// Placement : zombies par tick.
    pub budget_placement: usize,
    /// Naissance des entités : par tick (0 = toutes d'un coup).
    pub lot_naissance: usize,
    /// Destruction des entités : par tick (0 = toutes d'un coup).
    pub lot_destruction: usize,
    /// Désactivations au plus par tick (0 : sans limite).
    pub desactivations_par_tick: usize,
    /// Zone accessible précalculée par la chaîne de données (sinon,
    /// calculée à l'activation, sur la tile seule).
    pub zone_precalculee: bool,
    /// Bord de la zone active réfléchissant pour les zombies sans cible
    /// (errants, veille profonde, dernière position) : flux net nul vers
    /// les tiles inertes, dont les zombies ne bougent pas.
    pub bord_reflechi: bool,
    pub format: Format,
    pub fsync: bool,
    pub dossier: PathBuf,
    pub vitesse_promenade: f64,
    pub portee_tir: f32,
    pub periode_tir: u32,
    pub proba_tir: f32,
    pub pv_barricade: i32,
    pub periode_migration: u64,
    pub taux_migration: f32,
    /// Une tile qui compte déjà autant de zombies ne reçoit plus de
    /// migrants (capacité d'un slot : CAP).
    pub saturation_migration: u32,
    /// Entités créées puis détruites au démarrage, pour que les tables de
    /// l'ECS aient d'emblée leur taille maximale (pas de réallocation en jeu).
    pub prechauffe_entites: usize,
    /// Stimuli de migration (fumée d'incinérateur) : position Lambert-93,
    /// portée (m).
    pub stimuli: Vec<([f64; 2], f64)>,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            vitesse_course: 5.0,
            vitesse_errance: 1.0,
            rayon_ouie: 40.0,
            distance_vue: 400.0,
            demi_angle_vue_deg: 60.0,
            distance_separation: 0.7,
            capacite_case: 24,
            periode_perception: 5,
            rayons: 4,
            relais: true,
            filtre_relais: true,
            rayon_relais: 3.0,
            fraicheur_alerte: 60,
            seuil_source: 5,
            veille: true,
            periode_veille: 20,
            pas_veille: 4,
            profonde: true,
            seuil_profond: 500.0,
            duree_derniere_position: 1200,
            periode_champ: 20,
            etalement: true,
            delai: 1200,
            etalement_activation: true,
            budget_terrain: 100_000,
            budget_placement: 5_000,
            lot_naissance: 2_000,
            lot_destruction: 2_000,
            desactivations_par_tick: 1,
            zone_precalculee: true,
            bord_reflechi: true,
            format: Format::Zstd,
            fsync: false,
            dossier: PathBuf::from("sauvegardes"),
            vitesse_promenade: 1.5,
            portee_tir: 25.0,
            periode_tir: 20,
            proba_tir: 0.5,
            pv_barricade: 6_000,
            periode_migration: 1200,
            taux_migration: 0.02,
            saturation_migration: 40_000,
            prechauffe_entites: 0,
            stimuli: vec![],
        }
    }
}

// ---------------------------------------------------------------- Composants

#[derive(Component)]
pub struct Corps {
    pub pos: [f64; 2],
    pub cap: f32,
}

#[derive(Component)]
pub struct Cerveau {
    pub etat: u8,
    /// Coordonnées locales à la tile du zombie.
    pub cible: [f32; 2],
    pub alerte: u32,
    pub perdu_depuis: u32,
}

#[derive(Component, Clone, Copy)]
pub struct Ident {
    pub slot: u32,
    pub rang: u32,
}

// ---------------------------------------------------------------- Slots

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EtatSlot {
    Libre,
    Chantier,
    Actif,
    /// Tile sauvegardée, entités en cours de destruction.
    Destruction,
}

/// Données d'une tile restaurée : en mémoire (écriture pas finie), ou à
/// relire sur disque.
pub enum Donnees {
    Memoire(Arc<TuileSauvee>, Option<JoinHandle<Ecrit>>),
    Disque(PathBuf),
}

pub struct Ecrit {
    pub octets: usize,
    pub encode_ns: u64,
    pub ecriture_ns: u64,
}

pub enum Source {
    Compteur(u32),
    Memoire(Arc<TuileSauvee>),
    Lecture(Option<JoinHandle<std::io::Result<(Vec<u8>, u64)>>>),
}

/// Chantier d'activation d'une tile.
pub struct Chantier {
    pub tile: usize,
    pub debut: u64,
    /// 0 zone, 1 terrain, 2 zombies, 3 naissance.
    pub etape: u8,
    source: Option<Source>,
    /// Effectif attendu (compteur, ou sauvegarde + journal).
    pub n_source: u32,
    somme_attendue: u64,
    journal: Vec<Mouvement>,
    /// Événements du journal déjà rejoués.
    rejeu: usize,
    /// Zombies entrés à pied pendant le chantier (position locale, cap).
    pub attente: Vec<([f32; 2], f32)>,
    /// Zombies à faire naître (position locale, cap).
    pub liste: Vec<([f32; 2], f32)>,
    pub naissance: usize,
    zone_locale: Option<Vec<u64>>,
    corps: Option<Vec<u16>>,
    pub restauration: bool,
    pub controle_ok: Option<bool>,
    pub constructions_ok: Option<bool>,
    pub ns: u64,
    pub ns_max: u64,
    pub ticks_travail: u32,
    pub ns_etapes: [u64; 3],
    pub lecture_ns: u64,
    pub decode_ns: u64,
    pub naissance_ns: u64,
}

impl Chantier {
    fn compte(&self) -> usize {
        (if self.etape < 3 { self.n_source as usize } else { self.liste.len() }) + self.attente.len()
    }
}

pub struct Slot {
    pub tile: usize,
    pub origine: [f64; 2],
    pub etat: EtatSlot,
    pub chantier: Option<Box<Chantier>>,
    pub terrain: Terrain,
    ch_terrain: ChantierTerrain,
    pub proies: Champ,
    proies_suivant: Champ,
    sources_suivantes: Vec<Graine>,
    sources_proies: Vec<Graine>,
    budget: u64,
    ns_cours: u64,
    dial: Dial,
    export_pret: Option<Echange>,
    destructions: Vec<(u16, u16)>,
    pub hpos: Vec<[f32; 2]>,
    hid: Vec<u32>,
    index: Index,
    proximite: Proximite,
    pub n_haut: usize,
    libres: BinaryHeap<Reverse<u32>>,
    pub vivants: usize,
    pub entites: Vec<Option<Entity>>,
    pub inst: Vec<AtomicU64>,
    horodatage: Vec<AtomicU32>,
    cible_al: Vec<AtomicU64>,
    pub indexe: Vec<AtomicU8>,
    pub abattu: Vec<AtomicU8>,
    souhait: Vec<AtomicU64>,
    attente: Vec<AtomicU8>,
    demandes: Vec<AtomicU64>,
    nb_demandes: AtomicU32,
    sortants: Vec<AtomicU32>,
    nb_sortants: AtomicU32,
    pub vois: Voisinage,
    pub corps: Vec<u16>,
    a_detruire: Vec<Entity>,
    destruction_ns: u64,
    pub stat: StatChamps,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct StatChamps {
    pub recalculs: u64,
    pub ns: u64,
    pub max_ns: u64,
}

fn atomes<T: Default>(n: usize) -> Vec<T> {
    (0..n).map(|_| T::default()).collect()
}

impl Slot {
    fn new(_p: &Params) -> Slot {
        let terrain = Terrain::vide(2, Regle::Majorite, true);
        Slot {
            tile: usize::MAX,
            origine: [0.0; 2],
            etat: EtatSlot::Libre,
            chantier: None,
            proies: Champ::new(&terrain),
            proies_suivant: Champ::new(&terrain),
            sources_suivantes: vec![],
            sources_proies: vec![],
            budget: u64::MAX,
            ns_cours: 0,
            dial: Dial::new(&terrain),
            terrain,
            ch_terrain: ChantierTerrain::default(),
            export_pret: None,
            destructions: vec![],
            hpos: vec![],
            hid: vec![],
            index: Index::new(Structure::Kdtree, 25.0),
            proximite: Proximite::avec_etendue(CASE_PROXIMITE, -HALO, COTE_F + 2.0 * HALO),
            n_haut: 0,
            libres: BinaryHeap::new(),
            vivants: 0,
            entites: vec![],
            inst: atomes(CAP),
            horodatage: atomes(CAP),
            cible_al: atomes(CAP),
            indexe: atomes(CAP),
            abattu: atomes(CAP),
            souhait: atomes(CAP),
            attente: atomes(CAP),
            demandes: atomes(CAP),
            nb_demandes: AtomicU32::new(0),
            sortants: atomes(CAP),
            nb_sortants: AtomicU32::new(0),
            vois: Voisinage::new(voisinage::Structure::Dense16, COTE_F, 2.0, CAP),
            corps: vec![0; COTE_CORPS * COTE_CORPS],
            a_detruire: vec![],
            destruction_ns: 0,
            stat: StatChamps::default(),
        }
    }

    /// Prépare le slot pour un chantier (réutilisation sans allocation).
    fn preparer(&mut self, tile: usize, origine: [f64; 2], ch: Chantier) {
        self.tile = tile;
        self.origine = origine;
        self.etat = EtatSlot::Chantier;
        self.chantier = Some(Box::new(ch));
        self.ch_terrain = ChantierTerrain::default();
        self.proies.vide = true;
        self.proies_suivant.vide = true;
        self.sources_proies.clear();
        self.sources_suivantes.clear();
        self.dial.abandonner();
        self.budget = u64::MAX;
        self.export_pret = None;
        self.destructions.clear();
        self.n_haut = 0;
        self.libres.clear();
        self.vivants = 0;
        self.entites.clear();
        self.nb_demandes.store(0, Relaxed);
        self.nb_sortants.store(0, Relaxed);
        self.corps.fill(0);
        self.stat = StatChamps::default();
    }

    #[inline(always)]
    fn local(&self, p: [f64; 2]) -> [f32; 2] {
        [(p[0] - self.origine[0]) as f32, (p[1] - self.origine[1]) as f32]
    }

    fn allouer(&mut self) -> usize {
        let r = match self.libres.pop() {
            Some(Reverse(r)) => r as usize,
            None => {
                self.n_haut += 1;
                self.entites.push(None);
                self.n_haut - 1
            }
        };
        assert!(self.n_haut <= CAP, "slot plein");
        r
    }

    fn liberer(&mut self, r: usize) {
        self.indexe[r].store(0, Relaxed);
        self.abattu[r].store(0, Relaxed);
        self.horodatage[r].store(0, Relaxed);
        self.entites[r] = None;
        self.libres.push(Reverse(r as u32));
        self.vivants -= 1;
    }

    fn installer(&mut self, r: usize, e: Entity, pos: [f32; 2], etat: u8) {
        self.entites[r] = Some(e);
        self.inst[r].store(coder(pos), Relaxed);
        self.horodatage[r].store(0, Relaxed);
        self.cible_al[r].store(0, Relaxed);
        self.indexe[r].store((etat != PROFOND && etat != DEDANS) as u8, Relaxed);
        self.abattu[r].store(0, Relaxed);
        self.attente[r].store(0, Relaxed);
        self.vivants += 1;
    }
}

#[derive(Resource)]
pub struct Slots(pub Vec<Slot>);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EtatTuile {
    Inerte,
    Chantier(u32),
    Actif(u32),
}

#[derive(Resource)]
pub struct Topo {
    pub geo: Geo,
    pub etat: Vec<EtatTuile>,
    /// Par slot : slots actifs voisins, `[dy + 1][dx + 1]`.
    pub voisines: Vec<[[Option<u32>; 3]; 3]>,
}

#[derive(Resource)]
pub struct Bordures(pub Vec<Option<Echange>>);

#[derive(Resource)]
struct Horloge(u64);

#[derive(Resource)]
struct Reglages {
    p: Params,
    cos_demi_angle: f32,
    sin_demi_angle: f32,
    lots: BatchingStrategy,
    capacite: u32,
}

#[inline(always)]
pub fn coder(p: [f32; 2]) -> u64 {
    (p[0].to_bits() as u64) << 32 | p[1].to_bits() as u64
}
#[inline(always)]
pub fn decoder(v: u64) -> [f32; 2] {
    [f32::from_bits((v >> 32) as u32), f32::from_bits(v as u32)]
}
#[inline(always)]
pub fn d2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}
#[inline(always)]
fn dedans(p: [f32; 2]) -> bool {
    p[0] >= 0.0 && p[1] >= 0.0 && p[0] < COTE_F && p[1] < COTE_F
}

const VOISINS8: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

// ------------------------------------------------- Utilitaires de P-03

/// Case libre la plus proche de `p` dans la plus grande zone connexe de
/// rues de la tile seule (P-03).
pub fn centre_accessible(v: &Ville, p: [f32; 2]) -> [f32; 2] {
    let mut zone = vec![u32::MAX; COTE * COTE];
    let (mut meilleure, mut taille_max) = (u32::MAX, 0usize);
    let mut file = std::collections::VecDeque::new();
    let mut n = 0u32;
    for depart in 0..COTE * COTE {
        let (x0, y0) = ((depart % COTE) as i32, (depart / COTE) as i32);
        if zone[depart] != u32::MAX || v.bati(x0, y0) {
            continue;
        }
        zone[depart] = n;
        file.push_back((x0, y0));
        let mut taille = 0usize;
        while let Some((x, y)) = file.pop_front() {
            taille += 1;
            for (dx, dy) in VOISINS8 {
                let (nx, ny) = (x + dx, y + dy);
                if v.bati(nx, ny) || (dx != 0 && dy != 0 && (v.bati(x + dx, y) || v.bati(x, y + dy))) {
                    continue;
                }
                let k = ny as usize * COTE + nx as usize;
                if zone[k] == u32::MAX {
                    zone[k] = n;
                    file.push_back((nx, ny));
                }
            }
        }
        if taille > taille_max {
            (meilleure, taille_max) = (n, taille);
        }
        n += 1;
    }
    let (x0, y0) = (p[0] as i32, p[1] as i32);
    for r in 0..COTE as i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (x0 + dx, y0 + dy);
                if (dx.abs() == r || dy.abs() == r) && !v.bati(x, y) && zone[y as usize * COTE + x as usize] == meilleure {
                    return [x as f32 + 0.5, y as f32 + 0.5];
                }
            }
        }
    }
    unreachable!("la plus grande zone n'est pas vide")
}

/// Champ exact à 1 m depuis des points (zone accessible locale).
pub fn champ_1m(v: &Ville, sources: &[[f32; 2]]) -> (Terrain, Champ) {
    let t = Terrain::de_ville(v, 1, Regle::Majorite, false);
    let mut c = Champ::new(&t);
    let mut d = Dial::new(&t);
    let g: Vec<Graine> = sources.iter().map(|&p| Graine { i: t.indice(p) as u32, d: 0, ts: 0 }).collect();
    d.calculer(&t, &mut c, &g);
    (t, c)
}

pub fn zone_de(t1: &Terrain, c1: &Champ) -> Vec<u64> {
    let mut zone = vec![0u64; carte::MOTS];
    for y in 0..COTE as i32 {
        for x in 0..COTE as i32 {
            if c1.dist[t1.ixy(x, y)] != champ::INF {
                carte::poser_bit(&mut zone, y as usize * COTE + x as usize);
            }
        }
    }
    zone
}

/// Anneau de barricades (P-03) autour de `centre`, rayon `r0`, épaisseur
/// 2 m ; toutes les rues barrées sauf les deux plus proches de l'est et de
/// l'ouest.
pub fn anneau_barricades(v: &Ville, zone: &[u64], centre: [f32; 2], r0: f32) -> Vec<(u16, u16)> {
    let mut cases = vec![];
    let (cx, cy) = (centre[0] as i32, centre[1] as i32);
    let rr = r0 as i32 + 3;
    for y in cy - rr..=cy + rr {
        for x in cx - rr..=cx + rr {
            if x < 0 || y < 0 || x >= COTE as i32 || y >= COTE as i32 || v.bati(x, y) {
                continue;
            }
            let d = d2([x as f32 + 0.5, y as f32 + 0.5], centre).sqrt();
            if (r0..r0 + 2.0).contains(&d) && bit(zone, y as usize * COTE + x as usize) {
                cases.push((x, y));
            }
        }
    }
    let ens: std::collections::HashSet<(i32, i32)> = cases.iter().copied().collect();
    let mut vu = std::collections::HashSet::new();
    let mut comps: Vec<(f32, Vec<(i32, i32)>)> = vec![];
    for &c in &cases {
        if !vu.insert(c) {
            continue;
        }
        let mut comp = vec![c];
        let mut k = 0;
        while k < comp.len() {
            let (x, y) = comp[k];
            k += 1;
            for (dx, dy) in VOISINS8 {
                let q = (x + dx, y + dy);
                if ens.contains(&q) && vu.insert(q) {
                    comp.push(q);
                }
            }
        }
        let (sx, sy) = comp.iter().fold((0f32, 0f32), |a, &(x, y)| (a.0 + x as f32, a.1 + y as f32));
        let n = comp.len() as f32;
        comps.push(((sy / n - centre[1]).atan2(sx / n - centre[0]), comp));
    }
    let ecart = |a: f32, b: f32| {
        let d = (a - b).rem_euclid(std::f32::consts::TAU);
        d.min(std::f32::consts::TAU - d)
    };
    let ouverte = |cible: f32| {
        (0..comps.len())
            .min_by(|&a, &b| ecart(comps[a].0, cible).total_cmp(&ecart(comps[b].0, cible)))
            .unwrap_or(usize::MAX)
    };
    let (o1, o2) = (ouverte(0.0), ouverte(std::f32::consts::PI));
    comps
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != o1 && *i != o2)
        .flat_map(|(_, c)| c.1.iter().map(|&(x, y)| (x as u16, y as u16)))
        .collect()
}

// ------------------------------------------------- Environnement d'un slot

/// Vue d'un zombie sur le monde : son slot, les slots voisins actifs, la
/// carte. Les requêtes de voisinage traversent les frontières.
struct Env<'a> {
    slots: &'a [Slot],
    topo: &'a Topo,
    carte: &'a Carte,
    s: usize,
}

impl<'a> Env<'a> {
    #[inline(always)]
    fn slot(&self) -> &'a Slot {
        &self.slots[self.s]
    }
    #[inline(always)]
    fn bati(&self, p: [f32; 2]) -> bool {
        self.carte.bati_p(self.slot().tile, p)
    }
    /// Point hors de la tile, dans une tile qui n'est pas active ?
    #[inline(always)]
    fn hors_zone_active(&self, p: [f32; 2]) -> bool {
        if dedans(p) {
            return false;
        }
        let (dx, dy) = ((p[0] / COTE_F).floor() as i32, (p[1] / COTE_F).floor() as i32);
        match self.topo.geo.voisine(self.slot().tile, dx, dy) {
            Some(v) => !matches!(self.topo.etat[v], EtatTuile::Actif(_)),
            None => true,
        }
    }
    /// Case (x, y) de la grille du slot, éventuellement hors de la tile :
    /// (slot, x, y dans ce slot, décalage à ajouter aux positions, propre).
    #[inline(always)]
    fn resoudre(&self, x: i32, y: i32) -> Option<(usize, i32, i32, [f32; 2], bool)> {
        let c = self.slot().vois.cote as i32;
        let (ox, oy) = (x.div_euclid(c), y.div_euclid(c));
        if ox == 0 && oy == 0 {
            return Some((self.s, x, y, [0.0; 2], true));
        }
        if ox.abs() > 1 || oy.abs() > 1 {
            return None;
        }
        let v = self.topo.voisines[self.s][(oy + 1) as usize][(ox + 1) as usize]? as usize;
        Some((v, x - ox * c, y - oy * c, [ox as f32 * COTE_F, oy as f32 * COTE_F], false))
    }
    #[inline(always)]
    fn case_de(&self, p: [f32; 2]) -> (i32, i32) {
        let inv = 1.0 / self.slot().vois.taille;
        ((p[0] * inv).floor() as i32, (p[1] * inv).floor() as i32)
    }
    /// Occupants des 3 × 3 cases autour de `pos`, ligne par ligne puis case
    /// par case ; `f(élément, décalage, propre)` ; arrêt si `f` rend faux.
    #[inline(always)]
    fn pour_voisins(&self, pos: [f32; 2], mut f: impl FnMut(&Elem, [f32; 2], bool) -> bool) {
        let g = &self.slot().vois;
        let (cx, cy) = g.coord(pos);
        let m = g.cote as i32 - 1;
        if cx >= 1 && cy >= 1 && cx < m && cy < m {
            for y in cy - 1..=cy + 1 {
                if !g.pour_plage(y, cx - 1, cx + 1, |e| f(e, [0.0; 2], true)) {
                    return;
                }
            }
            return;
        }
        for y in cy - 1..=cy + 1 {
            for x in cx - 1..=cx + 1 {
                if let Some((v, x2, y2, off, propre)) = self.resoudre(x, y) {
                    for e in self.slots[v].vois.case(x2, y2) {
                        if !f(e, off, propre) {
                            return;
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- Systèmes

/// Phase 1 (suite) : humains vus par chaque tile (halo compris), arbre k-d
/// et carte de proximité.
fn humains_locaux(mut s: ResMut<Slots>, h: Res<Humains>) {
    let hs = &*h;
    ComputeTaskPool::get().scope(|sc| {
        for slot in s.0.iter_mut().filter(|x| x.etat == EtatSlot::Actif) {
            sc.spawn(async move {
                slot.hpos.clear();
                slot.hid.clear();
                for (i, p) in hs.pos.iter().enumerate() {
                    if !hs.actif[i] {
                        continue;
                    }
                    let l = slot.local(*p);
                    if l[0] >= -HALO && l[1] >= -HALO && l[0] < COTE_F + HALO && l[1] < COTE_F + HALO {
                        slot.hpos.push(l);
                        slot.hid.push(i as u32);
                    }
                }
                slot.index.reconstruire(&slot.hpos);
                slot.proximite.reconstruire(&slot.hpos);
            });
        }
    });
}

/// Phase 2 : index des zombies.
fn grilles(mut s: ResMut<Slots>) {
    ComputeTaskPool::get().scope(|sc| {
        for slot in s.0.iter_mut().filter(|x| x.etat == EtatSlot::Actif) {
            sc.spawn(async move {
                let Slot { vois, indexe, inst, horodatage, cible_al, n_haut, .. } = slot;
                vois.reconstruire_avec(
                    *n_haut,
                    |i| indexe[i].load(Relaxed) != 0,
                    |i| decoder(inst[i].load(Relaxed)),
                    |i| (horodatage[i].load(Relaxed), decoder(cible_al[i].load(Relaxed))),
                );
            });
        }
    });
}

/// Phase 3 : champs des tiles actives, et chantiers d'activation.
fn champs(
    mut s: ResMut<Slots>,
    b: Res<Bordures>,
    topo: Res<Topo>,
    carte: Res<Carte>,
    h: Res<Humains>,
    r: Res<Reglages>,
    hor: Res<Horloge>,
) {
    let tick = hor.0;
    let (b, topo, carte, h, p) = (&*b, &*topo, &*carte, &*h, &r.p);
    ComputeTaskPool::get().scope(|sc| {
        for (k, slot) in s.0.iter_mut().enumerate() {
            match slot.etat {
                EtatSlot::Actif => sc.spawn(async move { champs_slot(k, slot, b, topo, carte, h, p, tick) }),
                EtatSlot::Chantier if slot.chantier.as_ref().is_some_and(|c| c.etape < 3) => {
                    sc.spawn(async move { avancer_chantier(slot, carte, p) })
                }
                _ => {}
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn champs_slot(k: usize, slot: &mut Slot, b: &Bordures, topo: &Topo, carte: &Carte, h: &Humains, p: &Params, tick: u64) {
    let maintenant = tick as u32 + 1;
    let tile = slot.tile;
    if !slot.destructions.is_empty() {
        let liste = std::mem::take(&mut slot.destructions);
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &(x, y) in &liste {
            let (x, y) = (x as i32, y as i32);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        slot.dial.abandonner();
        let maj = slot.terrain.mettre_a_jour(x0, y0, x1, y1, |x, y| carte.fin(tile, x, y));
        if !slot.proies.vide {
            if !maj.hausses.is_empty() {
                slot.dial.recalcul_hausse(&slot.terrain, &mut slot.proies, &maj.hausses, &slot.sources_proies);
            }
            if !maj.baisses.is_empty() {
                slot.dial.recalcul_baisse(&slot.terrain, &mut slot.proies, &maj.baisses);
            }
        }
    }
    let per = p.periode_champ.max(1) as u64;
    let due = (tick + tile as u64) % per == 0;
    if !slot.dial.en_cours() {
        if !due {
            return;
        }
        let t0 = Instant::now();
        let src = &mut slot.sources_suivantes;
        src.clear();
        let dec = [(h.origine[0] - slot.origine[0]) as f32, (h.origine[1] - slot.origine[1]) as f32];
        for i in 0..h.pos.len() {
            let ot = h.obs_tick[i].load(Relaxed);
            if ot != 0 && maintenant - ot < p.duree_derniere_position {
                let q = decoder(h.obs_pos[i].load(Relaxed));
                let l = [q[0] + dec[0], q[1] + dec[1]];
                if dedans(l) {
                    src.push(Graine { i: slot.terrain.indice(l) as u32, d: 0, ts: ot as u16 });
                }
            }
        }
        // Bordures des voisines actives (horodatées : pas de comptage à
        // l'infini).
        let mut vois: [[Option<&Echange>; 3]; 3] = [[None; 3]; 3];
        for (dy, ligne) in vois.iter_mut().enumerate() {
            for (dx, v) in ligne.iter_mut().enumerate() {
                if let Some(j) = topo.voisines[k][dy][dx] {
                    *v = b.0[j as usize].as_ref();
                }
            }
        }
        champ::graines_halo(&slot.terrain, &vois, maintenant as u16, Some(p.duree_derniere_position as u16), src);
        src.sort_unstable_by_key(|g| (g.i, g.d, Reverse(g.ts)));
        src.dedup_by_key(|g| g.i);
        slot.dial.demarrer(&mut slot.proies_suivant, &slot.sources_suivantes);
        slot.ns_cours = t0.elapsed().as_nanos() as u64;
    }
    let t0 = Instant::now();
    let budget = if p.etalement && per > 1 { slot.budget } else { u64::MAX };
    let (fini, _) = slot.dial.avancer(&slot.terrain, &mut slot.proies_suivant, budget);
    slot.ns_cours += t0.elapsed().as_nanos() as u64;
    if fini {
        std::mem::swap(&mut slot.proies, &mut slot.proies_suivant);
        std::mem::swap(&mut slot.sources_proies, &mut slot.sources_suivantes);
        slot.export_pret = Some(Echange::exporter(&slot.terrain, &slot.proies));
        let f = slot.dial.fixees_cours();
        slot.budget = (f * 5 / 4 / per).max(1024);
        slot.stat.ns += slot.ns_cours;
        slot.stat.max_ns = slot.stat.max_ns.max(slot.ns_cours);
        slot.stat.recalculs += 1;
    }
}

/// Point de la zone tiré pour le zombie `i` d'une tile jamais activée :
/// ne dépend que de (tile, i), pas du découpage du placement.
pub fn placer(v: &Ville, zone: &[u64], tile: usize, i: usize) -> ([f32; 2], f32) {
    let g = hacher(0x5EED_0004, (tile as u64) << 32 | i as u64);
    for j in 0..100_000u64 {
        let h = hacher(g, j);
        let p = [unite(h) * COTE_F, unite(hacher(h, 1)) * COTE_F];
        let (x, y) = (p[0] as usize, p[1] as usize);
        if bit(zone, y * COTE + x) && !v.bati(x as i32, y as i32) {
            return (p, unite(hacher(h, 2)) * std::f32::consts::TAU);
        }
    }
    panic!("tile {tile} sans zone accessible");
}

/// Avance le chantier d'activation d'un slot (dans la phase des champs).
fn avancer_chantier(slot: &mut Slot, carte: &Carte, p: &Params) {
    let t0 = Instant::now();
    let mut ch = slot.chantier.take().unwrap();
    let t = ch.tile;
    let tout = !p.etalement_activation;
    loop {
        match ch.etape {
            0 => {
                let te = Instant::now();
                if !p.zone_precalculee {
                    ch.zone_locale = Some(carte::zone_locale(&carte.tuiles[t].ville));
                }
                ch.ns_etapes[0] += te.elapsed().as_nanos() as u64;
                ch.etape = 1;
            }
            1 => {
                let te = Instant::now();
                let budget = if tout { u64::MAX } else { p.budget_terrain };
                let fini = slot.terrain.avancer_chantier(&mut slot.ch_terrain, budget, &|x, y| carte.fin(t, x, y));
                ch.ns_etapes[1] += te.elapsed().as_nanos() as u64;
                if !fini {
                    break;
                }
                ch.etape = 2;
                if !tout {
                    break;
                }
            }
            2 => {
                let te = Instant::now();
                match ch.source.take() {
                    Some(Source::Compteur(n)) => {
                        let tu = &carte.tuiles[t];
                        let zone = ch.zone_locale.as_deref().unwrap_or(&tu.zone);
                        let fin = if tout { n as usize } else { (ch.liste.len() + p.budget_placement).min(n as usize) };
                        for i in ch.liste.len()..fin {
                            ch.liste.push(placer(&tu.ville, zone, t, i));
                        }
                        if ch.liste.len() < n as usize {
                            ch.source = Some(Source::Compteur(n));
                        }
                    }
                    Some(Source::Lecture(mut jh)) => {
                        let tl = Instant::now();
                        let lu = jh.take().unwrap().join().expect("lecture");
                        let attente_ns = tl.elapsed().as_nanos() as u64;
                        let (octets, ns) = lu.expect("lecture de la sauvegarde");
                        ch.lecture_ns = ns.max(attente_ns);
                        let td = Instant::now();
                        let ts = sauvegarde::decoder(&octets).expect("sauvegarde illisible");
                        ch.decode_ns = td.elapsed().as_nanos() as u64;
                        restaurer(&mut ch, &ts, carte, t);
                    }
                    Some(Source::Memoire(ts)) => restaurer(&mut ch, &ts, carte, t),
                    None => {}
                }
                // Rejeu du journal de migration, quelques événements par tick.
                if ch.source.is_none() && ch.restauration {
                    let fin = if tout { ch.journal.len() } else { (ch.rejeu + REJEU_PAR_TICK).min(ch.journal.len()) };
                    let journal = std::mem::take(&mut ch.journal);
                    for m in &journal[ch.rejeu..fin] {
                        sauvegarde::appliquer(&mut ch.liste, m, carte, t);
                    }
                    ch.rejeu = fin;
                    ch.journal = journal;
                }
                ch.ns_etapes[2] += te.elapsed().as_nanos() as u64;
                if ch.source.is_none() && ch.rejeu == ch.journal.len() {
                    ch.etape = 3;
                }
                break;
            }
            _ => break,
        }
    }
    let ns = t0.elapsed().as_nanos() as u64;
    ch.ns += ns;
    ch.ns_max = ch.ns_max.max(ns);
    ch.ticks_travail += 1;
    slot.chantier = Some(ch);
}

fn restaurer(ch: &mut Chantier, ts: &TuileSauvee, carte: &Carte, t: usize) {
    ch.controle_ok = Some(sauvegarde::somme(ts) == ch.somme_attendue);
    ch.constructions_ok = Some(ts.constructions == carte.constructions(t));
    ch.liste = ts.zombies.iter().map(|z| (z.pos(), z.cap())).collect();
    let _ = carte;
    ch.corps = Some(ts.corps.clone());
}

#[inline(always)]
fn requete(pos: [f32; 2], cap: f32, r: &Reglages) -> Requete {
    let p = &r.p;
    Requete {
        pos,
        dir: [cap.cos(), cap.sin()],
        ouie2: p.rayon_ouie * p.rayon_ouie,
        vue2: p.distance_vue * p.distance_vue,
        cos_demi: r.cos_demi_angle,
        sin_demi: r.sin_demi_angle,
        k: p.rayons.min(RAYONS_MAX),
        ouie_possible: true,
    }
}

/// Phase 4 : perception.
fn percevoir(
    mut q: Query<(&Corps, &Ident, &mut Cerveau)>,
    s: Res<Slots>,
    topo: Res<Topo>,
    carte: Res<Carte>,
    h: Res<Humains>,
    r: Res<Reglages>,
    hor: Res<Horloge>,
) {
    let periode = r.p.periode_perception.max(1) as u64;
    q.par_iter_mut().batching_strategy(r.lots.clone()).for_each(|(corps, id, mut cerveau)| {
        let sl = &s.0[id.slot as usize];
        let rang = id.rang as usize;
        if sl.etat != EtatSlot::Actif || sl.abattu[rang].load(Relaxed) != 0 {
            return;
        }
        let env = Env { slots: &s.0, topo: &topo, carte: &carte, s: id.slot as usize };
        let pos = sl.local(corps.pos);
        let c = cerveau.bypass_change_detection();
        if c.etat == DEDANS {
            return;
        }
        let lent = c.etat == PROFOND || (r.p.veille && c.etat == ERRANCE);
        let per = if lent { r.p.periode_veille.max(1) as u64 } else { periode };
        let due = (hor.0 + id.rang as u64) % per == 0;
        let maintenant = hor.0 as u32 + 1;
        if c.etat == PROFOND {
            if !due || sl.proximite.distance_min(pos) > r.p.seuil_profond {
                return;
            }
            c.etat = ERRANCE;
        }
        decider(pos, corps.cap, id.rang, &env, &h, &r, c, due, maintenant);
        let relayable = c.etat == POURSUITE || c.etat == RELAIS;
        sl.horodatage[rang].store(if relayable { c.alerte } else { 0 }, Relaxed);
        if relayable {
            sl.cible_al[rang].store(coder(c.cible), Relaxed);
        }
        sl.indexe[rang].store((c.etat != PROFOND) as u8, Relaxed);
    });
}

#[inline(always)]
fn meilleur_relais(pos: [f32; 2], rang: u32, env: &Env, p: &Params, maintenant: u32) -> Option<(u32, [f32; 2])> {
    let r2 = p.rayon_relais * p.rayon_relais;
    let g = &env.slot().vois;
    let (cx, cy) = g.coord(pos);
    let m = g.cote as i32 - 1;
    if p.filtre_relais && cx >= 1 && cy >= 1 && cx < m && cy < m {
        if !(cy - 1..=cy + 1).any(|y| (cx - 1..=cx + 1).any(|x| g.alerte_dans(x, y))) {
            return None;
        }
    }
    let mut meilleur: Option<(u32, [f32; 2])> = None;
    let mut examines = 0u32;
    env.pour_voisins(pos, |e, off, propre| {
        if propre && e.id == rang {
            return true;
        }
        examines += 1;
        let ep = [e.p[0] + off[0], e.p[1] + off[1]];
        if e.alerte != 0
            && maintenant - e.alerte <= p.fraicheur_alerte
            && d2(ep, pos) <= r2
            && meilleur.is_none_or(|(a, _)| e.alerte > a)
        {
            meilleur = Some((e.alerte, [e.cible[0] + off[0], e.cible[1] + off[1]]));
        }
        examines < VOISINS_MAX
    });
    meilleur
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn decider(
    pos: [f32; 2],
    cap: f32,
    rang: u32,
    env: &Env,
    h: &Humains,
    r: &Reglages,
    c: &mut Cerveau,
    due: bool,
    maintenant: u32,
) {
    let p = &r.p;
    let sl = env.slot();
    let relais = if p.relais { meilleur_relais(pos, rang, env, p, maintenant) } else { None };
    let suivre = |c: &mut Cerveau, (a, cible): (u32, [f32; 2])| {
        c.etat = RELAIS;
        c.alerte = a;
        c.cible = cible;
    };
    let perdre = |c: &mut Cerveau| {
        c.etat = PERDU;
        c.alerte = 0;
        c.perdu_depuis = maintenant;
    };
    if !due {
        if c.etat != POURSUITE {
            if let Some(rel) = relais {
                suivre(c, rel);
            } else if c.etat == RELAIS {
                perdre(c);
            }
        }
        return;
    }
    if let Some(rel) = relais.filter(|(a, _)| maintenant - a <= p.seuil_source) {
        suivre(c, rel);
        return;
    }
    if let Some((q, id)) = percevoir_humain(pos, cap, env, r) {
        c.etat = POURSUITE;
        c.cible = q;
        c.alerte = maintenant;
        // Valeur canonique (relative au bloc) : la même pour tous les
        // observateurs du tick, quelle que soit leur tile.
        let hid = sl.hid[id as usize] as usize;
        h.obs_tick[hid].fetch_max(maintenant, Relaxed);
        h.obs_pos[hid].store(h.code[hid], Relaxed);
        return;
    }
    if let Some(rel) = relais {
        suivre(c, rel);
        return;
    }
    if c.etat == POURSUITE || c.etat == RELAIS {
        perdre(c);
    }
    if c.etat == PERDU && maintenant - c.perdu_depuis < p.duree_derniere_position && d2(c.cible, pos) > 4.0 {
        return;
    }
    c.etat = if p.profonde && sl.proximite.distance_min(pos) > p.seuil_profond { PROFOND } else { ERRANCE };
}

#[inline(always)]
fn percevoir_humain(pos: [f32; 2], cap: f32, env: &Env, r: &Reglages) -> Option<([f32; 2], u32)> {
    let p = &r.p;
    let sl = env.slot();
    let mut q = requete(pos, cap, r);
    let dmin = sl.proximite.distance_min(pos);
    if dmin > p.distance_vue.max(p.rayon_ouie) {
        return None;
    }
    q.ouie_possible = dmin <= p.rayon_ouie;
    let mut c = Compte::default();
    let res = sl.index.chercher(&q, &mut c);
    if let Some(e) = res.entendu {
        return Some((e.p, e.id));
    }
    let tile = sl.tile;
    for v in &res.vus[..res.nb] {
        if crate::ville::vue_degagee_avec(|x, y| env.carte.bati(tile, x, y), pos, v.p) {
            return Some((v.p, v.id));
        }
    }
    None
}

enum Pas {
    Aller([f32; 2]),
    /// Entrée dans une case : à réserver dans le slot `1`, case `2`.
    Demander([f32; 2], usize, usize),
}

/// Phase 5 : déplacement.
fn deplacer(
    mut q: Query<(&mut Corps, &Ident, &Cerveau)>,
    s: Res<Slots>,
    topo: Res<Topo>,
    carte: Res<Carte>,
    r: Res<Reglages>,
    hor: Res<Horloge>,
) {
    q.par_iter_mut().batching_strategy(r.lots.clone()).for_each(|(mut corps, id, cerveau)| {
        let sl = &s.0[id.slot as usize];
        let rang = id.rang as usize;
        if sl.etat != EtatSlot::Actif || sl.abattu[rang].load(Relaxed) != 0 || cerveau.etat == DEDANS {
            return;
        }
        let env = Env { slots: &s.0, topo: &topo, carte: &carte, s: id.slot as usize };
        let corps = corps.bypass_change_detection();
        let pos = sl.local(corps.pos);
        let pas = if env.bati(pos) {
            Pas::Aller(expulser(&env, pos))
        } else if cerveau.etat == PROFOND {
            Pas::Aller(deplacer_profond(pos, corps, id, &env, &r.p, hor.0))
        } else {
            deplacer_un(pos, corps, id, cerveau, &env, &r, hor.0)
        };
        match pas {
            Pas::Aller(np) => {
                if np != pos {
                    corps.pos = [sl.origine[0] + np[0] as f64, sl.origine[1] + np[1] as f64];
                    sl.inst[rang].store(coder(np), Relaxed);
                    if !dedans(np) {
                        let k = sl.nb_sortants.fetch_add(1, Relaxed) as usize;
                        sl.sortants[k].store(id.rang, Relaxed);
                    }
                }
            }
            Pas::Demander(np, dest, case) => {
                let d = &s.0[dest];
                let slot = d.nb_demandes.fetch_add(1, Relaxed) as usize;
                let tile = sl.tile as u64;
                let prio = hacher(tile << 32 | id.rang as u64, hor.0) & 0xFF_FFFF;
                d.demandes[slot].store((case as u64) << 46 | prio << 22 | (id.slot as u64) << 16 | id.rang as u64, Relaxed);
                sl.souhait[rang].store(coder(np), Relaxed);
                sl.attente[rang].store(1, Relaxed);
            }
        }
    });
}

#[inline(never)]
fn expulser(env: &Env, p: [f32; 2]) -> [f32; 2] {
    let (x0, y0) = (p[0].floor() as i32, p[1].floor() as i32);
    let t = env.slot().tile;
    for r in 1..=8i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                if (dx.abs() == r || dy.abs() == r) && !env.carte.bati(t, x0 + dx, y0 + dy) {
                    return [(x0 + dx) as f32 + 0.5, (y0 + dy) as f32 + 0.5];
                }
            }
        }
    }
    p
}

#[inline(always)]
fn deplacer_profond(pos: [f32; 2], corps: &mut Corps, id: &Ident, env: &Env, p: &Params, tick: u64) -> [f32; 2] {
    let n = p.periode_veille.max(1) as u64;
    if (tick + id.rang as u64) % n != 0 {
        return pos;
    }
    let graine = (env.slot().tile as u64) << 32 | id.rang as u64;
    let mut cap = corps.cap + (unite(hacher(graine, tick)) - 0.5);
    let pas = p.vitesse_errance * DT * n as f32;
    let sous = (pas / 0.25).ceil().max(1.0);
    let mut np = pos;
    for _ in 0..sous as u32 {
        let q = [np[0] + cap.cos() * pas / sous, np[1] + cap.sin() * pas / sous];
        if env.bati(q) || (p.bord_reflechi && env.hors_zone_active(q)) {
            cap += std::f32::consts::FRAC_PI_2 + unite(hacher(graine, !tick)) * 2.0;
            break;
        }
        np = q;
    }
    corps.cap = cap;
    np
}

#[inline(always)]
fn deplacer_un(pos: [f32; 2], corps: &mut Corps, id: &Ident, c: &Cerveau, env: &Env, r: &Reglages, tick: u64) -> Pas {
    let p = &r.p;
    let sl = env.slot();
    let graine = (sl.tile as u64) << 32 | id.rang as u64;
    let mut facteur = 1.0;
    if p.veille && c.etat == ERRANCE {
        let n = p.pas_veille.max(1) as u64;
        if (tick + id.rang as u64) % n != 0 {
            return Pas::Aller(pos);
        }
        facteur = n as f32;
    }
    let (vitesse, mut cap) = if c.etat == ERRANCE {
        let mut cap = corps.cap;
        if (tick + id.rang as u64) % 40 < facteur as u64 {
            cap += (unite(hacher(graine, tick)) - 0.5) * 2.0;
        }
        (p.vitesse_errance, cap)
    } else {
        let d = [c.cible[0] - pos[0], c.cible[1] - pos[1]];
        let d2c = d[0] * d[0] + d[1] * d[1];
        if d2c < 0.25 {
            (0.0, corps.cap)
        } else if let Some(dir) = (d2c > APPROCHE * APPROCHE).then(|| sl.proies.direction(&sl.terrain, pos)).flatten() {
            (p.vitesse_course, dir[1].atan2(dir[0]))
        } else {
            (p.vitesse_course, d[1].atan2(d[0]))
        }
    };
    let pas = vitesse * DT * facteur;
    let mut np = [pos[0] + cap.cos() * pas, pos[1] + cap.sin() * pas];

    let sep = p.distance_separation;
    let mut corr = [0f32; 2];
    let mut examines = 0u32;
    env.pour_voisins(pos, |e, off, propre| {
        if propre && e.id == id.rang {
            return true;
        }
        let d = [pos[0] - e.p[0] - off[0], pos[1] - e.p[1] - off[1]];
        let dd = d[0] * d[0] + d[1] * d[1];
        if dd < sep * sep && dd > 1e-10 {
            let l = dd.sqrt();
            let k = (sep - l) / l * 0.5;
            corr[0] += d[0] * k;
            corr[1] += d[1] * k;
        }
        examines += 1;
        examines < VOISINS_MAX
    });
    let lc = (corr[0] * corr[0] + corr[1] * corr[1]).sqrt();
    if lc > 0.3 {
        corr = [corr[0] * 0.3 / lc, corr[1] * 0.3 / lc];
    }
    np = [np[0] + corr[0], np[1] + corr[1]];

    let ici = env.case_de(pos);
    let capa = r.capacite;
    // Réservation exacte partout : une case pleine (y compris dans la tile
    // voisine) n'accepte plus personne ; la réservation décidera.
    let sans_cible = c.etat == ERRANCE || c.etat == PERDU;
    let libre = |q: [f32; 2]| {
        if env.bati(q) || (sans_cible && p.bord_reflechi && env.hors_zone_active(q)) {
            return false;
        }
        let c = env.case_de(q);
        if c == ici {
            return true;
        }
        match env.resoudre(c.0, c.1) {
            Some((v, x, y, _, _)) => env.slots[v].vois.occupants(x, y) < capa,
            None => true,
        }
    };
    if !libre(np) {
        // Les zombies qui ont une cible humaine entament la barricade qui
        // les bloque, si sa tile est active (sans humain, pas de dégâts).
        if c.etat == POURSUITE || c.etat == RELAIS {
            if let Some((bt, bi)) = env.carte.barricade(sl.tile, np[0].floor() as i32, np[1].floor() as i32) {
                if matches!(env.topo.etat[bt], EtatTuile::Actif(_)) {
                    env.carte.tuiles[bt].barricades[bi as usize].pv.fetch_sub(1, Relaxed);
                }
            }
        }
        if libre([np[0], pos[1]]) {
            np = [np[0], pos[1]];
        } else if libre([pos[0], np[1]]) {
            np = [pos[0], np[1]];
        } else {
            np = pos;
            if c.etat == ERRANCE {
                cap += std::f32::consts::FRAC_PI_2 + unite(hacher(graine, !tick)) * 2.0;
            }
        }
    }
    corps.cap = cap;
    let cn = env.case_de(np);
    if cn != ici {
        if let Some((v, x, y, _, _)) = env.resoudre(cn.0, cn.1) {
            return Pas::Demander(np, v, y as usize * env.slots[v].vois.cote + x as usize);
        }
    }
    Pas::Aller(np)
}

/// Phase 6a : décision des réservations, par slot de destination.
fn admettre(s: Res<Slots>, r: Res<Reglages>) {
    let capa = r.capacite;
    let sl = &s.0;
    ComputeTaskPool::get().scope(|sc| {
        for occ in sl.iter().filter(|x| x.etat == EtatSlot::Actif) {
            sc.spawn(async move {
                let nb = occ.nb_demandes.swap(0, Relaxed) as usize;
                if nb == 0 {
                    return;
                }
                let mut d: Vec<u64> = occ.demandes[..nb].iter().map(|a| a.load(Relaxed)).collect();
                d.sort_unstable();
                let cote = occ.vois.cote;
                let mut i = 0;
                while i < d.len() {
                    let case = (d[i] >> 46) as usize;
                    let mut j = i;
                    while j < d.len() && (d[j] >> 46) as usize == case {
                        j += 1;
                    }
                    let occupants = occ.vois.occupants((case % cote) as i32, (case / cote) as i32);
                    let places = capa.saturating_sub(occupants) as usize;
                    for &x in &d[i..(i + places).min(j)] {
                        let src = ((x >> 16) & 63) as usize;
                        sl[src].attente[(x & 0xFFFF) as usize].store(2, Relaxed);
                    }
                    i = j;
                }
            });
        }
    });
}

/// Phase 6b : application des réservations admises.
fn appliquer(mut q: Query<(&mut Corps, &Ident)>, s: Res<Slots>, r: Res<Reglages>) {
    q.par_iter_mut().batching_strategy(r.lots.clone()).for_each(|(mut corps, id)| {
        let sl = &s.0[id.slot as usize];
        if sl.etat != EtatSlot::Actif {
            return;
        }
        let k = id.rang as usize;
        let a = sl.attente[k].load(Relaxed);
        if a == 0 {
            return;
        }
        sl.attente[k].store(0, Relaxed);
        if a == 2 {
            let v = sl.souhait[k].load(Relaxed);
            let np = decoder(v);
            corps.bypass_change_detection().pos = [sl.origine[0] + np[0] as f64, sl.origine[1] + np[1] as f64];
            sl.inst[k].store(v, Relaxed);
            if !dedans(np) {
                let j = sl.nb_sortants.fetch_add(1, Relaxed) as usize;
                sl.sortants[j].store(id.rang, Relaxed);
            }
        }
    });
}

// ---------------------------------------------------------------- Monde

pub enum Stock {
    Compteur(u32),
    Sauvee { n: u32, somme: u64, donnees: Donnees, journal: Vec<Mouvement> },
    /// Active, ou en chantier : les zombies sont dans un slot.
    Vivante,
}

pub struct Tuile {
    pub stock: Stock,
    /// Dernier tick où la tile était requise.
    pub besoin: Option<u64>,
}

/// Statistiques cumulées du monde.
#[derive(Default, Debug, Clone)]
pub struct StatsMonde {
    pub activations: u64,
    pub premieres: u64,
    pub restaurations: u64,
    pub desactivations: u64,
    pub controles_ok: u64,
    pub controles_ko: u64,
    pub constructions_ok: u64,
    pub constructions_ko: u64,
    pub chantier_ticks: Vec<u64>,
    pub chantier_ms: Vec<f64>,
    /// Travail cumulé des chantiers terminés (ns).
    pub chantier_ns: u64,
    pub chantier_max_tick_ms: f64,
    pub etapes_ms: [f64; 3],
    pub lecture_ms: Vec<f64>,
    pub decode_ms: Vec<f64>,
    pub naissance_ms: Vec<f64>,
    pub instantane_ms: Vec<f64>,
    pub destruction_ms: Vec<f64>,
    pub encode_ms: Vec<f64>,
    pub ecriture_ms: Vec<f64>,
    pub octets: Vec<usize>,
    pub zombies_sauves: Vec<usize>,
    pub passages: u64,
    pub passages_max_tick: u64,
    pub vers_chantier: u64,
    pub vers_inerte: u64,
    pub migres: u64,
    pub migration_saturee: u64,
    /// Passages refusés : slot de destination plein.
    pub passages_refuses: u64,
    pub migration_ms: f64,
    pub arrivees_actives: u64,
    pub abattus: u64,
    pub conservation_ko: u64,
    pub slots_max: usize,
    pub actifs_max: usize,
}

pub struct ConfigSim {
    pub threads: usize,
    pub lot_min: usize,
    pub slots_initiaux: usize,
    pub params: Params,
}

pub const NB_PHASES: usize = 8;
pub const NOMS_PHASES: [&str; NB_PHASES] =
    ["gestion", "humains", "grilles", "champs", "perception", "deplacement", "admission", "frontieres"];

pub struct Simulation {
    pub world: World,
    phases: [Schedule; 6],
    pub tuiles: Vec<Tuile>,
    pub stats: StatsMonde,
    pub p: Params,
    pub total_initial: u64,
    abattus_tick: Vec<(u32, u32)>,
    /// Naissances et destructions d'entités (ms) du dernier tick.
    pub entites_tick_ms: f64,
}

impl Simulation {
    pub fn new(c: &ConfigSim, carte: Carte, humains: Humains) -> Simulation {
        ComputeTaskPool::get_or_init(|| TaskPoolBuilder::new().num_threads(c.threads).build());
        let p = c.params.clone();
        std::fs::create_dir_all(&p.dossier).expect("dossier des sauvegardes");
        let geo = carte.geo;
        let mut world = World::new();
        let tuiles: Vec<Tuile> = carte
            .tuiles
            .iter()
            .map(|t| Tuile { stock: Stock::Compteur(t.population), besoin: None })
            .collect();
        let total_initial = carte.tuiles.iter().map(|t| t.population as u64).sum();
        world.insert_resource(carte);
        world.insert_resource(humains);
        world.insert_resource(Slots((0..c.slots_initiaux).map(|_| Slot::new(&p)).collect()));
        world.insert_resource(Topo { geo, etat: vec![EtatTuile::Inerte; geo.n()], voisines: vec![] });
        world.insert_resource(Bordures(vec![]));
        world.insert_resource(Horloge(0));
        let demi = p.demi_angle_vue_deg.to_radians();
        world.insert_resource(Reglages {
            cos_demi_angle: demi.cos(),
            sin_demi_angle: demi.sin(),
            p: p.clone(),
            lots: BatchingStrategy::new().min_batch_size(c.lot_min.max(1)),
            capacite: p.capacite_case,
        });
        let mut phases: [Schedule; 6] = Default::default();
        phases[0].add_systems(humains_locaux);
        phases[1].add_systems(grilles);
        phases[2].add_systems(champs);
        phases[3].add_systems(percevoir);
        phases[4].add_systems(deplacer);
        phases[5].add_systems((admettre, appliquer).chain());
        for s in &mut phases {
            s.set_executor(MultiThreadedExecutor::new());
            s.initialize(&mut world).expect("initialisation du schedule");
        }
        if p.prechauffe_entites > 0 {
            let e: Vec<Entity> = world
                .spawn_batch((0..p.prechauffe_entites).map(|_| {
                    (
                        Corps { pos: [0.0; 2], cap: 0.0 },
                        Cerveau { etat: 0, cible: [0.0; 2], alerte: 0, perdu_depuis: 0 },
                        Ident { slot: 0, rang: 0 },
                    )
                }))
                .collect();
            for x in e {
                world.despawn(x);
            }
        }
        let mut sim = Simulation {
            world,
            phases,
            tuiles,
            stats: StatsMonde::default(),
            p,
            total_initial,
            abattus_tick: vec![],
            entites_tick_ms: 0.0,
        };
        sim.maj_topo();
        sim
    }

    pub fn horloge(&self) -> u64 {
        self.world.resource::<Horloge>().0
    }

    pub fn carte(&self) -> &Carte {
        self.world.resource::<Carte>()
    }
    #[allow(dead_code)]
    pub fn carte_mut(&mut self) -> Mut<'_, Carte> {
        self.world.resource_mut::<Carte>()
    }
    pub fn humains(&self) -> &Humains {
        self.world.resource::<Humains>()
    }
    pub fn slots(&self) -> &[Slot] {
        &self.world.resource::<Slots>().0
    }
    pub fn topo(&self) -> &Topo {
        self.world.resource::<Topo>()
    }

    /// Un tick ; durée de chaque phase.
    pub fn tick(&mut self) -> [Duration; NB_PHASES] {
        let mut d = [Duration::ZERO; NB_PHASES];
        let t = Instant::now();
        self.gestion();
        d[0] = t.elapsed();
        let t = Instant::now();
        self.humains_et_tirs();
        self.phases[0].run(&mut self.world);
        d[1] = t.elapsed();
        for i in 1..6 {
            let t = Instant::now();
            self.phases[i].run(&mut self.world);
            d[i + 1] = t.elapsed();
        }
        let t = Instant::now();
        self.frontieres();
        d[7] = t.elapsed();
        self.world.resource_mut::<Horloge>().0 += 1;
        // Ce que fait la boucle d'une App Bevy à chaque frame : sans cela, les
        // journaux de composants retirés (un par despawn) grossissent sans fin
        // et leur réallocation coûte plusieurs ms d'un coup.
        self.world.clear_trackers();
        d
    }

    fn maj_topo(&mut self) {
        let geo = self.world.resource::<Topo>().geo;
        let (etat, tiles): (Vec<EtatSlot>, Vec<usize>) =
            self.world.resource::<Slots>().0.iter().map(|s| (s.etat, s.tile)).unzip();
        let mut topo = self.world.resource_mut::<Topo>();
        let mut slot_de = vec![None; geo.n()];
        for (k, (&e, &t)) in etat.iter().zip(&tiles).enumerate() {
            if e == EtatSlot::Actif {
                slot_de[t] = Some(k as u32);
            }
        }
        topo.voisines = (0..etat.len())
            .map(|k| {
                let mut v = [[None; 3]; 3];
                if etat[k] == EtatSlot::Actif {
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            if (dx, dy) != (0, 0)
                                && let Some(n) = geo.voisine(tiles[k], dx, dy)
                            {
                                v[(dy + 1) as usize][(dx + 1) as usize] = slot_de[n];
                            }
                        }
                    }
                }
                v
            })
            .collect();
        let n = etat.len();
        let mut b = self.world.resource_mut::<Bordures>();
        b.0.resize_with(n, || None);
    }

    /// Phase 0 (exclusive) : écritures finies, barricades détruites,
    /// besoins, désactivations, activations, naissances, destructions,
    /// migration.
    fn gestion(&mut self) {
        let tick = self.horloge();
        let te = Instant::now();
        // Écritures terminées : la tile ne vit plus que sur disque.
        for t in &mut self.tuiles {
            if let Stock::Sauvee { donnees, .. } = &mut t.stock
                && let Donnees::Memoire(ts, h) = donnees
                && h.as_ref().is_some_and(|h| h.is_finished())
            {
                let e = h.take().unwrap().join().expect("écriture");
                self.stats.encode_ms.push(e.encode_ns as f64 / 1e6);
                self.stats.ecriture_ms.push(e.ecriture_ns as f64 / 1e6);
                self.stats.octets.push(e.octets);
                let (x, y) = (ts.x_km, ts.y_km);
                *donnees = Donnees::Disque(self.p.dossier.join(format!("x{x}_y{y}.tile")));
            }
        }
        let mut topo_change = false;
        // Barricades détruites (tiles actives).
        {
            let actives: Vec<(usize, usize)> = self
                .world
                .resource::<Slots>()
                .0
                .iter()
                .enumerate()
                .filter(|(_, s)| s.etat == EtatSlot::Actif)
                .map(|(k, s)| (k, s.tile))
                .collect();
            for (k, t) in actives {
                if self.world.resource::<Carte>().tuiles[t].barricades.is_empty() {
                    continue;
                }
                let d = self.world.resource_mut::<Carte>().detruire_barricades(t);
                if !d.is_empty() {
                    self.world.resource_mut::<Slots>().0[k].destructions.extend(d);
                }
            }
        }
        // Besoins.
        let geo = self.world.resource::<Topo>().geo;
        let mut requis = vec![false; geo.n()];
        let hs = self.world.resource::<Humains>();
        for (i, p) in hs.pos.iter().enumerate() {
            if !hs.actif[i] {
                continue;
            }
            if let Some(t) = geo.tile_de(*p) {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if let Some(v) = geo.voisine(t, dx, dy) {
                            requis[v] = true;
                        }
                    }
                }
            }
        }
        for (t, r) in requis.iter().enumerate() {
            if *r {
                self.tuiles[t].besoin = Some(tick);
            }
        }
        // Désactivations.
        let a_desactiver: Vec<usize> = self
            .world
            .resource::<Slots>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.etat == EtatSlot::Actif
                    && self.tuiles[s.tile].besoin.is_none_or(|b| tick.saturating_sub(b) > self.p.delai)
            })
            .map(|(k, _)| k)
            .take(if self.p.desactivations_par_tick == 0 { usize::MAX } else { self.p.desactivations_par_tick })
            .collect();
        if !a_desactiver.is_empty() {
            self.desactiver(&a_desactiver, tick);
            topo_change = true;
        }
        // Activations.
        for t in 0..geo.n() {
            if requis[t] && !matches!(self.tuiles[t].stock, Stock::Vivante) {
                self.lancer_chantier(t, tick);
            }
        }
        // Naissances des chantiers prêts.
        let prets: Vec<usize> = self
            .world
            .resource::<Slots>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, s)| s.etat == EtatSlot::Chantier && s.chantier.as_ref().is_some_and(|c| c.etape == 3))
            .map(|(k, _)| k)
            .collect();
        for k in prets {
            if self.faire_naitre(k, tick) {
                topo_change = true;
            }
        }
        // Destructions d'entités.
        self.detruire_entites();
        // Migration entre tiles inertes.
        if !self.p.stimuli.is_empty() && tick > 0 && tick % self.p.periode_migration == 0 {
            self.migrer(tick);
        }
        if topo_change {
            self.maj_topo();
        }
        let n = self.world.resource::<Slots>().0.len();
        self.stats.slots_max = self.stats.slots_max.max(n);
        let actifs = self.world.resource::<Slots>().0.iter().filter(|s| s.etat == EtatSlot::Actif).count();
        self.stats.actifs_max = self.stats.actifs_max.max(actifs);
        let _ = te;
    }

    fn slot_libre(&mut self) -> usize {
        let p = self.p.clone();
        let slots = &mut self.world.resource_mut::<Slots>().0;
        if let Some(k) = slots.iter().position(|s| s.etat == EtatSlot::Libre) {
            return k;
        }
        assert!(slots.len() < SLOTS_MAX, "plus de slot libre");
        slots.push(Slot::new(&p));
        slots.len() - 1
    }

    fn lancer_chantier(&mut self, t: usize, tick: u64) {
        let stock = std::mem::replace(&mut self.tuiles[t].stock, Stock::Vivante);
        let (source, n, somme, journal, restauration) = match stock {
            Stock::Compteur(n) => (Source::Compteur(n), n, 0, vec![], false),
            Stock::Sauvee { n, somme, donnees, journal } => {
                let src = match donnees {
                    Donnees::Memoire(ts, h) => {
                        // Écriture pas finie : on garde son résultat.
                        if let Some(h) = h {
                            let e = h.join().expect("écriture");
                            self.stats.encode_ms.push(e.encode_ns as f64 / 1e6);
                            self.stats.ecriture_ms.push(e.ecriture_ns as f64 / 1e6);
                            self.stats.octets.push(e.octets);
                        }
                        Source::Memoire(ts)
                    }
                    Donnees::Disque(chemin) => Source::Lecture(Some(std::thread::spawn(move || {
                        let t0 = Instant::now();
                        let b = std::fs::read(&chemin)?;
                        Ok((b, t0.elapsed().as_nanos() as u64))
                    }))),
                };
                (src, n, somme, journal, true)
            }
            Stock::Vivante => unreachable!(),
        };
        let ch = Chantier {
            tile: t,
            debut: tick,
            etape: 0,
            source: Some(source),
            n_source: n,
            somme_attendue: somme,
            journal,
            rejeu: 0,
            attente: vec![],
            liste: vec![],
            naissance: 0,
            zone_locale: None,
            corps: None,
            restauration,
            controle_ok: None,
            constructions_ok: None,
            ns: 0,
            ns_max: 0,
            ticks_travail: 0,
            ns_etapes: [0; 3],
            lecture_ns: 0,
            decode_ns: 0,
            naissance_ns: 0,
        };
        let k = self.slot_libre();
        let origine = self.world.resource::<Topo>().geo.origine(t);
        self.world.resource_mut::<Slots>().0[k].preparer(t, origine, ch);
        self.world.resource_mut::<Topo>().etat[t] = EtatTuile::Chantier(k as u32);
    }

    /// Carte de proximité temporaire des humains autour d'une tile.
    fn proximite_de(&self, origine: [f64; 2]) -> Proximite {
        let mut pr = Proximite::avec_etendue(CASE_PROXIMITE, -HALO, COTE_F + 2.0 * HALO);
        let pts: Vec<[f32; 2]> = self
            .humains()
            .pos
            .iter()
            .map(|p| [(p[0] - origine[0]) as f32, (p[1] - origine[1]) as f32])
            .filter(|l| l[0] >= -HALO && l[1] >= -HALO && l[0] < COTE_F + HALO && l[1] < COTE_F + HALO)
            .collect();
        pr.reconstruire(&pts);
        pr
    }

    /// Fait naître (par lots) les zombies d'un chantier prêt ; active la
    /// tile quand tout est né. Renvoie vrai si la tile est devenue active.
    fn faire_naitre(&mut self, k: usize, tick: u64) -> bool {
        let t0 = Instant::now();
        let origine = self.world.resource::<Slots>().0[k].origine;
        let prox = self.proximite_de(origine);
        let (lot, seuil, profonde) = (self.p.lot_naissance, self.p.seuil_profond, self.p.profonde);
        let mut slots = self.world.remove_resource::<Slots>().unwrap();
        let slot = &mut slots.0[k];
        let mut ch = slot.chantier.take().unwrap();
        // Les entrées à pied pendant le chantier naissent avec les autres.
        let att = std::mem::take(&mut ch.attente);
        ch.liste.extend(att);
        let debut = ch.naissance;
        let fin = if lot == 0 { ch.liste.len() } else { (debut + lot).min(ch.liste.len()) };
        let etat_de = |p: [f32; 2]| if profonde && prox.distance_min(p) > seuil { PROFOND } else { ERRANCE };
        let lots: Vec<(Corps, Cerveau, Ident)> = (debut..fin)
            .map(|r| {
                let (p, cap) = ch.liste[r];
                (
                    Corps { pos: [origine[0] + p[0] as f64, origine[1] + p[1] as f64], cap },
                    Cerveau { etat: etat_de(p), cible: [0.0; 2], alerte: 0, perdu_depuis: 0 },
                    Ident { slot: k as u32, rang: r as u32 },
                )
            })
            .collect();
        let etats: Vec<u8> = lots.iter().map(|l| l.1.etat).collect();
        let ents: Vec<Entity> = self.world.spawn_batch(lots).collect();
        for (j, e) in ents.into_iter().enumerate() {
            let r = slot.allouer();
            debug_assert_eq!(r, debut + j);
            slot.installer(r, e, ch.liste[r].0, etats[j]);
        }
        ch.naissance = fin;
        ch.naissance_ns += t0.elapsed().as_nanos() as u64;
        let fini = ch.naissance == ch.liste.len();
        if fini {
            let s = &mut self.stats;
            s.activations += 1;
            if ch.restauration {
                s.restaurations += 1;
            } else {
                s.premieres += 1;
            }
            match ch.controle_ok {
                Some(true) => s.controles_ok += 1,
                Some(false) => s.controles_ko += 1,
                None => {}
            }
            match ch.constructions_ok {
                Some(true) => s.constructions_ok += 1,
                Some(false) => s.constructions_ko += 1,
                None => {}
            }
            s.chantier_ticks.push(tick - ch.debut);
            s.chantier_ms.push(ch.ns as f64 / 1e6);
            s.chantier_ns += ch.ns;
            s.chantier_max_tick_ms = s.chantier_max_tick_ms.max(ch.ns_max as f64 / 1e6);
            for i in 0..3 {
                s.etapes_ms[i] += ch.ns_etapes[i] as f64 / 1e6;
            }
            if ch.restauration && ch.lecture_ns > 0 {
                s.lecture_ms.push(ch.lecture_ns as f64 / 1e6);
                s.decode_ms.push(ch.decode_ns as f64 / 1e6);
            }
            s.naissance_ms.push(ch.naissance_ns as f64 / 1e6);
            if let Some(c) = ch.corps.take() {
                slot.corps.copy_from_slice(&c);
            }
            // Budget d'étalement du premier champ, estimé sur le terrain :
            // sans lui, le premier calcul d'une tile neuve serait d'un seul
            // tenant (pic de 4 à 6 ms dans la phase des champs).
            let per = self.p.periode_champ.max(1) as u64;
            slot.budget = (slot.terrain.libres() as u64 * 5 / 4 / per).max(1024);
            slot.etat = EtatSlot::Actif;
            let t = slot.tile;
            self.world.insert_resource(slots);
            self.world.resource_mut::<Topo>().etat[t] = EtatTuile::Actif(k as u32);
            self.entites_tick_ms += t0.elapsed().as_secs_f64() * 1e3;
            return true;
        }
        slot.chantier = Some(ch);
        self.world.insert_resource(slots);
        self.entites_tick_ms += t0.elapsed().as_secs_f64() * 1e3;
        false
    }

    /// Instantané normalisé, sauvegarde en tâche de fond, entités à détruire.
    fn desactiver(&mut self, ks: &[usize], tick: u64) {
        let t0 = Instant::now();
        // Lecture des seules entités du slot (accès direct par entité).
        let mut par_slot: Vec<Vec<(u32, [f64; 2], f32, Entity)>> = vec![vec![]; SLOTS_MAX];
        for &k in ks {
            let ents: Vec<(usize, Entity)> = self.world.resource::<Slots>().0[k]
                .entites
                .iter()
                .enumerate()
                .filter_map(|(r, e)| e.map(|e| (r, e)))
                .collect();
            for (r, e) in ents {
                let c = self.world.get::<Corps>(e).unwrap();
                par_slot[k].push((r as u32, c.pos, c.cap, e));
            }
        }
        let ms_lecture = t0.elapsed().as_secs_f64() * 1e3 / ks.len() as f64;
        let geo = self.world.resource::<Topo>().geo;
        for &k in ks {
            let t1 = Instant::now();
            let mut l = std::mem::take(&mut par_slot[k]);
            l.sort_unstable_by_key(|z| z.0);
            let (tile, origine, corps, vivants) = {
                let s = &self.world.resource::<Slots>().0[k];
                (s.tile, s.origine, s.corps.clone(), s.vivants)
            };
            assert_eq!(l.len(), vivants, "instantané incomplet");
            let mut zombies: Vec<ZSauve> = l
                .iter()
                .map(|z| ZSauve::de([(z.1[0] - origine[0]) as f32, (z.1[1] - origine[1]) as f32], z.2))
                .collect();
            zombies.sort_unstable();
            let (x_km, y_km) = geo.km(tile);
            let ts = Arc::new(TuileSauvee {
                x_km,
                y_km,
                tick,
                zombies,
                corps,
                constructions: self.carte().constructions(tile),
            });
            let somme = sauvegarde::somme(&ts);
            let n = ts.zombies.len() as u32;
            self.stats.zombies_sauves.push(n as usize);
            let (f, fsync) = (self.p.format, self.p.fsync);
            let chemin = self.p.dossier.join(format!("x{x_km}_y{y_km}.tile"));
            let ts2 = Arc::clone(&ts);
            let h = std::thread::spawn(move || {
                let t0 = Instant::now();
                let b = sauvegarde::encoder(&ts2, f);
                let encode_ns = t0.elapsed().as_nanos() as u64;
                let t1 = Instant::now();
                {
                    use std::io::Write;
                    let mut fic = std::fs::File::create(&chemin).expect("création de la sauvegarde");
                    fic.write_all(&b).expect("écriture de la sauvegarde");
                    if fsync {
                        fic.sync_all().expect("fsync");
                    }
                }
                Ecrit { octets: b.len(), encode_ns, ecriture_ns: t1.elapsed().as_nanos() as u64 }
            });
            self.tuiles[tile].stock = Stock::Sauvee { n, somme, donnees: Donnees::Memoire(ts, Some(h)), journal: vec![] };
            self.tuiles[tile].besoin = None;
            {
                let s = &mut self.world.resource_mut::<Slots>().0[k];
                s.a_detruire = l.iter().map(|z| z.3).collect();
                s.etat = EtatSlot::Destruction;
                s.vivants = 0;
                s.n_haut = 0;
            }
            self.world.resource_mut::<Topo>().etat[tile] = EtatTuile::Inerte;
            self.world.resource_mut::<Bordures>().0[k] = None;
            self.stats.desactivations += 1;
            self.stats.instantane_ms.push(ms_lecture + t1.elapsed().as_secs_f64() * 1e3);
        }
    }

    fn detruire_entites(&mut self) {
        let lot = self.p.lot_destruction;
        let mut slots = self.world.remove_resource::<Slots>().unwrap();
        for s in slots.0.iter_mut().filter(|s| s.etat == EtatSlot::Destruction) {
            let t0 = Instant::now();
            let n = if lot == 0 { s.a_detruire.len() } else { lot.min(s.a_detruire.len()) };
            let debut = s.a_detruire.len() - n;
            for e in s.a_detruire.drain(debut..) {
                self.world.despawn(e);
            }
            let ns = t0.elapsed().as_nanos() as u64;
            self.entites_tick_ms += ns as f64 / 1e6;
            s.destruction_ns += ns;
            if s.a_detruire.is_empty() {
                self.stats.destruction_ms.push(s.destruction_ns as f64 / 1e6);
                s.destruction_ns = 0;
                s.etat = EtatSlot::Libre;
                s.entites.clear();
            }
        }
        self.world.insert_resource(slots);
    }

    /// Un pas de migration : chaque tile inerte à portée d'un stimulus
    /// envoie une part de ses zombies vers la voisine la plus proche du
    /// stimulus. Calculé sur les effectifs du début du pas (ordre sans
    /// importance), puis appliqué dans l'ordre des tiles.
    fn migrer(&mut self, tick: u64) {
        let t0 = Instant::now();
        let geo = self.world.resource::<Topo>().geo;
        let mut mouvements = vec![];
        for t in 0..geo.n() {
            let n = match &self.tuiles[t].stock {
                Stock::Compteur(n) | Stock::Sauvee { n, .. } => *n,
                Stock::Vivante => continue,
            };
            let o = geo.origine(t);
            let c = [o[0] + 500.0, o[1] + 500.0];
            for &(s, portee) in &self.p.stimuli {
                let d = [s[0] - c[0], s[1] - c[1]];
                if d[0].hypot(d[1]) > portee {
                    continue;
                }
                let sg = |v: f64| if v > 500.0 { 1 } else if v < -500.0 { -1 } else { 0 };
                let (dx, dy) = (sg(d[0]), sg(d[1]));
                if (dx, dy) == (0, 0) {
                    continue;
                }
                let k = (n as f32 * self.p.taux_migration).floor() as u32;
                if let (Some(v), true) = (geo.voisine(t, dx, dy), k > 0) {
                    let nv = match &self.tuiles[v].stock {
                        Stock::Compteur(n) | Stock::Sauvee { n, .. } => *n as u64,
                        Stock::Vivante => self.compte_vivante(v) as u64,
                    };
                    let k = k.min(self.p.saturation_migration.saturating_sub(nv.min(u32::MAX as u64) as u32));
                    if k > 0 {
                        mouvements.push((t, v, dx, dy, k));
                    } else {
                        self.stats.migration_saturee += 1;
                    }
                }
                break;
            }
        }
        for (t, v, dx, dy, k) in mouvements {
            let k = match &mut self.tuiles[t].stock {
                Stock::Compteur(n) => {
                    let k = k.min(*n);
                    *n -= k;
                    k
                }
                Stock::Sauvee { n, journal, .. } => {
                    let k = k.min(*n);
                    *n -= k;
                    journal.push(Mouvement::Depart { cote: Cote { dx: dx as i8, dy: dy as i8 }, k });
                    k
                }
                Stock::Vivante => continue,
            };
            let entree = Cote { dx: -dx as i8, dy: -dy as i8 };
            let graine = hacher(tick, (t as u64) << 32 | v as u64);
            self.stats.migres += k as u64;
            match &mut self.tuiles[v].stock {
                Stock::Compteur(n) => *n += k,
                Stock::Sauvee { n, journal, .. } => {
                    *n += k;
                    journal.push(Mouvement::Arrivee { cote: entree, k, graine });
                }
                Stock::Vivante => {
                    let arr = sauvegarde::arrivees(self.carte(), v, entree, k, graine);
                    let etat = self.world.resource::<Topo>().etat[v];
                    match etat {
                        EtatTuile::Actif(s) => {
                            self.faire_entrer(s as usize, &arr);
                            self.stats.arrivees_actives += k as u64;
                        }
                        EtatTuile::Chantier(s) => {
                            let mut slots = self.world.resource_mut::<Slots>();
                            slots.0[s as usize].chantier.as_mut().unwrap().attente.extend(arr);
                        }
                        EtatTuile::Inerte => unreachable!(),
                    }
                }
            }
        }
        self.stats.migration_ms += t0.elapsed().as_secs_f64() * 1e3;
    }

    /// Zombies d'une tile active ou en chantier.
    fn compte_vivante(&self, t: usize) -> usize {
        match self.world.resource::<Topo>().etat[t] {
            EtatTuile::Actif(s) => self.slots()[s as usize].vivants,
            EtatTuile::Chantier(s) => self.slots()[s as usize].chantier.as_ref().unwrap().compte(),
            EtatTuile::Inerte => 0,
        }
    }

    /// Fait naître des zombies (errants) dans un slot actif.
    fn faire_entrer(&mut self, k: usize, arr: &[([f32; 2], f32)]) {
        let origine = self.world.resource::<Slots>().0[k].origine;
        for &(p, cap) in arr {
            let e = self
                .world
                .spawn((
                    Corps { pos: [origine[0] + p[0] as f64, origine[1] + p[1] as f64], cap },
                    Cerveau { etat: ERRANCE, cible: [0.0; 2], alerte: 0, perdu_depuis: 0 },
                    Ident { slot: k as u32, rang: 0 },
                ))
                .id();
            let mut slots = self.world.resource_mut::<Slots>();
            let s = &mut slots.0[k];
            let r = s.allouer();
            s.installer(r, e, p, ERRANCE);
            drop(slots);
            self.world.get_mut::<Ident>(e).unwrap().rang = r as u32;
        }
    }

    /// Phase 1 (début) : déplacement des humains, puis tirs.
    fn humains_et_tirs(&mut self) {
        let tick = self.horloge();
        let vp = self.p.vitesse_promenade;
        let mut h = self.world.remove_resource::<Humains>().unwrap();
        h.deplacer(self.carte(), tick, vp);
        if h.tir.iter().any(|&t| t) {
            self.tirer(&h, tick);
        }
        self.world.insert_resource(h);
    }

    /// Chaque tireur, une fois par période, abat avec une probabilité donnée
    /// le zombie le plus proche à portée (grilles du tick précédent).
    fn tirer(&mut self, h: &Humains, tick: u64) {
        let slots = &self.world.resource::<Slots>().0;
        let topo = self.world.resource::<Topo>();
        let carte = self.world.resource::<Carte>();
        let geo = topo.geo;
        let per = self.p.periode_tir.max(1) as u64;
        let portee = self.p.portee_tir;
        for i in 0..h.pos.len() {
          // P-05 : un point peut porter plusieurs tireurs (bâtiment occupé).
          for coup in 0..h.tireurs[i] as u64 {
            if !h.actif[i] || !h.tir[i] || (tick + i as u64) % per != 0
                || unite(hacher(0x7123 ^ coup << 48, (i as u64) << 32 | tick)) >= self.p.proba_tir
            {
                continue;
            }
            let Some(t) = geo.tile_de(h.pos[i]) else { continue };
            let EtatTuile::Actif(s) = topo.etat[t] else { continue };
            let s = s as usize;
            let env = Env { slots, topo, carte, s };
            let pos = slots[s].local(h.pos[i]);
            let (cx, cy) = env.case_de(pos);
            let rc = (portee / slots[s].vois.taille).ceil() as i32;
            let mut best: Option<(f32, usize, u32)> = None;
            for y in cy - rc..=cy + rc {
                for x in cx - rc..=cx + rc {
                    if let Some((v, x2, y2, off, _)) = env.resoudre(x, y) {
                        for e in slots[v].vois.case(x2, y2) {
                            // Grille du tick précédent : le zombie a pu changer de
                            // tile depuis (rang libéré) ; on ne vise qu'un rang
                            // occupé et indexé.
                            let r = e.id as usize;
                            if slots[v].abattu[r].load(Relaxed) != 0
                                || slots[v].entites.get(r).is_none_or(|x| x.is_none())
                                || slots[v].indexe[r].load(Relaxed) == 0
                            {
                                continue;
                            }
                            let dd = d2([e.p[0] + off[0], e.p[1] + off[1]], pos);
                            if dd <= portee * portee && best.is_none_or(|b| (dd, v, e.id) < b) {
                                best = Some((dd, v, e.id));
                            }
                        }
                    }
                }
            }
            if let Some((_, v, r)) = best {
                slots[v].abattu[r as usize].store(1, Relaxed);
                slots[v].indexe[r as usize].store(0, Relaxed);
                self.abattus_tick.push((v as u32, r));
            }
          }
        }
    }

    // ------------------------------------------------------------ P-05

    /// Abat un zombie (combat auto-résolu dans un bâtiment) : corps et
    /// destruction de l'entité en fin de tick, comme un tir.
    pub fn abattre(&mut self, slot: u32, rang: u32) {
        let s = &self.world.resource::<Slots>().0[slot as usize];
        if s.abattu[rang as usize].swap(1, Relaxed) == 0 {
            s.indexe[rang as usize].store(0, Relaxed);
            self.abattus_tick.push((slot, rang));
        }
    }

    /// Fait naître un zombie (colon zombifié) dans un slot actif, à la
    /// position locale `p`, dans l'état `etat`. Le total attendu de la
    /// conservation augmente d'autant.
    pub fn zombifier(&mut self, k: usize, p: [f32; 2], etat: u8) -> Entity {
        let origine = self.world.resource::<Slots>().0[k].origine;
        let e = self
            .world
            .spawn((
                Corps { pos: [origine[0] + p[0] as f64, origine[1] + p[1] as f64], cap: 0.0 },
                Cerveau { etat, cible: [0.0; 2], alerte: 0, perdu_depuis: 0 },
                Ident { slot: k as u32, rang: 0 },
            ))
            .id();
        let mut slots = self.world.resource_mut::<Slots>();
        let s = &mut slots.0[k];
        let r = s.allouer();
        s.installer(r, e, p, etat);
        drop(slots);
        self.world.get_mut::<Ident>(e).unwrap().rang = r as u32;
        self.total_initial += 1;
        e
    }

    /// Phase 7 (exclusive) : abattus, changements de tile, bordures.
    fn frontieres(&mut self) {
        let mut slots = self.world.remove_resource::<Slots>().unwrap();
        // Abattus : corps comptés, entités détruites.
        let mut ab = std::mem::take(&mut self.abattus_tick);
        ab.sort_unstable();
        for &(s, r) in &ab {
            let sl = &mut slots.0[s as usize];
            let p = decoder(sl.inst[r as usize].load(Relaxed));
            let (cx, cy) = (
                ((p[0] / 10.0) as usize).min(COTE_CORPS - 1),
                ((p[1] / 10.0) as usize).min(COTE_CORPS - 1),
            );
            sl.corps[cy * COTE_CORPS + cx] = sl.corps[cy * COTE_CORPS + cx].saturating_add(1);
            let e = sl.entites[r as usize].unwrap();
            sl.liberer(r as usize);
            self.world.despawn(e);
            self.stats.abattus += 1;
        }
        // Changements de tile.
        let geo = self.world.resource::<Topo>().geo;
        let etats = self.world.resource::<Topo>().etat.clone();
        let mut passages = 0u64;
        for k in 0..slots.0.len() {
            if slots.0[k].etat != EtatSlot::Actif {
                slots.0[k].nb_sortants.store(0, Relaxed);
                continue;
            }
            let nb = slots.0[k].nb_sortants.swap(0, Relaxed) as usize;
            if nb == 0 {
                continue;
            }
            let mut l: Vec<u32> = slots.0[k].sortants[..nb].iter().map(|a| a.load(Relaxed)).collect();
            l.sort_unstable();
            l.dedup();
            for r in l {
                let r = r as usize;
                let Some(e) = slots.0[k].entites[r] else { continue };
                let pos = self.world.get::<Corps>(e).unwrap().pos;
                let Some(dest) = geo.tile_de(pos) else { continue };
                if dest == slots.0[k].tile {
                    continue;
                }
                passages += 1;
                let o_src = slots.0[k].origine;
                let o_dst = geo.origine(dest);
                let loc = [(pos[0] - o_dst[0]) as f32, (pos[1] - o_dst[1]) as f32];
                match etats[dest] {
                    EtatTuile::Actif(s2) if slots.0[s2 as usize].libres.is_empty() && slots.0[s2 as usize].n_haut >= CAP => {
                        // Slot plein : le zombie reste chez lui, ramené au bord.
                        let l = slots.0[k].local(pos);
                        let q = [l[0].clamp(0.0, COTE_F - 0.01), l[1].clamp(0.0, COTE_F - 0.01)];
                        self.world.get_mut::<Corps>(e).unwrap().pos = [o_src[0] + q[0] as f64, o_src[1] + q[1] as f64];
                        slots.0[k].inst[r].store(coder(q), Relaxed);
                        self.stats.passages_refuses += 1;
                    }
                    EtatTuile::Actif(s2) => {
                        let s2 = s2 as usize;
                        let dec = [(o_src[0] - o_dst[0]) as f32, (o_src[1] - o_dst[1]) as f32];
                        let (h, cible) = (
                            slots.0[k].horodatage[r].load(Relaxed),
                            decoder(slots.0[k].cible_al[r].load(Relaxed)),
                        );
                        slots.0[k].liberer(r);
                        let r2 = slots.0[s2].allouer();
                        let etat = self.world.get::<Cerveau>(e).unwrap().etat;
                        slots.0[s2].installer(r2, e, loc, etat);
                        slots.0[s2].horodatage[r2].store(h, Relaxed);
                        slots.0[s2].cible_al[r2].store(coder([cible[0] + dec[0], cible[1] + dec[1]]), Relaxed);
                        *self.world.get_mut::<Ident>(e).unwrap() = Ident { slot: s2 as u32, rang: r2 as u32 };
                        let mut c = self.world.get_mut::<Cerveau>(e).unwrap();
                        c.cible = [c.cible[0] + dec[0], c.cible[1] + dec[1]];
                    }
                    EtatTuile::Chantier(s2) => {
                        let cap = self.world.get::<Corps>(e).unwrap().cap;
                        slots.0[k].liberer(r);
                        self.world.despawn(e);
                        slots.0[s2 as usize].chantier.as_mut().unwrap().attente.push((loc, cap));
                        self.stats.vers_chantier += 1;
                    }
                    EtatTuile::Inerte => {
                        let cap = self.world.get::<Corps>(e).unwrap().cap;
                        slots.0[k].liberer(r);
                        self.world.despawn(e);
                        match &mut self.tuiles[dest].stock {
                            Stock::Compteur(n) => *n += 1,
                            Stock::Sauvee { n, journal, .. } => {
                                *n += 1;
                                journal.push(Mouvement::Exacte { pos: loc, cap });
                            }
                            Stock::Vivante => unreachable!("tile inerte sans stock"),
                        }
                        self.stats.vers_inerte += 1;
                    }
                }
            }
        }
        self.stats.passages += passages;
        self.stats.passages_max_tick = self.stats.passages_max_tick.max(passages);
        // Bordures publiées (lues au prochain démarrage de calcul).
        let mut b = self.world.remove_resource::<Bordures>().unwrap();
        for (k, s) in slots.0.iter_mut().enumerate() {
            if let Some(e) = s.export_pret.take() {
                b.0[k] = Some(e);
            }
        }
        self.world.insert_resource(b);
        self.world.insert_resource(slots);
    }

    // ------------------------------------------------------------ Bilans

    /// Conservation : (attendu, compté par les stocks, compté dans l'ECS,
    /// attendu dans l'ECS).
    pub fn conservation(&mut self) -> (u64, u64, u64, u64) {
        let mut stocks = 0u64;
        for t in &self.tuiles {
            match &t.stock {
                Stock::Compteur(n) | Stock::Sauvee { n, .. } => stocks += *n as u64,
                Stock::Vivante => {}
            }
        }
        let mut ecs_attendu = 0u64;
        for s in &self.world.resource::<Slots>().0 {
            match s.etat {
                EtatSlot::Actif => {
                    stocks += s.vivants as u64;
                    ecs_attendu += s.vivants as u64;
                }
                EtatSlot::Chantier => {
                    let ch = s.chantier.as_ref().unwrap();
                    stocks += ch.compte() as u64;
                    ecs_attendu += ch.naissance as u64;
                }
                EtatSlot::Destruction => ecs_attendu += s.a_detruire.len() as u64,
                EtatSlot::Libre => {}
            }
        }
        let ecs = self.world.query::<&Ident>().iter(&self.world).count() as u64;
        (self.total_initial, stocks + self.stats.abattus, ecs, ecs_attendu)
    }

    pub fn verifier_conservation(&mut self) -> bool {
        let (a, b, c, d) = self.conservation();
        let ok = a == b && c == d;
        if !ok {
            self.stats.conservation_ko += 1;
            eprintln!("conservation fausse au tick {} : attendu {a}, compté {b}, ECS {c} (attendu {d})", self.horloge());
        }
        ok
    }

    /// Somme de contrôle de tout l'état : entités, stocks, humains,
    /// constructions, corps.
    pub fn somme_controle(&mut self) -> u64 {
        let mut h = 0u64;
        let slots_tile: Vec<usize> = self.world.resource::<Slots>().0.iter().map(|s| s.tile).collect();
        let mut q = self.world.query::<(&Corps, &Cerveau, &Ident)>();
        for (c, cv, id) in q.iter(&self.world) {
            let x = hacher(c.pos[0].to_bits(), c.pos[1].to_bits() ^ c.cap.to_bits() as u64);
            let y = hacher(cv.etat as u64, (slots_tile[id.slot as usize] as u64) << 32 | id.rang as u64);
            h = h.wrapping_add(hacher(x, y));
        }
        for (t, tu) in self.tuiles.iter().enumerate() {
            let v = match &tu.stock {
                Stock::Compteur(n) => hacher(t as u64, *n as u64),
                Stock::Sauvee { n, somme, journal, .. } => hacher(hacher(t as u64, *n as u64), *somme ^ journal.len() as u64),
                Stock::Vivante => hacher(t as u64, u64::MAX),
            };
            h = h.wrapping_add(v);
        }
        for p in &self.humains().pos {
            h = h.wrapping_add(hacher(p[0].to_bits(), p[1].to_bits()));
        }
        for (t, tu) in self.carte().tuiles.iter().enumerate() {
            for b in &tu.barricades {
                h = h.wrapping_add(hacher(t as u64, (b.x as u64) << 48 | (b.y as u64) << 32 | b.pv.load(Relaxed) as u32 as u64));
            }
        }
        for s in &self.world.resource::<Slots>().0 {
            if s.etat == EtatSlot::Actif {
                for (i, &c) in s.corps.iter().enumerate() {
                    if c != 0 {
                        h = h.wrapping_add(hacher(s.tile as u64, (i as u64) << 16 | c as u64));
                    }
                }
            }
        }
        h
    }

    /// Attend la fin de toutes les écritures (fin de mesure).
    pub fn finir_ecritures(&mut self) {
        for t in &mut self.tuiles {
            if let Stock::Sauvee { donnees, .. } = &mut t.stock
                && let Donnees::Memoire(ts, h) = donnees
                && let Some(h) = h.take()
            {
                let e = h.join().expect("écriture");
                self.stats.encode_ms.push(e.encode_ns as f64 / 1e6);
                self.stats.ecriture_ms.push(e.ecriture_ns as f64 / 1e6);
                self.stats.octets.push(e.octets);
                let (x, y) = (ts.x_km, ts.y_km);
                *donnees = Donnees::Disque(self.p.dossier.join(format!("x{x}_y{y}.tile")));
            }
        }
    }

    /// Effectifs par état (tiles actives).
    #[allow(dead_code)]
    pub fn etats(&mut self) -> [usize; 6] {
        let mut e = [0usize; 6];
        let mut q = self.world.query::<&Cerveau>();
        for c in q.iter(&self.world) {
            e[c.etat as usize] += 1;
        }
        e
    }

    pub fn nb_actifs(&self) -> usize {
        self.slots().iter().filter(|s| s.etat == EtatSlot::Actif).count()
    }
    pub fn nb_chantiers(&self) -> usize {
        self.slots().iter().filter(|s| s.etat == EtatSlot::Chantier).count()
    }
}
