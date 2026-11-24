//! Simulation des zombies d'une ou plusieurs tiles actives, avec `bevy_ecs`
//! (exécuteur multi-thread, requêtes `par_iter_mut`). Repris de P-01 ; P-02
//! y ajoute :
//! - des **humains mobiles**, dont l'index est reconstruit à chaque tick ;
//! - le choix de la **structure d'index** des humains (`index.rs`) ;
//! - le **rejet rapide** par carte de proximité ;
//! - la **veille profonde** (A-01) : hors de l'index des zombies.
//!
//! Un tick = quatre phases, chronométrées séparément :
//! 0. **humains** : déplacement des humains, reconstruction de leur index et
//!    de la carte de proximité (une tâche par tile) ;
//! 1. **grilles** : chaque tile reconstruit sa grille de zombies (cases de
//!    2 m), sans les zombies en veille profonde ;
//! 2. **perception** : relais d'alerte, ouïe, vue, bruits, dernière position ;
//! 3. **déplacement** : vers la cible ou en errance, séparation, murs.
//!
//! Chaque phase ne lit que l'état du tick précédent (ou de la phase
//! précédente) et n'écrit que l'entité qu'elle traite : le résultat ne dépend
//! ni du nombre de threads, ni du découpage en lots, ni de la structure
//! d'index (vérifié par les tests).

use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use bevy_ecs::batching::BatchingStrategy;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::MultiThreadedExecutor;
use bevy_tasks::{ComputeTaskPool, TaskPoolBuilder};
use clap::ValueEnum;

use crate::alea::{Alea, hacher, unite};
use crate::compteurs;
use crate::grille::Grille;
use crate::index::{Compte, Index, Proximite, RAYONS_MAX, Requete, Structure};
use crate::ville::{COTE, COTE_F, Ville};

/// Pas de temps : 20 ticks par seconde de jeu.
pub const DT: f32 = 0.05;
/// Côté d'une case de la grille des zombies (collisions).
const CASE_ZOMBIES: f32 = 2.0;
/// Côté d'une case de la carte de proximité des humains.
const CASE_PROXIMITE: f32 = 25.0;
/// Nombre maximal de voisins examinés pour la séparation.
const VOISINS_MAX: u32 = 16;
/// Rayon du groupe d'humains des scénarios calme et assaut.
const RAYON_GROUPE: f32 = 60.0;
/// Distance sous laquelle un bruit atteint n'attire plus.
const ARRIVEE_BRUIT: f32 = 15.0;
/// Origine de la première tile, en Lambert-93 (Paris).
const ORIGINE: [f64; 2] = [650_000.0, 6_860_000.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Scenario {
    /// 1 000 humains regroupés au centre, aucun bruit.
    Calme,
    /// Idem, plus des tirs continus au centre entendus à 600 m.
    Assaut,
    /// 1 000 humains répartis dans toute la tile (pire cas de perception).
    Disperse,
}

/// Paramètres de comportement. Ce sont des **hypothèses de prototype**, pas
/// des statistiques de jeu (qui restent qualitatives dans la documentation).
#[derive(Clone)]
pub struct Params {
    pub vitesse_course: f32,
    pub vitesse_errance: f32,
    /// Marche des humains (patrouille), s'ils sont mobiles.
    pub vitesse_humains: f32,
    pub humains_mobiles: bool,
    pub rayon_ouie: f32,
    pub distance_vue: f32,
    pub demi_angle_vue_deg: f32,
    pub distance_separation: f32,
    pub rayon_bruit: f32,
    /// Zombies au plus dans une case de 2 m × 2 m avant qu'elle soit pleine.
    pub capacite_case: u32,
    /// Perception une fois tous les N ticks (étalée entre les zombies).
    pub periode_perception: u32,
    /// Lignes de vue tracées au plus par perception (≤ RAYONS_MAX).
    pub rayons: usize,
    /// Structure d'index des humains, et côté de ses cases (m).
    pub structure: Structure,
    pub case: f32,
    /// Rejet rapide : pas de requête si la carte de proximité garantit
    /// qu'aucun humain n'est à portée.
    pub rejet: bool,
    /// Relais de l'alerte entre voisins (ZOMBIES.md).
    pub relais: bool,
    /// Filtre du relais par la carte des cases porteuses d'alerte.
    pub filtre_relais: bool,
    pub rayon_relais: f32,
    /// Âge maximal d'une alerte relayée (ticks).
    pub fraicheur_alerte: u32,
    /// Âge (ticks) sous lequel une alerte voisine dispense de percevoir.
    pub seuil_source: u32,
    /// Mise en veille des zombies sans stimulus (A-03).
    pub veille: bool,
    /// En veille : perception une fois tous les N ticks.
    pub periode_veille: u32,
    /// En veille : déplacement un tick sur N, avec un pas N fois plus long.
    pub pas_veille: u32,
    /// Veille profonde (A-01) : zombie en errance à plus de `seuil_profond`
    /// mètres de tout humain. Hors de l'index des zombies, sans collision,
    /// déplacement et réveil une fois toutes les `periode_veille` ticks.
    pub profonde: bool,
    pub seuil_profond: f32,
    /// Durée de suivi de la dernière position connue (ticks).
    pub duree_derniere_position: u32,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            vitesse_course: 5.0,
            vitesse_errance: 1.0,
            vitesse_humains: 1.5,
            humains_mobiles: true,
            rayon_ouie: 40.0,
            distance_vue: 200.0,
            demi_angle_vue_deg: 60.0,
            distance_separation: 0.7,
            rayon_bruit: 600.0,
            capacite_case: 24,
            periode_perception: 5,
            rayons: 4,
            structure: Structure::Grille,
            case: 25.0,
            rejet: false,
            relais: false,
            filtre_relais: false,
            rayon_relais: 3.0,
            fraicheur_alerte: 60,
            seuil_source: 5,
            veille: false,
            periode_veille: 20,
            pas_veille: 4,
            profonde: false,
            seuil_profond: 300.0,
            duree_derniere_position: 1200,
        }
    }
}

/// Origine des emprises des bâtiments.
#[derive(Clone)]
pub enum SourceVille {
    Synthetique,
    /// Dossier produit par `p01-tile-paris/outils/extraire_tiles.py` ; tiles
    /// prises dans l'ordre de `tiles.csv` (de la plus bâtie à la moins bâtie).
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

/// Données écrites par le déplacement (groupées, cf. F-02).
#[derive(Component)]
struct Corps {
    /// Position en Lambert-93, 64 bits (C-01).
    pos: [f64; 2],
    /// Direction du regard et de la marche, en radians.
    cap: f32,
}

const ERRANCE: u8 = 0;
const POURSUITE: u8 = 1;
const ATTIRE: u8 = 2;
/// Suit une alerte transmise par un voisin, sans percevoir la cible.
const RELAIS: u8 = 3;
/// A perdu la trace : va vers la dernière position connue.
const PERDU: u8 = 4;
/// Veille profonde : loin de tout humain, hors de l'index.
const PROFOND: u8 = 5;

/// Données écrites par la perception. L'état est un champ (F-02).
#[derive(Component)]
struct Cerveau {
    etat: u8,
    /// Cible en coordonnées locales à la tile.
    cible: [f32; 2],
    /// POURSUITE et RELAIS : tick (+1) de la dernière observation directe.
    alerte: u32,
    /// PERDU : tick de la perte de la trace.
    perdu_depuis: u32,
}

/// Identité, jamais modifiée.
#[derive(Component)]
struct Ident {
    tile: u32,
    rang: u32,
}

// ---------------------------------------------------------------- Ressources

struct Humains {
    pos: Vec<[f32; 2]>,
    cap: Vec<f32>,
    index: Index,
    proximite: Proximite,
}

struct Tile {
    origine: [f64; 2],
    ville: Ville,
    /// Centre du groupe d'humains (et des bruits).
    centre: [f32; 2],
    /// Distance à pied de chaque case de 1 m au centre (u16::MAX =
    /// inaccessible), calculée une fois : flow field minimal (P-03).
    champ: Vec<u16>,
    humains: Humains,
    bruits: Vec<[f32; 2]>,
}

#[derive(Resource)]
struct Tiles {
    tiles: Vec<Tile>,
    zombies_par_tile: usize,
    scenario: Scenario,
}

#[derive(Resource)]
struct GrillesZombies(Vec<Grille>);

/// Positions locales de tous les zombies à la fin du tick précédent.
#[derive(Resource)]
struct Instantane(Vec<AtomicU64>);

/// Publié par chaque zombie à la fin de sa perception, lu au tick suivant :
/// alerte (horodatage, cible) et présence dans l'index (0 = veille profonde).
#[derive(Resource)]
struct Alertes {
    horodatage: Vec<AtomicU32>,
    cible: Vec<AtomicU64>,
    indexe: Vec<AtomicU8>,
}

#[derive(Resource)]
struct Horloge(u64);

#[derive(Resource)]
struct Reglages {
    p: Params,
    cos_demi_angle: f32,
    sin_demi_angle: f32,
    lots: BatchingStrategy,
}

#[inline(always)]
fn coder(p: [f32; 2]) -> u64 {
    (p[0].to_bits() as u64) << 32 | p[1].to_bits() as u64
}
#[inline(always)]
fn decoder(v: u64) -> [f32; 2] {
    [f32::from_bits((v >> 32) as u32), f32::from_bits(v as u32)]
}
#[inline(always)]
fn local(p: [f64; 2], o: [f64; 2]) -> [f32; 2] {
    [(p[0] - o[0]) as f32, (p[1] - o[1]) as f32]
}
#[inline(always)]
fn d2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
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

/// Case libre la plus proche de `p` appartenant à la plus grande zone
/// connexe de rues de la tile (les cours fermées sont exclues, cf. P-01).
fn centre_accessible(v: &Ville, p: [f32; 2]) -> [f32; 2] {
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

/// Parcours en largeur depuis `centre` sur les cases libres (8 voisins).
fn champ_distance(v: &Ville, centre: [f32; 2]) -> Vec<u16> {
    let mut d = vec![u16::MAX; COTE * COTE];
    let mut file = std::collections::VecDeque::new();
    let (cx, cy) = (centre[0] as i32, centre[1] as i32);
    d[cy as usize * COTE + cx as usize] = 0;
    file.push_back((cx, cy));
    while let Some((x, y)) = file.pop_front() {
        let dc = d[y as usize * COTE + x as usize];
        for (dx, dy) in VOISINS8 {
            let (nx, ny) = (x + dx, y + dy);
            if v.bati(nx, ny) || (dx != 0 && dy != 0 && (v.bati(x + dx, y) || v.bati(x, y + dy))) {
                continue;
            }
            let k = ny as usize * COTE + nx as usize;
            if d[k] == u16::MAX {
                d[k] = dc + 1;
                file.push_back((nx, ny));
            }
        }
    }
    d
}

#[inline(always)]
fn accessible(t: &Tile, q: [f32; 2]) -> bool {
    !t.ville.bati_p(q) && t.champ[q[1] as usize * COTE + q[0] as usize] != u16::MAX
}

/// Direction de descente du champ depuis `pos`, si la case est accessible.
#[inline(always)]
fn descente(t: &Tile, pos: [f32; 2]) -> Option<[f32; 2]> {
    let (x, y) = (pos[0] as i32, pos[1] as i32);
    let lire = |x: i32, y: i32| {
        if x < 0 || y < 0 || x >= COTE as i32 || y >= COTE as i32 {
            u16::MAX
        } else {
            t.champ[y as usize * COTE + x as usize]
        }
    };
    let ici = lire(x, y);
    if ici == u16::MAX || ici == 0 {
        return None;
    }
    let mut meilleur = (ici, [0f32; 2]);
    for (dx, dy) in VOISINS8 {
        let v = lire(x + dx, y + dy);
        if v < meilleur.0 {
            meilleur = (v, [dx as f32, dy as f32]);
        }
    }
    let [dx, dy] = meilleur.1;
    let l = (dx * dx + dy * dy).sqrt();
    (l > 0.0).then(|| [dx / l, dy / l])
}

// ---------------------------------------------------------------- Systèmes

/// Phase 0 : humains. Marche aléatoire dans les rues accessibles (dans le
/// disque du groupe pour les scénarios calme et assaut), puis
/// reconstruction de l'index et de la carte de proximité.
fn humains(mut t: ResMut<Tiles>, r: Res<Reglages>, h: Res<Horloge>) {
    if !r.p.humains_mobiles {
        return;
    }
    let tick = h.0;
    let groupe = t.scenario != Scenario::Disperse;
    let pas = r.p.vitesse_humains * DT;
    ComputeTaskPool::get().scope(|s| {
        for (k, tile) in t.tiles.iter_mut().enumerate() {
            s.spawn(async move {
                let n = tile.humains.pos.len();
                for i in 0..n {
                    let g = hacher((k as u64) << 32 | i as u64, tick);
                    let mut cap = tile.humains.cap[i];
                    if (tick + i as u64) % 40 == 0 {
                        cap += (unite(g) - 0.5) * 2.0;
                    }
                    let p = tile.humains.pos[i];
                    let np = [p[0] + cap.cos() * pas, p[1] + cap.sin() * pas];
                    let ok = accessible(tile, np)
                        && (!groupe || d2(np, tile.centre) <= RAYON_GROUPE * RAYON_GROUPE);
                    if ok {
                        tile.humains.pos[i] = np;
                    } else {
                        cap += std::f32::consts::FRAC_PI_2
                            + unite(hacher(g, 1)) * std::f32::consts::PI;
                    }
                    tile.humains.cap[i] = cap;
                }
                let hu = &mut tile.humains;
                hu.index.reconstruire(&hu.pos);
                hu.proximite.reconstruire(&hu.pos);
            });
        }
    });
}

fn construire_grilles(
    mut g: ResMut<GrillesZombies>,
    inst: Res<Instantane>,
    al: Res<Alertes>,
    t: Res<Tiles>,
) {
    let n = t.zombies_par_tile;
    let inst = &inst.0;
    let al = &*al;
    ComputeTaskPool::get().scope(|s| {
        for (k, grille) in g.0.iter_mut().enumerate() {
            s.spawn(async move {
                let base = k * n;
                grille.reconstruire_avec(
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
            });
        }
    });
}

/// Un bruit porte-t-il jusqu'à `pos` (et n'est-il pas déjà atteint) ?
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

#[allow(clippy::too_many_arguments)] // prototype jetable
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
                // Veille profonde : rien entre deux réveils ; au réveil, un
                // test en O(1) sur la carte de proximité et sur les bruits.
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
                &g.0[id.tile as usize],
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

/// Alerte la plus fraîche parmi les voisins proches (état du tick précédent).
#[inline(always)]
fn meilleur_relais(
    pos: [f32; 2],
    rang: u32,
    g: &Grille,
    p: &Params,
    maintenant: u32,
) -> Option<(u32, [f32; 2])> {
    let r2 = p.rayon_relais * p.rayon_relais;
    let (cx, cy) = g.coord(pos);
    let m = g.cote as i32 - 1;
    let (y0, y1, x0, x1) = ((cy - 1).max(0), (cy + 1).min(m), (cx - 1).max(0), (cx + 1).min(m));
    // Filtre : aucune alerte dans les 9 cases, rien à lire. Exact, car la
    // limite de VOISINS_MAX ne fait qu'arrêter plus tôt une recherche qui ne
    // trouverait rien.
    if p.filtre_relais && !(y0..=y1).any(|y| (x0..=x1).any(|x| g.alerte_dans(x, y))) {
        return None;
    }
    let mut meilleur: Option<(u32, [f32; 2])> = None;
    let mut examines = 0u32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            for e in g.case(x, y) {
                if e.id == rang {
                    continue;
                }
                examines += 1;
                if e.alerte != 0
                    && maintenant - e.alerte <= p.fraicheur_alerte
                    && d2(e.p, pos) <= r2
                    && meilleur.is_none_or(|(a, _)| e.alerte > a)
                {
                    meilleur = Some((e.alerte, e.cible));
                }
                if examines >= VOISINS_MAX {
                    return meilleur;
                }
            }
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

/// Ordre de priorité : humain perçu > alerte relayée > bruit > dernière
/// position connue > errance (> veille profonde).
#[allow(clippy::too_many_arguments)] // prototype jetable
#[inline(always)]
fn decider(
    pos: [f32; 2],
    cap: f32,
    rang: u32,
    tile: &Tile,
    g: &Grille,
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
    if let Some(q) = percevoir_humain(pos, cap, tile, r, compte) {
        c.etat = POURSUITE;
        c.cible = q;
        c.alerte = maintenant;
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

/// Humain perçu (ouïe ou vue) le plus proche, s'il y en a un.
#[inline(always)]
fn percevoir_humain(
    pos: [f32; 2],
    cap: f32,
    tile: &Tile,
    r: &Reglages,
    compte: &mut [u64; 5],
) -> Option<[f32; 2]> {
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
        return Some(e.p);
    }
    for v in &res.vus[..res.nb] {
        compte[compteurs::LIGNES] += 1;
        if tile.ville.vue_degagee(pos, v.p) {
            return Some(v.p);
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

fn deplacer(
    mut q: Query<(&mut Corps, &Ident, &Cerveau)>,
    t: Res<Tiles>,
    g: Res<GrillesZombies>,
    inst: Res<Instantane>,
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
            let np = if cerveau.etat == PROFOND {
                deplacer_profond(pos, corps, id, tile, &r.p, h.0)
            } else {
                deplacer_un(
                    pos,
                    corps,
                    id,
                    cerveau,
                    tile,
                    &g.0[id.tile as usize],
                    &r.p,
                    h.0,
                )
            };
            if np != pos {
                corps.pos = [
                    tile.origine[0] + np[0] as f64,
                    tile.origine[1] + np[1] as f64,
                ];
                inst.0[id.tile as usize * n + id.rang as usize].store(coder(np), Relaxed);
            }
        });
}

/// Veille profonde : un pas toutes les `periode_veille` ticks, sans
/// séparation ni capacité, en sous-pas de 25 cm pour ne pas sauter un mur.
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
        let q = [
            np[0] + cap.cos() * pas / sous,
            np[1] + cap.sin() * pas / sous,
        ];
        if tile.ville.bati_p(q) {
            cap += std::f32::consts::FRAC_PI_2 + unite(hacher(graine, !tick)) * 2.0;
            break;
        }
        np = q;
    }
    corps.cap = cap;
    np
}

#[allow(clippy::too_many_arguments)] // prototype jetable
#[inline(always)]
fn deplacer_un(
    pos: [f32; 2],
    corps: &mut Corps,
    id: &Ident,
    c: &Cerveau,
    tile: &Tile,
    g: &Grille,
    p: &Params,
    tick: u64,
) -> [f32; 2] {
    let graine = (id.tile as u64) << 32 | id.rang as u64;
    let mut facteur = 1.0;
    if p.veille && c.etat == ERRANCE {
        let n = p.pas_veille.max(1) as u64;
        if (tick + id.rang as u64) % n != 0 {
            return pos;
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
        let groupe = d2(c.cible, tile.centre) < RAYON_GROUPE * RAYON_GROUPE;
        if d2c < 0.25 {
            (0.0, corps.cap)
        } else if let Some(dir) = (groupe && d2c > 15.0 * 15.0)
            .then(|| descente(tile, pos))
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
    'voisins: for y in (cy - 1).max(0)..=(cy + 1).min(m) {
        for x in (cx - 1).max(0)..=(cx + 1).min(m) {
            for e in g.case(x, y) {
                if e.id == id.rang {
                    continue;
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
                if examines >= VOISINS_MAX {
                    break 'voisins;
                }
            }
        }
    }
    let lc = (corr[0] * corr[0] + corr[1] * corr[1]).sqrt();
    if lc > 0.3 {
        corr = [corr[0] * 0.3 / lc, corr[1] * 0.3 / lc];
    }
    np = [np[0] + corr[0], np[1] + corr[1]];

    let v = &tile.ville;
    let ici = g.coord(pos);
    let libre = |q: [f32; 2]| {
        if v.bati_p(q) {
            return false;
        }
        let c = g.coord(q);
        if c == ici {
            return true;
        }
        let occupants = g.case(c.0, c.1).len() as u32;
        let cap = p.capacite_case;
        occupants < cap && (hacher(graine, tick) % cap as u64) as u32 >= occupants
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
    np
}

// ---------------------------------------------------------------- Simulation

pub struct Bilan {
    pub errance: usize,
    pub poursuite: usize,
    pub attire: usize,
    pub relais: usize,
    pub perdu: usize,
    pub profond: usize,
    pub densite_max: u32,
}

/// Une ligne du banc d'essai des structures.
pub struct LigneBanc {
    pub structure: Structure,
    pub case: f32,
    pub rejet: bool,
    pub requetes: u64,
    /// Temps mono-thread d'une requête complète (recherche + lignes de vue).
    pub ns_requete: f64,
    /// Temps de la recherche des candidats seule.
    pub ns_recherche: f64,
    pub reconstruction_us: f64,
    pub cases: f64,
    pub candidats: f64,
    pub lignes: f64,
    pub empreinte: u64,
}

pub const NB_PHASES: usize = 4;
pub const NOMS_PHASES: [&str; NB_PHASES] = ["humains", "grilles", "perception", "deplacement"];

pub struct Simulation {
    world: World,
    phases: [Schedule; NB_PHASES],
    pub couverture: f32,
}

impl Simulation {
    pub fn new(c: &ConfigSim) -> Self {
        ComputeTaskPool::get_or_init(|| TaskPoolBuilder::new().num_threads(c.threads).build());
        let p = &c.params;
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
                        (
                            f.next().unwrap().parse().unwrap(),
                            f.next().unwrap().parse().unwrap(),
                        )
                    })
                    .collect()
            }
        };
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
                    let (x, y) = *liste_reelle.get(k).unwrap_or_else(|| {
                        panic!("seulement {} tiles réelles", liste_reelle.len())
                    });
                    let v = Ville::charger(&d.join(format!("x{x}_y{y}.bin"))).expect("tile réelle");
                    (v, [x as f64 * 1000.0, y as f64 * 1000.0])
                }
            };
            let centre = centre_accessible(&ville, centre);
            let champ = champ_distance(&ville, centre);
            let acc = |q: [f32; 2]| champ[q[1] as usize * COTE + q[0] as usize] != u16::MAX;
            let tirer = |a: &mut Alea, rayon: f32| loop {
                let q = ville.point_libre(a, centre, rayon);
                if acc(q) {
                    return q;
                }
            };
            let hpos: Vec<[f32; 2]> = (0..c.humains)
                .map(|_| match c.scenario {
                    Scenario::Calme | Scenario::Assaut => tirer(&mut a, RAYON_GROUPE),
                    Scenario::Disperse => tirer(&mut a, 710.0),
                })
                .collect();
            let hcap: Vec<f32> = (0..c.humains)
                .map(|_| a.f32() * std::f32::consts::TAU)
                .collect();
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
            tiles.push(Tile {
                origine,
                centre,
                champ,
                ville,
                humains: Humains {
                    pos: hpos,
                    cap: hcap,
                    index,
                    proximite,
                },
                bruits,
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
                .map(|_| Grille::new(COTE_F, CASE_ZOMBIES))
                .collect(),
        ));
        let total = inst.len();
        world.insert_resource(Instantane(inst));
        world.insert_resource(Alertes {
            horodatage: (0..total).map(|_| AtomicU32::new(0)).collect(),
            cible: (0..total).map(|_| AtomicU64::new(0)).collect(),
            indexe: (0..total).map(|_| AtomicU8::new(1)).collect(),
        });
        world.insert_resource(Horloge(0));
        let demi = p.demi_angle_vue_deg.to_radians();
        world.insert_resource(Reglages {
            cos_demi_angle: demi.cos(),
            sin_demi_angle: demi.sin(),
            p: p.clone(),
            lots: BatchingStrategy::new().min_batch_size(c.lot_min.max(1)),
        });

        let mut phases: [Schedule; NB_PHASES] = Default::default();
        phases[0].add_systems(humains);
        phases[1].add_systems(construire_grilles);
        phases[2].add_systems(percevoir);
        phases[3].add_systems(deplacer);
        for s in &mut phases {
            s.set_executor(MultiThreadedExecutor::new());
            s.initialize(&mut world)
                .expect("initialisation du schedule");
        }
        Simulation {
            world,
            phases,
            couverture,
        }
    }

    /// Un tick ; renvoie la durée de chaque phase.
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

    /// Change la structure d'index des humains en cours de partie (sans
    /// effet sur le résultat : toutes les structures sont équivalentes).
    pub fn changer_structure(&mut self, s: Structure, case: f32) {
        self.world.resource_mut::<Reglages>().p.structure = s;
        self.world.resource_mut::<Reglages>().p.case = case;
        for t in &mut self.world.resource_mut::<Tiles>().tiles {
            let mut ix = Index::new(s, case);
            ix.reconstruire(&t.humains.pos);
            t.humains.index = ix;
        }
    }

    /// Empreinte de l'état (zombies et humains), indépendante de l'ordre.
    pub fn somme_controle(&mut self) -> u64 {
        let mut h = 0u64;
        let mut q = self.world.query::<(&Corps, &Cerveau, &Ident)>();
        for (c, cv, id) in q.iter(&self.world) {
            let x = hacher(
                c.pos[0].to_bits(),
                c.pos[1].to_bits() ^ c.cap.to_bits() as u64,
            );
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

    /// Retire tous les humains (test de la perte de trace).
    #[cfg(test)]
    pub fn retirer_humains(&mut self) {
        for t in &mut self.world.resource_mut::<Tiles>().tiles {
            t.humains.pos.clear();
            t.humains.cap.clear();
            t.humains.index.reconstruire(&[]);
            t.humains.proximite.reconstruire(&[]);
        }
    }

    #[cfg(test)]
    pub fn zombies_dans_les_murs(&mut self) -> usize {
        let mut q = self.world.query::<(&Corps, &Ident)>();
        let t = self.world.resource::<Tiles>();
        q.iter(&self.world)
            .filter(|(c, id)| {
                let tile = &t.tiles[id.tile as usize];
                tile.ville.bati_p(local(c.pos, tile.origine))
            })
            .count()
    }

    pub fn bilan(&mut self) -> Bilan {
        let mut b = Bilan {
            errance: 0,
            poursuite: 0,
            attire: 0,
            relais: 0,
            perdu: 0,
            profond: 0,
            densite_max: 0,
        };
        let mut q = self.world.query::<&Cerveau>();
        for c in q.iter(&self.world) {
            match c.etat {
                ERRANCE => b.errance += 1,
                POURSUITE => b.poursuite += 1,
                ATTIRE => b.attire += 1,
                RELAIS => b.relais += 1,
                PROFOND => b.profond += 1,
                _ => b.perdu += 1,
            }
        }
        b.densite_max = self
            .world
            .resource::<GrillesZombies>()
            .0
            .iter()
            .map(|g| g.densite_max())
            .max()
            .unwrap_or(0);
        b
    }

    /// Banc d'essai mono-thread des structures sur l'instantané courant :
    /// tous les zombies de toutes les tiles posent une requête de
    /// perception, avec chaque structure, sur les mêmes humains. Vérifie que
    /// toutes rendent la même cible.
    pub fn banc(
        &mut self,
        variantes: &[(Structure, f32, bool)],
        repetitions: usize,
    ) -> Vec<LigneBanc> {
        let mut q = self.world.query::<(&Corps, &Ident)>();
        let t = self.world.resource::<Tiles>();
        let r = self.world.resource::<Reglages>();
        let mut zombies: Vec<(u32, [f32; 2], f32)> = q
            .iter(&self.world)
            .map(|(c, id)| {
                (
                    id.tile,
                    local(c.pos, t.tiles[id.tile as usize].origine),
                    c.cap,
                )
            })
            .collect();
        zombies.sort_by_key(|z| z.0);
        let mut lignes = Vec::new();
        for &(s, case, rejet) in variantes {
            // Index de chaque tile, et coût de la reconstruction.
            let mut index: Vec<Index> = t.tiles.iter().map(|_| Index::new(s, case)).collect();
            let t0 = Instant::now();
            for _ in 0..repetitions.max(1) * 10 {
                for (ix, tile) in index.iter_mut().zip(&t.tiles) {
                    ix.reconstruire(std::hint::black_box(&tile.humains.pos));
                }
            }
            let reconstruction_us =
                t0.elapsed().as_secs_f64() * 1e6 / (repetitions.max(1) * 10 * t.tiles.len()) as f64;
            // Même logique de rejet que `percevoir_humain`.
            let preparer = |tile: &Tile, pos: [f32; 2], cap: f32| {
                let mut q = requete(pos, cap, r);
                if rejet {
                    let dmin = tile.humains.proximite.distance_min(pos);
                    if dmin > r.p.distance_vue.max(r.p.rayon_ouie) {
                        return None;
                    }
                    q.ouie_possible = dmin <= r.p.rayon_ouie;
                }
                Some(q)
            };
            // Recherche seule.
            let t0 = Instant::now();
            let mut bidon = 0usize;
            for _ in 0..repetitions {
                for &(k, pos, cap) in &zombies {
                    let tile = &t.tiles[k as usize];
                    let Some(q) = preparer(tile, pos, cap) else {
                        continue;
                    };
                    let mut c = Compte::default();
                    let res = index[k as usize].chercher(&q, &mut c);
                    bidon += res.nb + res.entendu.is_some() as usize;
                }
            }
            std::hint::black_box(bidon);
            let ns_recherche =
                t0.elapsed().as_secs_f64() * 1e9 / (repetitions * zombies.len()) as f64;
            // Requête complète, avec lignes de vue.
            let mut c = Compte::default();
            let mut nb_lignes = 0u64;
            let mut empreinte = 0u64;
            let t0 = Instant::now();
            for rep in 0..repetitions {
                for &(k, pos, cap) in &zombies {
                    let tile = &t.tiles[k as usize];
                    let mut cible = None;
                    if let Some(q) = preparer(tile, pos, cap) {
                        let res = index[k as usize].chercher(&q, &mut c);
                        cible = res.entendu.map(|e| e.id);
                        if cible.is_none() {
                            for v in &res.vus[..res.nb] {
                                nb_lignes += 1;
                                if tile.ville.vue_degagee(pos, v.p) {
                                    cible = Some(v.id);
                                    break;
                                }
                            }
                        }
                    }
                    if rep == 0 {
                        empreinte = empreinte
                            .wrapping_mul(31)
                            .wrapping_add(cible.map_or(u64::MAX, |x| x as u64));
                    }
                }
            }
            let ns_requete =
                t0.elapsed().as_secs_f64() * 1e9 / (repetitions * zombies.len()) as f64;
            let n = (repetitions * zombies.len()) as f64;
            lignes.push(LigneBanc {
                structure: s,
                case,
                rejet,
                requetes: zombies.len() as u64,
                ns_requete,
                ns_recherche,
                reconstruction_us,
                cases: c.cases as f64 / n,
                candidats: c.candidats as f64 / n,
                lignes: nb_lignes as f64 / n,
                empreinte,
            });
        }
        lignes
    }
}
