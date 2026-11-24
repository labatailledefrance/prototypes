//! Simulation des zombies d'une ou plusieurs tiles actives, avec `bevy_ecs`
//! (exécuteur multi-thread, requêtes `par_iter_mut`). Reprise de P-02
//! (arbre k-d des humains, relais filtré, veille, veille profonde) ; P-03 y
//! ajoute :
//! - les **flow fields** (`champ.rs`) : un champ « proies » à sources
//!   multiples (les humains observés depuis moins d'une minute), recalculé
//!   périodiquement, et un champ « bruits », recalculé quand les bruits
//!   changent ; recalcul partiel ou complet quand un obstacle apparaît ;
//! - le choix de l'**index des zombies** (`voisinage.rs`) ;
//! - la **réservation exacte** des places sur les cases au contact
//!   (A-05), déterministe : demandes triées, puis admises dans l'ordre ;
//! - les scénarios **repli** (humains qui traversent la tile) et
//!   **barricade** (un anneau de barricades posé pendant un assaut).
//!
//! Un tick = six phases, chronométrées séparément :
//! 0. **humains** : déplacement, arbre k-d, carte de proximité ;
//! 1. **grilles** : index des zombies, cases au contact ;
//! 2. **champs** : obstacles nouveaux, recalcul des champs dus ;
//! 3. **perception** : relais, ouïe, vue, bruits, dernière position ;
//! 4. **déplacement** : descente du champ, séparation, murs, capacité ;
//! 5. **admission** : réservations sur les cases au contact.
//!
//! Chaque phase ne lit que l'état de la phase précédente et n'écrit que
//! l'entité (ou la tile) qu'elle traite : le résultat ne dépend ni du nombre
//! de threads, ni du découpage en lots, ni de l'index des zombies.

use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use bevy_ecs::batching::BatchingStrategy;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::MultiThreadedExecutor;
use bevy_tasks::{ComputeTaskPool, TaskPoolBuilder};
use clap::ValueEnum;

use crate::alea::{Alea, hacher, unite};
use crate::champ::{self, Champ, Dial, Graine, Regle, Terrain};
use crate::compteurs;
use crate::index::{Compte, Index, Proximite, RAYONS_MAX, Requete, Structure};
use crate::ville::{COTE, COTE_F, Ville};
use crate::voisinage::{self, Voisinage};

/// Pas de temps : 20 ticks par seconde de jeu.
pub const DT: f32 = 0.05;
/// Côté d'une case de la carte de proximité des humains.
const CASE_PROXIMITE: f32 = 25.0;
/// Nombre maximal de voisins examinés pour la séparation et le relais.
const VOISINS_MAX: u32 = 16;
/// Rayon du groupe d'humains des scénarios calme et assaut.
pub const RAYON_GROUPE: f32 = 60.0;
/// Rayon de la zone d'arrivée du repli.
const RAYON_ARRIVEE: f32 = 40.0;
/// Distance sous laquelle un bruit atteint n'attire plus.
const ARRIVEE_BRUIT: f32 = 15.0;
/// Sous cette distance de sa cible, un zombie va droit sur elle.
const APPROCHE: f32 = 15.0;
/// Rayon de l'anneau de barricades autour du groupe.
pub const RAYON_BARRICADE: f32 = 80.0;
/// Origine de la première tile, en Lambert-93 (Paris).
const ORIGINE: [f64; 2] = [650_000.0, 6_860_000.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Scenario {
    /// 1 000 humains regroupés au centre, aucun bruit.
    Calme,
    /// Idem, plus des tirs continus au centre entendus à 600 m.
    Assaut,
    /// 1 000 humains répartis dans toute la tile.
    Disperse,
    /// Les 1 000 humains du groupe se replient vers un point à environ
    /// 600 m de marche, en colonne, à 3 m/s.
    Repli,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ModeChamp {
    /// Ligne droite vers la cible (P-01, test 1).
    Aucun,
    /// Flow fields.
    Flow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Reservation {
    /// Admission progressive partout (P-01, P-02).
    Aucune,
    /// Réservation exacte sur les cases au contact (barricade, humains) ;
    /// admission progressive ailleurs (décision A-05).
    Contact,
    /// Réservation exacte sur toutes les cases.
    Partout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Recalcul {
    /// Recalcul partiel après un obstacle (descendants invalidés).
    Partiel,
    /// Recalcul complet.
    Complet,
}

/// Paramètres de comportement. Ce sont des **hypothèses de prototype**, pas
/// des statistiques de jeu (qui restent qualitatives dans la documentation).
#[derive(Clone)]
pub struct Params {
    pub vitesse_course: f32,
    pub vitesse_errance: f32,
    pub vitesse_humains: f32,
    pub vitesse_repli: f32,
    pub distance_repli: f32,
    pub humains_mobiles: bool,
    pub rayon_ouie: f32,
    pub distance_vue: f32,
    pub demi_angle_vue_deg: f32,
    pub distance_separation: f32,
    pub rayon_bruit: f32,
    /// Zombies au plus dans une case de 2 m × 2 m (mise à l'échelle pour
    /// d'autres tailles de case).
    pub capacite_case: u32,
    pub periode_perception: u32,
    pub rayons: usize,
    pub structure: Structure,
    pub case: f32,
    pub rejet: bool,
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
    // ---- P-03
    pub mode_champ: ModeChamp,
    /// Côté d'une case de champ (m) : 1, 2, 4 ou 5.
    pub resolution: usize,
    pub regle: Regle,
    /// Recalcul du champ « proies » tous les N ticks (étalé entre tiles).
    pub periode_champ: u32,
    pub recalcul: Recalcul,
    pub voisinage: voisinage::Structure,
    pub case_zombies: f32,
    pub reservation: Reservation,
    /// Coût de marge le long des murs (champs).
    pub marge: bool,
    /// Calcul du champ « proies » étalé sur la période (double tampon).
    pub etalement: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            vitesse_course: 5.0,
            vitesse_errance: 1.0,
            vitesse_humains: 1.5,
            vitesse_repli: 3.0,
            distance_repli: 600.0,
            humains_mobiles: true,
            rayon_ouie: 40.0,
            distance_vue: 400.0,
            demi_angle_vue_deg: 60.0,
            distance_separation: 0.7,
            rayon_bruit: 600.0,
            capacite_case: 24,
            periode_perception: 5,
            rayons: 4,
            structure: Structure::Kdtree,
            case: 25.0,
            rejet: true,
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
            mode_champ: ModeChamp::Flow,
            resolution: 2,
            regle: Regle::Majorite,
            periode_champ: 20,
            recalcul: Recalcul::Partiel,
            voisinage: voisinage::Structure::Dense16,
            case_zombies: 2.0,
            reservation: Reservation::Partout,
            marge: true,
            etalement: true,
        }
    }
}

#[derive(Clone)]
pub enum SourceVille {
    Synthetique,
    /// Dossier produit par `p01-tile-paris/outils/extraire_tiles.py`.
    Reelle(std::path::PathBuf),
}

pub struct ConfigSim {
    pub source: SourceVille,
    pub scenario: Scenario,
    pub tiles: usize,
    pub zombies: usize,
    pub humains: usize,
    pub threads: usize,
    pub lot_min: usize,
    pub params: Params,
}

// ---------------------------------------------------------------- Composants

#[derive(Component)]
struct Corps {
    pos: [f64; 2],
    cap: f32,
}

pub const ERRANCE: u8 = 0;
pub const POURSUITE: u8 = 1;
pub const ATTIRE: u8 = 2;
pub const RELAIS: u8 = 3;
pub const PERDU: u8 = 4;
pub const PROFOND: u8 = 5;

#[derive(Component)]
struct Cerveau {
    etat: u8,
    cible: [f32; 2],
    alerte: u32,
    perdu_depuis: u32,
}

#[derive(Component)]
struct Ident {
    tile: u32,
    rang: u32,
}

// ---------------------------------------------------------------- Ressources

pub struct Humains {
    pub pos: Vec<[f32; 2]>,
    cap: Vec<f32>,
    index: Index,
    proximite: Proximite,
    /// Tick (+1) de la dernière observation directe par un zombie (0 =
    /// jamais), et position de l'humain à ce moment : sources du champ.
    obs_tick: Vec<AtomicU32>,
    obs_pos: Vec<AtomicU64>,
}

/// Le repli : champ de marche des humains vers leur destination (1 m).
pub struct Repli {
    pub terrain: Terrain,
    pub champ: Champ,
    pub dest: [f32; 2],
}

#[derive(Default, Clone, Copy, Debug)]
pub struct StatChamps {
    pub recalculs: u64,
    pub ns: u64,
    pub fixees: u64,
    pub obstacles: u64,
    pub obstacle_ns: u64,
    pub invalidees: u64,
    pub max_ns: u64,
}

pub struct Tile {
    pub origine: [f64; 2],
    pub ville: Ville,
    pub centre: [f32; 2],
    /// Cases de 1 m de la plus grande zone de rues connexe (bits).
    zone: Vec<u64>,
    pub humains: Humains,
    pub bruits: Vec<[f32; 2]>,
    version_bruits: u64,
    pub terrain: Terrain,
    pub proies: Champ,
    /// Champ « proies » en cours de calcul (étalement).
    proies_suivant: Champ,
    sources_suivantes: Vec<Graine>,
    /// Cases à fixer par tick pour finir le calcul en une période.
    budget: u64,
    ns_cours: u64,
    pub champ_bruits: Champ,
    dial: Dial,
    sources_proies: Vec<Graine>,
    sources_bruits: Vec<Graine>,
    pub repli: Option<Repli>,
    /// Cases de 1 m à barricader au prochain tick.
    barricade_en_attente: Vec<(u16, u16)>,
    /// Cases de l'index des zombies au contact d'une barricade (bits).
    contact_fixe: Vec<u64>,
    pub stat: StatChamps,
}

#[derive(Resource)]
pub struct Tiles {
    pub tiles: Vec<Tile>,
    zombies_par_tile: usize,
    scenario: Scenario,
}

pub struct Occupation {
    pub vois: Voisinage,
    /// Cases au contact (barricade, humains) : réservation exacte (bits).
    pub contact: Vec<u64>,
}

#[derive(Resource)]
struct GrillesZombies(Vec<Occupation>);

#[derive(Resource)]
struct Instantane(Vec<AtomicU64>);

#[derive(Resource)]
struct Alertes {
    horodatage: Vec<AtomicU32>,
    cible: Vec<AtomicU64>,
    indexe: Vec<AtomicU8>,
}

/// Réservations du tick : demandes d'entrée dans une case au contact, par
/// tile (ordre d'écriture quelconque, trié avant décision), position
/// souhaitée et état de chaque zombie (0 rien, 1 en attente, 2 admis).
#[derive(Resource)]
struct Reservations {
    demandes: Vec<Vec<AtomicU64>>,
    nb: Vec<AtomicU32>,
    souhait: Vec<AtomicU64>,
    attente: Vec<AtomicU8>,
    /// Demandes et admissions du dernier tick (toutes tiles).
    total_demandes: AtomicU64,
    total_admis: AtomicU64,
}

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
fn local(p: [f64; 2], o: [f64; 2]) -> [f32; 2] {
    [(p[0] - o[0]) as f32, (p[1] - o[1]) as f32]
}
#[inline(always)]
pub fn d2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}
#[inline(always)]
fn bit(b: &[u64], k: usize) -> bool {
    (b[k >> 6] >> (k & 63)) & 1 == 1
}
#[inline(always)]
fn poser_bit(b: &mut [u64], k: usize) {
    b[k >> 6] |= 1 << (k & 63);
}

const VOISINS8: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// Case libre la plus proche de `p` dans la plus grande zone connexe de rues
/// (les cours fermées sont exclues, cf. P-01).
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
                if v.bati(nx, ny)
                    || (dx != 0 && dy != 0 && (v.bati(x + dx, y) || v.bati(x, y + dy)))
                {
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
                if (dx.abs() == r || dy.abs() == r)
                    && !v.bati(x, y)
                    && zone[y as usize * COTE + x as usize] == meilleure
                {
                    return [x as f32 + 0.5, y as f32 + 0.5];
                }
            }
        }
    }
    unreachable!("la plus grande zone n'est pas vide")
}

/// Champ exact à 1 m depuis des points (oracle des mesures de crédibilité,
/// zone accessible, champ de repli).
pub fn champ_1m(v: &Ville, sources: &[[f32; 2]]) -> (Terrain, Champ) {
    let t = Terrain::de_ville(v, 1, Regle::Majorite, false);
    let mut c = Champ::new(&t);
    let mut d = Dial::new(&t);
    let g: Vec<Graine> = sources
        .iter()
        .map(|&p| Graine {
            i: t.indice(p) as u32,
            d: 0,
            ts: 0,
        })
        .collect();
    d.calculer(&t, &mut c, &g);
    (t, c)
}

#[inline(always)]
fn accessible(v: &Ville, zone: &[u64], q: [f32; 2]) -> bool {
    !v.bati_p(q) && bit(zone, q[1] as usize * COTE + q[0] as usize)
}

/// Case libre la plus proche (zombie ou humain pris dans un obstacle neuf).
#[inline(never)]
fn expulser(v: &Ville, p: [f32; 2]) -> [f32; 2] {
    let (x0, y0) = (p[0] as i32, p[1] as i32);
    for r in 1..=8i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                if (dx.abs() == r || dy.abs() == r) && !v.bati(x0 + dx, y0 + dy) {
                    return [(x0 + dx) as f32 + 0.5, (y0 + dy) as f32 + 0.5];
                }
            }
        }
    }
    p
}

/// Cases libres (zone accessible) de l'anneau [r, r + 2 m[ autour de
/// `centre`, regroupées en composantes connexes : une par rue traversée,
/// avec l'angle de son barycentre.
pub fn composantes_anneau(v: &Ville, zone: &[u64], centre: [f32; 2], r0: f32) -> Vec<(f32, Vec<(i32, i32)>)> {
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
    let mut comps = vec![];
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
    comps
}

/// Écart angulaire entre deux angles, dans [0, π].
fn ecart_angle(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(std::f32::consts::TAU);
    d.min(std::f32::consts::TAU - d)
}

/// Anneau de barricades autour du groupe : toutes les rues qui traversent
/// le cercle de 80 m sont barrées, sauf deux (les plus proches de l'est et
/// de l'ouest). Épaisseur 2 m.
pub fn anneau_barricades(v: &Ville, zone: &[u64], centre: [f32; 2]) -> Vec<(u16, u16)> {
    let comps = composantes_anneau(v, zone, centre, RAYON_BARRICADE);
    let ouverte = |cible: f32| {
        (0..comps.len())
            .min_by(|&a, &b| ecart_angle(comps[a].0, cible).total_cmp(&ecart_angle(comps[b].0, cible)))
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

/// Une seule rue barrée : la traversée du cercle de rayon `r0` la plus
/// proche de l'est.
pub fn barricade_rue(v: &Ville, zone: &[u64], centre: [f32; 2], r0: f32) -> Vec<(u16, u16)> {
    let comps = composantes_anneau(v, zone, centre, r0);
    comps
        .iter()
        .min_by(|a, b| ecart_angle(a.0, 0.0).total_cmp(&ecart_angle(b.0, 0.0)))
        .map(|c| c.1.iter().map(|&(x, y)| (x as u16, y as u16)).collect())
        .unwrap_or_default()
}

/// Zone accessible (bits) depuis un champ exact à 1 m.
pub fn zone_de(t1: &Terrain, c1: &Champ) -> Vec<u64> {
    let mut zone = vec![0u64; (COTE * COTE).div_ceil(64)];
    for y in 0..COTE as i32 {
        for x in 0..COTE as i32 {
            if c1.dist[t1.ixy(x, y)] != champ::INF {
                poser_bit(&mut zone, y as usize * COTE + x as usize);
            }
        }
    }
    zone
}

pub fn dans_zone(zone: &[u64], q: [f32; 2]) -> bool {
    bit(zone, q[1] as usize * COTE + q[0] as usize)
}

// ---------------------------------------------------------------- Systèmes

/// Phase 0 : humains.
fn humains(mut t: ResMut<Tiles>, r: Res<Reglages>, h: Res<Horloge>) {
    if !r.p.humains_mobiles {
        return;
    }
    let tick = h.0;
    let sc = t.scenario;
    let p = &r.p;
    ComputeTaskPool::get().scope(|s| {
        for (k, tile) in t.tiles.iter_mut().enumerate() {
            s.spawn(async move {
                let n = tile.humains.pos.len();
                for i in 0..n {
                    let g = hacher((k as u64) << 32 | i as u64, tick);
                    let mut cap = tile.humains.cap[i];
                    let mut pos = tile.humains.pos[i];
                    if tile.ville.bati_p(pos) {
                        pos = expulser(&tile.ville, pos);
                    }
                    // Repli : descente du champ de marche, en colonne.
                    let mut vitesse = p.vitesse_humains;
                    let mut disque = match sc {
                        Scenario::Disperse => None,
                        Scenario::Repli => None,
                        _ => Some((tile.centre, RAYON_GROUPE)),
                    };
                    let mut libre_errance = true;
                    if let Some(rp) = &tile.repli {
                        if d2(pos, rp.dest) > RAYON_ARRIVEE * RAYON_ARRIVEE {
                            if let Some(d) = rp.champ.direction(&rp.terrain, pos) {
                                cap = d[1].atan2(d[0]) + (unite(g) - 0.5) * 0.8;
                                vitesse = p.vitesse_repli;
                                libre_errance = false;
                            }
                        } else {
                            disque = Some((rp.dest, RAYON_ARRIVEE));
                        }
                    }
                    if libre_errance && (tick + i as u64) % 40 == 0 {
                        cap += (unite(g) - 0.5) * 2.0;
                    }
                    let pas = vitesse * DT;
                    let np = [pos[0] + cap.cos() * pas, pos[1] + cap.sin() * pas];
                    let ok = accessible(&tile.ville, &tile.zone, np)
                        && disque.is_none_or(|(c, r)| d2(np, c) <= r * r);
                    if ok {
                        pos = np;
                    } else if libre_errance {
                        cap += std::f32::consts::FRAC_PI_2 + unite(hacher(g, 1)) * std::f32::consts::PI;
                    }
                    tile.humains.pos[i] = pos;
                    tile.humains.cap[i] = cap;
                }
                let hu = &mut tile.humains;
                hu.index.reconstruire(&hu.pos);
                hu.proximite.reconstruire(&hu.pos);
            });
        }
    });
}

/// Phase 1 : index des zombies et cases au contact.
fn construire_grilles(
    mut g: ResMut<GrillesZombies>,
    inst: Res<Instantane>,
    al: Res<Alertes>,
    t: Res<Tiles>,
    r: Res<Reglages>,
) {
    let n = t.zombies_par_tile;
    let inst = &inst.0;
    let al = &*al;
    let reservation = r.p.reservation != Reservation::Aucune;
    let t = &*t;
    ComputeTaskPool::get().scope(|s| {
        for (k, occ) in g.0.iter_mut().enumerate() {
            s.spawn(async move {
                let base = k * n;
                occ.vois.reconstruire_avec(
                    n,
                    |i| al.indexe[base + i].load(Relaxed) != 0,
                    |i| decoder(inst[base + i].load(Relaxed)),
                    |i| {
                        (
                            al.horodatage[base + i].load(Relaxed),
                            decoder(al.cible[base + i].load(Relaxed)),
                        )
                    },
                );
                if reservation {
                    // Contact : barricades (fixe), et 3 × 3 cases autour de
                    // chaque humain.
                    let tile = &t.tiles[k];
                    occ.contact.copy_from_slice(&tile.contact_fixe);
                    let cote = occ.vois.cote as i32;
                    for &hp in &tile.humains.pos {
                        let (x, y) = occ.vois.coord(hp);
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let (a, b) = (x + dx, y + dy);
                                if a >= 0 && b >= 0 && a < cote && b < cote {
                                    poser_bit(&mut occ.contact, (b * cote + a) as usize);
                                }
                            }
                        }
                    }
                }
            });
        }
    });
}

/// Phase 2 : obstacles nouveaux et champs dus.
fn champs(mut t: ResMut<Tiles>, r: Res<Reglages>, h: Res<Horloge>, g: Res<GrillesZombies>) {
    let tick = h.0;
    let p = &r.p;
    let maintenant = tick as u32 + 1;
    let g = &*g;
    ComputeTaskPool::get().scope(|s| {
        for (k, tile) in t.tiles.iter_mut().enumerate() {
            let vois_taille = g.0[k].vois.taille;
            let vois_cote = g.0[k].vois.cote;
            s.spawn(async move {
                // Obstacle neuf : bâti, terrain, cases au contact, champs.
                if !tile.barricade_en_attente.is_empty() {
                    let liste = std::mem::take(&mut tile.barricade_en_attente);
                    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                    for &(x, y) in &liste {
                        tile.ville.poser_barricade(x as usize, y as usize);
                        let (x, y) = (x as i32, y as i32);
                        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                        let inv = 1.0 / vois_taille;
                        let (cx, cy) = ((x as f32 * inv) as i32, (y as f32 * inv) as i32);
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let (a, b) = (cx + dx, cy + dy);
                                if a >= 0 && b >= 0 && (a as usize) < vois_cote && (b as usize) < vois_cote {
                                    poser_bit(&mut tile.contact_fixe, b as usize * vois_cote + a as usize);
                                }
                            }
                        }
                    }
                    if p.mode_champ == ModeChamp::Flow {
                        let t0 = Instant::now();
                        // Un calcul étalé en cours repartira de zéro.
                        tile.dial.abandonner();
                        let v = &tile.ville;
                        let maj = tile.terrain.mettre_a_jour(x0, y0, x1, y1, |x, y| champ::fin_de(v, x, y));
                        for (c, src) in [
                            (&mut tile.proies, &tile.sources_proies),
                            (&mut tile.champ_bruits, &tile.sources_bruits),
                        ] {
                            if c.vide {
                                continue;
                            }
                            match p.recalcul {
                                Recalcul::Partiel => {
                                    let w = tile.dial.recalcul_hausse(&tile.terrain, c, &maj.hausses, src);
                                    tile.stat.invalidees += w.invalidees;
                                    if !maj.baisses.is_empty() {
                                        tile.dial.recalcul_baisse(&tile.terrain, c, &maj.baisses);
                                    }
                                }
                                Recalcul::Complet => {
                                    tile.dial.calculer(&tile.terrain, c, src);
                                }
                            }
                        }
                        tile.stat.obstacles += 1;
                        tile.stat.obstacle_ns += t0.elapsed().as_nanos() as u64;
                    }
                }
                if p.mode_champ != ModeChamp::Flow {
                    return;
                }
                // Bruits : recalcul quand la liste change.
                if tile.champ_bruits.version != tile.version_bruits {
                    tile.sources_bruits.clear();
                    for &b in &tile.bruits {
                        tile.sources_bruits.push(Graine {
                            i: tile.terrain.indice(b) as u32,
                            d: 0,
                            ts: maintenant as u16,
                        });
                    }
                    let t0 = Instant::now();
                    tile.dial.calculer(&tile.terrain, &mut tile.champ_bruits, &tile.sources_bruits);
                    tile.champ_bruits.version = tile.version_bruits;
                    let ns = t0.elapsed().as_nanos() as u64;
                    tile.stat.ns += ns;
                    tile.stat.recalculs += 1;
                }
                // Proies : recalcul périodique, décalé d'une tile à l'autre ;
                // étalé sur la période si demandé (double tampon : les
                // zombies lisent l'ancien champ pendant le calcul).
                let per = p.periode_champ.max(1) as u64;
                let due = (tick + k as u64) % per == 0;
                let etale = p.etalement && per > 1;
                if !tile.dial.en_cours() {
                    if !due {
                        return;
                    }
                    let t0 = Instant::now();
                    let src = &mut tile.sources_suivantes;
                    src.clear();
                    let hu = &tile.humains;
                    for i in 0..hu.pos.len() {
                        let ot = hu.obs_tick[i].load(Relaxed);
                        if ot != 0 && maintenant - ot < p.duree_derniere_position {
                            let q = decoder(hu.obs_pos[i].load(Relaxed));
                            src.push(Graine {
                                i: tile.terrain.indice(q) as u32,
                                d: 0,
                                ts: ot as u16,
                            });
                        }
                    }
                    // Une graine par case, la plus fraîche : ordre déterministe.
                    src.sort_unstable_by_key(|g| (g.i, std::cmp::Reverse(g.ts)));
                    src.dedup_by_key(|g| g.i);
                    tile.dial.demarrer(&mut tile.proies_suivant, &tile.sources_suivantes);
                    tile.ns_cours = t0.elapsed().as_nanos() as u64;
                }
                let t0 = Instant::now();
                let budget = if etale { tile.budget } else { u64::MAX };
                let (fini, _) = tile.dial.avancer(&tile.terrain, &mut tile.proies_suivant, budget);
                tile.ns_cours += t0.elapsed().as_nanos() as u64;
                if fini {
                    std::mem::swap(&mut tile.proies, &mut tile.proies_suivant);
                    std::mem::swap(&mut tile.sources_proies, &mut tile.sources_suivantes);
                    let f = tile.dial.fixees_cours();
                    tile.budget = (f * 5 / 4 / per).max(1024);
                    tile.stat.ns += tile.ns_cours;
                    tile.stat.max_ns = tile.stat.max_ns.max(tile.ns_cours);
                    tile.stat.fixees += f;
                    tile.stat.recalculs += 1;
                }
            });
        }
    });
}

#[inline(always)]
fn bruit_proche(tile: &Tile, pos: [f32; 2], p: &Params) -> Option<[f32; 2]> {
    let rb2 = p.rayon_bruit * p.rayon_bruit;
    let arrivee2 = ARRIVEE_BRUIT * ARRIVEE_BRUIT;
    let mut bruit: Option<(f32, [f32; 2])> = None;
    for &b in &tile.bruits {
        let dd = d2(b, pos);
        if dd <= rb2 && dd > arrivee2 && bruit.is_none_or(|(x, _)| dd < x) {
            bruit = Some((dd, b));
        }
    }
    bruit.map(|(_, b)| b)
}

/// Phase 3 : perception.
fn percevoir(
    mut q: Query<(&Corps, &Ident, &mut Cerveau)>,
    t: Res<Tiles>,
    g: Res<GrillesZombies>,
    al: Res<Alertes>,
    r: Res<Reglages>,
    h: Res<Horloge>,
) {
    let periode = r.p.periode_perception.max(1) as u64;
    let n = t.zombies_par_tile;
    q.par_iter_mut()
        .batching_strategy(r.lots.clone())
        .for_each(|(corps, id, mut cerveau)| {
            let tile = &t.tiles[id.tile as usize];
            let pos = local(corps.pos, tile.origine);
            let c = cerveau.bypass_change_detection();
            let lent = c.etat == PROFOND || (r.p.veille && c.etat == ERRANCE);
            let per = if lent {
                r.p.periode_veille.max(1) as u64
            } else {
                periode
            };
            let due = (h.0 + id.rang as u64) % per == 0;
            let maintenant = h.0 as u32 + 1;
            let k = id.tile as usize * n + id.rang as usize;
            if c.etat == PROFOND {
                if !due
                    || (tile.humains.proximite.distance_min(pos) > r.p.seuil_profond
                        && bruit_proche(tile, pos, &r.p).is_none())
                {
                    return;
                }
                c.etat = ERRANCE;
            }
            let mut compte = [0u64; 5];
            decider(
                pos,
                corps.cap,
                id.rang,
                tile,
                &g.0[id.tile as usize].vois,
                &r,
                c,
                due,
                maintenant,
                &mut compte,
            );
            if compte[compteurs::REQUETES] > 0 {
                compteurs::ajouter(compte);
            }
            let relayable = c.etat == POURSUITE || c.etat == RELAIS;
            al.horodatage[k].store(if relayable { c.alerte } else { 0 }, Relaxed);
            if relayable {
                al.cible[k].store(coder(c.cible), Relaxed);
            }
            al.indexe[k].store((c.etat != PROFOND) as u8, Relaxed);
        });
}

#[inline(always)]
fn meilleur_relais(
    pos: [f32; 2],
    rang: u32,
    g: &Voisinage,
    p: &Params,
    maintenant: u32,
) -> Option<(u32, [f32; 2])> {
    let r2 = p.rayon_relais * p.rayon_relais;
    let (cx, cy) = g.coord(pos);
    let m = g.cote as i32 - 1;
    let (y0, y1, x0, x1) = ((cy - 1).max(0), (cy + 1).min(m), (cx - 1).max(0), (cx + 1).min(m));
    if p.filtre_relais && !(y0..=y1).any(|y| (x0..=x1).any(|x| g.alerte_dans(x, y))) {
        return None;
    }
    let mut meilleur: Option<(u32, [f32; 2])> = None;
    let mut examines = 0u32;
    for y in y0..=y1 {
        let suite = g.pour_plage(y, x0, x1, |e| {
            if e.id == rang {
                return true;
            }
            examines += 1;
            if e.alerte != 0
                && maintenant - e.alerte <= p.fraicheur_alerte
                && d2(e.p, pos) <= r2
                && meilleur.is_none_or(|(a, _)| e.alerte > a)
            {
                meilleur = Some((e.alerte, e.cible));
            }
            examines < VOISINS_MAX
        });
        if !suite {
            return meilleur;
        }
    }
    meilleur
}

#[inline(always)]
fn perdre(c: &mut Cerveau, maintenant: u32) {
    c.etat = PERDU;
    c.alerte = 0;
    c.perdu_depuis = maintenant;
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn decider(
    pos: [f32; 2],
    cap: f32,
    rang: u32,
    tile: &Tile,
    g: &Voisinage,
    r: &Reglages,
    c: &mut Cerveau,
    due: bool,
    maintenant: u32,
    compte: &mut [u64; 5],
) {
    let p = &r.p;
    let relais = if p.relais {
        meilleur_relais(pos, rang, g, p, maintenant)
    } else {
        None
    };
    let suivre = |c: &mut Cerveau, (a, cible): (u32, [f32; 2])| {
        c.etat = RELAIS;
        c.alerte = a;
        c.cible = cible;
    };
    if !due {
        if c.etat != POURSUITE {
            if let Some(rel) = relais {
                suivre(c, rel);
            } else if c.etat == RELAIS {
                perdre(c, maintenant);
            }
        }
        return;
    }
    if let Some(rel) = relais.filter(|(a, _)| maintenant - a <= p.seuil_source) {
        suivre(c, rel);
        return;
    }
    if let Some((q, id)) = percevoir_humain(pos, cap, tile, r, compte) {
        c.etat = POURSUITE;
        c.cible = q;
        c.alerte = maintenant;
        // Source du champ « proies » : même valeur écrite par tous les
        // observateurs d'un même tick, donc résultat déterministe.
        let hu = &tile.humains;
        hu.obs_tick[id as usize].fetch_max(maintenant, Relaxed);
        hu.obs_pos[id as usize].store(coder(q), Relaxed);
        return;
    }
    if let Some(rel) = relais {
        suivre(c, rel);
        return;
    }
    if c.etat == POURSUITE || c.etat == RELAIS {
        perdre(c, maintenant);
    }
    if let Some(b) = bruit_proche(tile, pos, p) {
        c.etat = ATTIRE;
        c.cible = b;
        return;
    }
    if c.etat == PERDU
        && maintenant - c.perdu_depuis < p.duree_derniere_position
        && d2(c.cible, pos) > 4.0
    {
        return;
    }
    c.etat = if p.profonde && tile.humains.proximite.distance_min(pos) > p.seuil_profond {
        PROFOND
    } else {
        ERRANCE
    };
}

#[inline(always)]
fn percevoir_humain(
    pos: [f32; 2],
    cap: f32,
    tile: &Tile,
    r: &Reglages,
    compte: &mut [u64; 5],
) -> Option<([f32; 2], u32)> {
    let p = &r.p;
    compte[compteurs::REQUETES] += 1;
    let mut q = requete(pos, cap, r);
    if p.rejet {
        let dmin = tile.humains.proximite.distance_min(pos);
        if dmin > p.distance_vue.max(p.rayon_ouie) {
            compte[compteurs::REJETS] += 1;
            return None;
        }
        q.ouie_possible = dmin <= p.rayon_ouie;
    }
    let mut c = Compte::default();
    let res = tile.humains.index.chercher(&q, &mut c);
    compte[compteurs::CASES] += c.cases;
    compte[compteurs::CANDIDATS] += c.candidats;
    if let Some(e) = res.entendu {
        return Some((e.p, e.id));
    }
    for v in &res.vus[..res.nb] {
        compte[compteurs::LIGNES] += 1;
        if tile.ville.vue_degagee(pos, v.p) {
            return Some((v.p, v.id));
        }
    }
    None
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

/// Résultat du déplacement d'un zombie.
enum Pas {
    /// Nouvelle position (éventuellement inchangée).
    Aller([f32; 2]),
    /// Entrée dans une case au contact : à réserver.
    Demander([f32; 2], usize),
}

/// Phase 4 : déplacement.
#[allow(clippy::too_many_arguments)]
fn deplacer(
    mut q: Query<(&mut Corps, &Ident, &Cerveau)>,
    t: Res<Tiles>,
    g: Res<GrillesZombies>,
    inst: Res<Instantane>,
    res: Res<Reservations>,
    r: Res<Reglages>,
    h: Res<Horloge>,
) {
    let n = t.zombies_par_tile;
    q.par_iter_mut()
        .batching_strategy(r.lots.clone())
        .for_each(|(mut corps, id, cerveau)| {
            let corps = corps.bypass_change_detection();
            let tile = &t.tiles[id.tile as usize];
            let pos = local(corps.pos, tile.origine);
            let k = id.tile as usize * n + id.rang as usize;
            let pas = if tile.ville.bati_p(pos) {
                // Pris dans un obstacle neuf : vers la case libre la plus
                // proche.
                Pas::Aller(expulser(&tile.ville, pos))
            } else if cerveau.etat == PROFOND {
                Pas::Aller(deplacer_profond(pos, corps, id, tile, &r.p, h.0))
            } else {
                deplacer_un(pos, corps, id, cerveau, tile, &g.0[id.tile as usize], &r, h.0)
            };
            match pas {
                Pas::Aller(np) => {
                    if np != pos {
                        corps.pos = [tile.origine[0] + np[0] as f64, tile.origine[1] + np[1] as f64];
                        inst.0[k].store(coder(np), Relaxed);
                    }
                }
                Pas::Demander(np, case) => {
                    let ti = id.tile as usize;
                    let slot = res.nb[ti].fetch_add(1, Relaxed) as usize;
                    let prio = hacher((id.tile as u64) << 32 | id.rang as u64, h.0) & 0x7FFF_FFFF;
                    res.demandes[ti][slot].store((case as u64) << 46 | prio << 15 | id.rang as u64, Relaxed);
                    res.souhait[k].store(coder(np), Relaxed);
                    res.attente[k].store(1, Relaxed);
                }
            }
        });
}

#[inline(always)]
fn deplacer_profond(
    pos: [f32; 2],
    corps: &mut Corps,
    id: &Ident,
    tile: &Tile,
    p: &Params,
    tick: u64,
) -> [f32; 2] {
    let n = p.periode_veille.max(1) as u64;
    if (tick + id.rang as u64) % n != 0 {
        return pos;
    }
    let graine = (id.tile as u64) << 32 | id.rang as u64;
    let mut cap = corps.cap + (unite(hacher(graine, tick)) - 0.5);
    let pas = p.vitesse_errance * DT * n as f32;
    let sous = (pas / 0.25).ceil().max(1.0);
    let mut np = pos;
    for _ in 0..sous as u32 {
        let q = [np[0] + cap.cos() * pas / sous, np[1] + cap.sin() * pas / sous];
        if tile.ville.bati_p(q) {
            cap += std::f32::consts::FRAC_PI_2 + unite(hacher(graine, !tick)) * 2.0;
            break;
        }
        np = q;
    }
    corps.cap = cap;
    np
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn deplacer_un(
    pos: [f32; 2],
    corps: &mut Corps,
    id: &Ident,
    c: &Cerveau,
    tile: &Tile,
    occ: &Occupation,
    r: &Reglages,
    tick: u64,
) -> Pas {
    let p = &r.p;
    let g = &occ.vois;
    let graine = (id.tile as u64) << 32 | id.rang as u64;
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
        let champ = if c.etat == ATTIRE {
            &tile.champ_bruits
        } else {
            &tile.proies
        };
        if d2c < 0.25 {
            (0.0, corps.cap)
        } else if let Some(dir) = (p.mode_champ == ModeChamp::Flow && d2c > APPROCHE * APPROCHE)
            .then(|| champ.direction(&tile.terrain, pos))
            .flatten()
        {
            (p.vitesse_course, dir[1].atan2(dir[0]))
        } else {
            (p.vitesse_course, d[1].atan2(d[0]))
        }
    };
    let pas = vitesse * DT * facteur;
    let mut np = [pos[0] + cap.cos() * pas, pos[1] + cap.sin() * pas];

    let sep = p.distance_separation;
    let (cx, cy) = g.coord(pos);
    let m = g.cote as i32 - 1;
    let mut corr = [0f32; 2];
    let mut examines = 0u32;
    for y in (cy - 1).max(0)..=(cy + 1).min(m) {
        let suite = g.pour_plage(y, (cx - 1).max(0), (cx + 1).min(m), |e| {
            if e.id == id.rang {
                return true;
            }
            let d = [pos[0] - e.p[0], pos[1] - e.p[1]];
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
        if !suite {
            break;
        }
    }
    let lc = (corr[0] * corr[0] + corr[1] * corr[1]).sqrt();
    if lc > 0.3 {
        corr = [corr[0] * 0.3 / lc, corr[1] * 0.3 / lc];
    }
    np = [np[0] + corr[0], np[1] + corr[1]];

    let v = &tile.ville;
    let ici = g.coord(pos);
    let capa = r.capacite;
    let exacte = |c: (i32, i32)| match p.reservation {
        Reservation::Aucune => false,
        Reservation::Contact => bit(&occ.contact, c.1 as usize * g.cote + c.0 as usize),
        Reservation::Partout => true,
    };
    // Case libre ? Réservation exacte : capacité exacte (la réservation
    // décidera) ; ailleurs : admission progressive (P-01).
    let libre = |q: [f32; 2]| {
        if v.bati_p(q) {
            return false;
        }
        let c = g.coord(q);
        if c == ici {
            return true;
        }
        let occupants = g.occupants(c.0, c.1);
        if exacte(c) {
            return occupants < capa;
        }
        occupants < capa && (hacher(graine, tick) % capa as u64) as u32 >= occupants
    };
    if !libre(np) {
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
    let cn = g.coord(np);
    if cn != ici && exacte(cn) {
        return Pas::Demander(np, cn.1 as usize * g.cote + cn.0 as usize);
    }
    Pas::Aller(np)
}

/// Phase 5a : décision des réservations, par tile. Demandes triées par
/// (case, priorité, rang) : l'ordre d'écriture des threads ne compte pas.
/// Places libres = capacité − occupants au début du tick (les sortants ne
/// libèrent leur place qu'au tick suivant : la borne est exacte).
fn admettre(res: Res<Reservations>, g: Res<GrillesZombies>, r: Res<Reglages>, t: Res<Tiles>) {
    if r.p.reservation == Reservation::Aucune {
        return;
    }
    let n = t.zombies_par_tile;
    let capa = r.capacite;
    let res = &*res;
    let total_d = AtomicU64::new(0);
    let total_a = AtomicU64::new(0);
    ComputeTaskPool::get().scope(|s| {
        for (k, occ) in g.0.iter().enumerate() {
            let (td, ta) = (&total_d, &total_a);
            s.spawn(async move {
                let nb = res.nb[k].swap(0, Relaxed) as usize;
                if nb == 0 {
                    return;
                }
                let mut d: Vec<u64> = res.demandes[k][..nb].iter().map(|a| a.load(Relaxed)).collect();
                d.sort_unstable();
                let cote = occ.vois.cote;
                let mut admis = 0u64;
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
                        let rang = (x & 0x7FFF) as usize;
                        res.attente[k * n + rang].store(2, Relaxed);
                        admis += 1;
                    }
                    i = j;
                }
                td.fetch_add(nb as u64, Relaxed);
                ta.fetch_add(admis, Relaxed);
            });
        }
    });
    res.total_demandes.store(total_d.into_inner(), Relaxed);
    res.total_admis.store(total_a.into_inner(), Relaxed);
}

/// Phase 5b : application des réservations admises.
fn appliquer(
    mut q: Query<(&mut Corps, &Ident)>,
    res: Res<Reservations>,
    inst: Res<Instantane>,
    t: Res<Tiles>,
    r: Res<Reglages>,
) {
    if r.p.reservation == Reservation::Aucune || res.total_demandes.load(Relaxed) == 0 {
        return;
    }
    let n = t.zombies_par_tile;
    q.par_iter_mut()
        .batching_strategy(r.lots.clone())
        .for_each(|(mut corps, id)| {
            let k = id.tile as usize * n + id.rang as usize;
            let a = res.attente[k].load(Relaxed);
            if a == 0 {
                return;
            }
            res.attente[k].store(0, Relaxed);
            if a == 2 {
                let tile = &t.tiles[id.tile as usize];
                let v = res.souhait[k].load(Relaxed);
                let np = decoder(v);
                corps.bypass_change_detection().pos =
                    [tile.origine[0] + np[0] as f64, tile.origine[1] + np[1] as f64];
                inst.0[k].store(v, Relaxed);
            }
        });
}

// ---------------------------------------------------------------- Simulation

#[derive(Default, Debug)]
pub struct Bilan {
    pub errance: usize,
    pub poursuite: usize,
    pub attire: usize,
    pub relais: usize,
    pub perdu: usize,
    pub profond: usize,
    /// Plus grand nombre de zombies sur une case de 2 m × 2 m.
    pub densite_max: u32,
    /// Idem, sur les cases au contact (réservation).
    pub densite_contact: u32,
    pub murs: usize,
}

pub const NB_PHASES: usize = 6;
pub const NOMS_PHASES: [&str; NB_PHASES] = [
    "humains",
    "grilles",
    "champs",
    "perception",
    "deplacement",
    "admission",
];

/// Un zombie, pour les mesures de crédibilité.
#[derive(Clone, Copy)]
pub struct Vue {
    pub tile: u32,
    pub rang: u32,
    pub pos: [f32; 2],
    pub etat: u8,
    pub cible: [f32; 2],
}

pub struct Simulation {
    world: World,
    phases: [Schedule; NB_PHASES],
    pub couverture: f32,
}

impl Simulation {
    pub fn new(c: &ConfigSim) -> Self {
        ComputeTaskPool::get_or_init(|| TaskPoolBuilder::new().num_threads(c.threads).build());
        let p = &c.params;
        assert!(c.zombies < 1 << 15, "rang codé sur 15 bits");
        let centre = [COTE_F / 2.0, COTE_F / 2.0];
        let mut world = World::new();

        let liste_reelle: Vec<(i64, i64)> = match &c.source {
            SourceVille::Synthetique => vec![],
            SourceVille::Reelle(d) => {
                let csv = std::fs::read_to_string(d.join("tiles.csv")).expect("tiles.csv");
                csv.lines()
                    .skip(1)
                    .map(|l| {
                        let mut f = l.split(',');
                        (f.next().unwrap().parse().unwrap(), f.next().unwrap().parse().unwrap())
                    })
                    .collect()
            }
        };
        let cote_vois = (COTE_F / p.case_zombies).ceil() as usize;
        let mut tiles = Vec::with_capacity(c.tiles);
        let mut inst = Vec::with_capacity(c.tiles * c.zombies);
        let mut zombies = Vec::with_capacity(c.tiles * c.zombies);
        for k in 0..c.tiles {
            let mut a = Alea::new(hacher(0x5EED, k as u64));
            let graine_ville = a.u64();
            let (ville, origine) = match &c.source {
                SourceVille::Synthetique => (
                    Ville::generer(graine_ville),
                    [
                        ORIGINE[0] + (k % 16) as f64 * 1000.0,
                        ORIGINE[1] + (k / 16) as f64 * 1000.0,
                    ],
                ),
                SourceVille::Reelle(d) => {
                    let (x, y) = *liste_reelle
                        .get(k)
                        .unwrap_or_else(|| panic!("seulement {} tiles réelles", liste_reelle.len()));
                    let v = Ville::charger(&d.join(format!("x{x}_y{y}.bin"))).expect("tile réelle");
                    (v, [x as f64 * 1000.0, y as f64 * 1000.0])
                }
            };
            let centre = centre_accessible(&ville, centre);
            // Zone accessible : champ exact à 1 m depuis le centre.
            let (t1, c1) = champ_1m(&ville, &[centre]);
            let zone = zone_de(&t1, &c1);
            let acc = |q: [f32; 2]| bit(&zone, q[1] as usize * COTE + q[0] as usize);
            let tirer = |a: &mut Alea, rayon: f32| loop {
                let q = ville.point_libre(a, centre, rayon);
                if acc(q) {
                    return q;
                }
            };
            let hpos: Vec<[f32; 2]> = (0..c.humains)
                .map(|_| match c.scenario {
                    Scenario::Disperse => tirer(&mut a, 710.0),
                    _ => tirer(&mut a, RAYON_GROUPE),
                })
                .collect();
            let hcap: Vec<f32> = (0..c.humains).map(|_| a.f32() * std::f32::consts::TAU).collect();
            // Repli : destination à `distance_repli` de marche du centre,
            // la plus éloignée à vol d'oiseau.
            let repli = (c.scenario == Scenario::Repli).then(|| {
                let cible = (p.distance_repli * champ::UNITE) as i32;
                let mut best = (f32::MIN, centre);
                for y in 60..COTE as i32 - 60 {
                    for x in 60..COTE as i32 - 60 {
                        let d = c1.dist[t1.ixy(x, y)];
                        if d != champ::INF && (d as i32 - cible).abs() < 100 {
                            let q = [x as f32 + 0.5, y as f32 + 0.5];
                            let e = d2(q, centre);
                            if e > best.0 {
                                best = (e, q);
                            }
                        }
                    }
                }
                let (terrain, champ) = champ_1m(&ville, &[best.1]);
                Repli {
                    terrain,
                    champ,
                    dest: best.1,
                }
            });
            let mut index = Index::new(p.structure, p.case);
            index.reconstruire(&hpos);
            let mut proximite = Proximite::new(CASE_PROXIMITE);
            proximite.reconstruire(&hpos);
            let bruits = match c.scenario {
                Scenario::Assaut => vec![centre],
                _ => vec![],
            };
            for rang in 0..c.zombies {
                let q = tirer(&mut a, 710.0);
                inst.push(AtomicU64::new(coder(q)));
                zombies.push((
                    Corps {
                        pos: [origine[0] + q[0] as f64, origine[1] + q[1] as f64],
                        cap: a.f32() * std::f32::consts::TAU,
                    },
                    Cerveau {
                        etat: ERRANCE,
                        cible: [0.0; 2],
                        alerte: 0,
                        perdu_depuis: 0,
                    },
                    Ident {
                        tile: k as u32,
                        rang: rang as u32,
                    },
                ));
            }
            let terrain = Terrain::de_ville(&ville, p.resolution, p.regle, p.marge);
            tiles.push(Tile {
                origine,
                centre,
                zone,
                humains: Humains {
                    obs_tick: (0..c.humains).map(|_| AtomicU32::new(0)).collect(),
                    obs_pos: (0..c.humains).map(|_| AtomicU64::new(0)).collect(),
                    pos: hpos,
                    cap: hcap,
                    index,
                    proximite,
                },
                bruits,
                version_bruits: 0,
                proies: Champ::new(&terrain),
                proies_suivant: Champ::new(&terrain),
                sources_suivantes: vec![],
                budget: u64::MAX,
                ns_cours: 0,
                champ_bruits: Champ::new(&terrain),
                dial: Dial::new(&terrain),
                terrain,
                sources_proies: vec![],
                sources_bruits: vec![],
                repli,
                barricade_en_attente: vec![],
                contact_fixe: vec![0; (cote_vois * cote_vois).div_ceil(64)],
                stat: StatChamps::default(),
                ville,
            });
        }
        world.spawn_batch(zombies);
        let couverture = tiles.iter().map(|t| t.ville.couverture).sum::<f32>() / c.tiles as f32;
        world.insert_resource(Tiles {
            tiles,
            zombies_par_tile: c.zombies,
            scenario: c.scenario,
        });
        world.insert_resource(GrillesZombies(
            (0..c.tiles)
                .map(|_| Occupation {
                    vois: Voisinage::new(p.voisinage, COTE_F, p.case_zombies, c.zombies),
                    contact: vec![0; (cote_vois * cote_vois).div_ceil(64)],
                })
                .collect(),
        ));
        let total = inst.len();
        world.insert_resource(Instantane(inst));
        world.insert_resource(Alertes {
            horodatage: (0..total).map(|_| AtomicU32::new(0)).collect(),
            cible: (0..total).map(|_| AtomicU64::new(0)).collect(),
            indexe: (0..total).map(|_| AtomicU8::new(1)).collect(),
        });
        world.insert_resource(Reservations {
            demandes: (0..c.tiles)
                .map(|_| (0..c.zombies).map(|_| AtomicU64::new(0)).collect())
                .collect(),
            nb: (0..c.tiles).map(|_| AtomicU32::new(0)).collect(),
            souhait: (0..total).map(|_| AtomicU64::new(0)).collect(),
            attente: (0..total).map(|_| AtomicU8::new(0)).collect(),
            total_demandes: AtomicU64::new(0),
            total_admis: AtomicU64::new(0),
        });
        world.insert_resource(Horloge(0));
        let demi = p.demi_angle_vue_deg.to_radians();
        let echelle = (p.case_zombies / 2.0).powi(2);
        world.insert_resource(Reglages {
            cos_demi_angle: demi.cos(),
            sin_demi_angle: demi.sin(),
            p: p.clone(),
            lots: BatchingStrategy::new().min_batch_size(c.lot_min.max(1)),
            capacite: (p.capacite_case as f32 * echelle).round() as u32,
        });

        let mut phases: [Schedule; NB_PHASES] = Default::default();
        phases[0].add_systems(humains);
        phases[1].add_systems(construire_grilles);
        phases[2].add_systems(champs);
        phases[3].add_systems(percevoir);
        phases[4].add_systems(deplacer);
        phases[5].add_systems((admettre, appliquer).chain());
        for s in &mut phases {
            s.set_executor(MultiThreadedExecutor::new());
            s.initialize(&mut world).expect("initialisation du schedule");
        }
        Simulation {
            world,
            phases,
            couverture,
        }
    }

    pub fn tick(&mut self) -> [Duration; NB_PHASES] {
        let mut d = [Duration::ZERO; NB_PHASES];
        for (i, s) in self.phases.iter_mut().enumerate() {
            let t = Instant::now();
            s.run(&mut self.world);
            d[i] = t.elapsed();
        }
        self.world.resource_mut::<Horloge>().0 += 1;
        d
    }

    pub fn horloge(&self) -> u64 {
        self.world.resource::<Horloge>().0
    }

    /// Change l'index des zombies en cours de partie (sans effet sur le
    /// résultat pour une même taille de case).
    pub fn changer_voisinage(&mut self, s: voisinage::Structure) {
        let n = self.world.resource::<Tiles>().zombies_par_tile;
        let taille = self.world.resource::<Reglages>().p.case_zombies;
        self.world.resource_mut::<Reglages>().p.voisinage = s;
        for o in &mut self.world.resource_mut::<GrillesZombies>().0 {
            o.vois = Voisinage::new(s, COTE_F, taille, n);
        }
    }

    /// Pose l'anneau de barricades sur chaque tile (appliqué au tick suivant,
    /// pendant la phase des champs).
    pub fn poser_barricades(&mut self) -> usize {
        let mut total = 0;
        for t in &mut self.world.resource_mut::<Tiles>().tiles {
            let b = anneau_barricades(&t.ville, &t.zone, t.centre);
            total += b.len();
            t.barricade_en_attente = b;
        }
        total
    }

    /// Remet à zéro le maximum des temps de calcul de champ (début d'une
    /// mesure).
    pub fn remettre_max(&mut self) {
        for t in &mut self.world.resource_mut::<Tiles>().tiles {
            t.stat.max_ns = 0;
        }
    }

    pub fn stats_champs(&self) -> StatChamps {
        let mut s = StatChamps::default();
        for t in &self.world.resource::<Tiles>().tiles {
            s.recalculs += t.stat.recalculs;
            s.ns += t.stat.ns;
            s.fixees += t.stat.fixees;
            s.obstacles += t.stat.obstacles;
            s.obstacle_ns += t.stat.obstacle_ns;
            s.invalidees += t.stat.invalidees;
            s.max_ns = s.max_ns.max(t.stat.max_ns);
        }
        s
    }

    pub fn reservations(&self) -> (u64, u64) {
        let r = self.world.resource::<Reservations>();
        (r.total_demandes.load(Relaxed), r.total_admis.load(Relaxed))
    }

    pub fn somme_controle(&mut self) -> u64 {
        let mut h = 0u64;
        let mut q = self.world.query::<(&Corps, &Cerveau, &Ident)>();
        for (c, cv, id) in q.iter(&self.world) {
            let x = hacher(c.pos[0].to_bits(), c.pos[1].to_bits() ^ c.cap.to_bits() as u64);
            let y = hacher(cv.etat as u64, (id.tile as u64) << 32 | id.rang as u64);
            h = h.wrapping_add(hacher(x, y));
        }
        for t in &self.world.resource::<Tiles>().tiles {
            for p in &t.humains.pos {
                h = h.wrapping_add(hacher(p[0].to_bits() as u64, p[1].to_bits() as u64));
            }
        }
        h
    }

    pub fn tiles(&self) -> &Tiles {
        self.world.resource::<Tiles>()
    }

    pub fn occupation(&self, k: usize) -> &Occupation {
        &self.world.resource::<GrillesZombies>().0[k]
    }

    /// Sources du champ « proies » à cet instant : positions observées des
    /// humains vus depuis moins d'une minute.
    pub fn positions_sources(&self, k: usize) -> Vec<[f32; 2]> {
        let t = &self.world.resource::<Tiles>().tiles[k];
        let r = self.world.resource::<Reglages>();
        let maintenant = self.horloge() as u32 + 1;
        let hu = &t.humains;
        (0..hu.pos.len())
            .filter_map(|i| {
                let ot = hu.obs_tick[i].load(Relaxed);
                (ot != 0 && maintenant - ot < r.p.duree_derniere_position)
                    .then(|| decoder(hu.obs_pos[i].load(Relaxed)))
            })
            .collect()
    }

    /// Champ « proies » recalculé maintenant sur la tile `k`, avec les
    /// sources du moment (mesure de l'écart dû à la période de recalcul).
    pub fn champ_frais(&self, k: usize) -> Champ {
        let t = &self.world.resource::<Tiles>().tiles[k];
        let r = self.world.resource::<Reglages>();
        let maintenant = self.horloge() as u32 + 1;
        let hu = &t.humains;
        let mut src = vec![];
        for i in 0..hu.pos.len() {
            let ot = hu.obs_tick[i].load(Relaxed);
            if ot != 0 && maintenant - ot < r.p.duree_derniere_position {
                let q = decoder(hu.obs_pos[i].load(Relaxed));
                src.push(Graine {
                    i: t.terrain.indice(q) as u32,
                    d: 0,
                    ts: ot as u16,
                });
            }
        }
        src.sort_unstable_by_key(|g| (g.i, std::cmp::Reverse(g.ts)));
        src.dedup_by_key(|g| g.i);
        let mut c = Champ::new(&t.terrain);
        Dial::new(&t.terrain).calculer(&t.terrain, &mut c, &src);
        c
    }

    pub fn zone(&self, k: usize) -> &[u64] {
        &self.world.resource::<Tiles>().tiles[k].zone
    }

    /// Positions et états de tous les zombies, par (tile, rang).
    pub fn vues(&mut self) -> Vec<Vue> {
        let mut q = self.world.query::<(&Corps, &Cerveau, &Ident)>();
        let t = self.world.resource::<Tiles>();
        let mut v: Vec<Vue> = q
            .iter(&self.world)
            .map(|(c, cv, id)| Vue {
                tile: id.tile,
                rang: id.rang,
                pos: local(c.pos, t.tiles[id.tile as usize].origine),
                etat: cv.etat,
                cible: cv.cible,
            })
            .collect();
        v.sort_unstable_by_key(|z| (z.tile, z.rang));
        v
    }

    /// Instantané de l'index pour le banc : (n par tile, indexé, position,
    /// alerte) de chaque zombie.
    #[allow(clippy::type_complexity)]
    pub fn donnees_index(&self) -> (usize, Vec<bool>, Vec<[f32; 2]>, Vec<(u32, [f32; 2])>) {
        let n = self.world.resource::<Tiles>().zombies_par_tile;
        let inst = &self.world.resource::<Instantane>().0;
        let al = self.world.resource::<Alertes>();
        (
            n,
            al.indexe.iter().map(|a| a.load(Relaxed) != 0).collect(),
            inst.iter().map(|a| decoder(a.load(Relaxed))).collect(),
            al.horodatage
                .iter()
                .zip(&al.cible)
                .map(|(h, c)| (h.load(Relaxed), decoder(c.load(Relaxed))))
                .collect(),
        )
    }

    #[cfg(test)]
    pub fn retirer_humains(&mut self) {
        for t in &mut self.world.resource_mut::<Tiles>().tiles {
            t.humains.pos.clear();
            t.humains.cap.clear();
            t.humains.index.reconstruire(&[]);
            t.humains.proximite.reconstruire(&[]);
        }
    }

    pub fn bilan(&mut self) -> Bilan {
        let mut b = Bilan::default();
        let vues = self.vues();
        for z in &vues {
            match z.etat {
                ERRANCE => b.errance += 1,
                POURSUITE => b.poursuite += 1,
                ATTIRE => b.attire += 1,
                RELAIS => b.relais += 1,
                PROFOND => b.profond += 1,
                _ => b.perdu += 1,
            }
        }
        let t = self.world.resource::<Tiles>();
        let g = self.world.resource::<GrillesZombies>();
        b.murs = vues
            .iter()
            .filter(|z| t.tiles[z.tile as usize].ville.bati_p(z.pos))
            .count();
        // Densités sur des cases de 2 m, quel que soit l'index (zombies
        // indexés seulement, comme la capacité).
        let mut cles: Vec<(u32, u32)> = vues
            .iter()
            .filter(|z| z.etat != PROFOND)
            .map(|z| {
                let (x, y) = ((z.pos[0] / 2.0) as u32, (z.pos[1] / 2.0) as u32);
                (z.tile, y.min(499) * 500 + x.min(499))
            })
            .collect();
        cles.sort_unstable();
        let mut i = 0;
        while i < cles.len() {
            let mut j = i;
            while j < cles.len() && cles[j] == cles[i] {
                j += 1;
            }
            let nb = (j - i) as u32;
            b.densite_max = b.densite_max.max(nb);
            let (tile, case) = cles[i];
            let occ = &g.0[tile as usize];
            let centre = [(case % 500) as f32 * 2.0 + 1.0, (case / 500) as f32 * 2.0 + 1.0];
            let (cx, cy) = occ.vois.coord(centre);
            if bit(&occ.contact, cy as usize * occ.vois.cote + cx as usize) {
                b.densite_contact = b.densite_contact.max(nb);
            }
            i = j;
        }
        b
    }
}
