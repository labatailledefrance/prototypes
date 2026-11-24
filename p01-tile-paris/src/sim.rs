//! Simulation des zombies d'une ou plusieurs tiles parisiennes actives, avec
//! `bevy_ecs` (exécuteur multi-thread, requêtes `par_iter_mut`).
//!
//! Un tick = trois phases, chronométrées séparément :
//! 1. **grilles** : chaque tile reconstruit sa grille spatiale de zombies
//!    (cases de 2 m) à partir de l'instantané des positions ;
//! 2. **perception** : relais d'alerte entre voisins (variante), puis ouïe
//!    (humains proches, à travers les murs), vue (cône, distance maximale,
//!    ligne de vue sur la grille du bâti), bruits, dernière position connue ;
//! 3. **déplacement** : vers la cible ou en errance, séparation entre
//!    zombies (collisions simples), glissement le long des bâtiments.
//!
//! Chaque phase ne lit que l'état du tick précédent et n'écrit que l'entité
//! qu'elle traite : le résultat ne dépend ni du nombre de threads ni du
//! découpage en lots (vérifié par les tests).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use bevy_ecs::batching::BatchingStrategy;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::MultiThreadedExecutor;
use bevy_tasks::{ComputeTaskPool, TaskPoolBuilder};
use clap::ValueEnum;

use crate::alea::{Alea, hacher, unite};
use crate::grille::Grille;
use crate::ville::{COTE_F, Ville};

/// Pas de temps : 20 ticks par seconde de jeu.
pub const DT: f32 = 0.05;
/// Côté d'une case de la grille des zombies (collisions).
const CASE_ZOMBIES: f32 = 2.0;
/// Côté d'une case de la grille des humains (perception).
const CASE_HUMAINS: f32 = 25.0;
/// Nombre maximal de voisins examinés pour la séparation.
const VOISINS_MAX: u32 = 16;
/// Nombre maximal de lignes de vue tracées par zombie et par perception.
const RAYONS_MAX: usize = 8;
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
    /// Variante : relais de l'alerte entre voisins (ZOMBIES.md).
    pub relais: bool,
    /// Distance maximale de transmission de l'alerte entre deux zombies.
    pub rayon_relais: f32,
    /// Âge maximal d'une alerte relayée (ticks depuis la dernière observation
    /// directe) : au-delà, elle est perdue.
    pub fraicheur_alerte: u32,
    /// Âge (ticks) sous lequel une alerte voisine dispense de percevoir : un
    /// voisin a vu la cible directement il y a moins que cela.
    pub seuil_source: u32,
    /// Variante : mise en veille des zombies sans stimulus (A-03).
    pub veille: bool,
    /// En veille : perception une fois tous les N ticks.
    pub periode_veille: u32,
    /// En veille : déplacement un tick sur N, avec un pas N fois plus long.
    pub pas_veille: u32,
    /// Durée pendant laquelle un zombie qui a perdu la trace continue vers la
    /// dernière position connue (ticks).
    pub duree_derniere_position: u32,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            vitesse_course: 5.0,
            vitesse_errance: 1.0,
            rayon_ouie: 40.0,
            distance_vue: 200.0,
            demi_angle_vue_deg: 60.0,
            distance_separation: 0.7,
            rayon_bruit: 600.0,
            // 6 par m² : foule très compacte.
            capacite_case: 24,
            periode_perception: 5,
            rayons: 4,
            relais: false,
            rayon_relais: 3.0,
            // 3 s : portée d'environ 55 sauts, soit 150 m de foule.
            fraicheur_alerte: 60,
            seuil_source: 5,
            veille: false,
            periode_veille: 20,
            pas_veille: 4,
            // « Une petite minute ».
            duree_derniere_position: 1200,
        }
    }
}

/// Origine des emprises des bâtiments.
#[derive(Clone)]
pub enum SourceVille {
    Synthetique,
    /// Dossier produit par `outils/extraire_tiles.py` ; tiles prises dans
    /// l'ordre de `tiles.csv` (de la plus bâtie à la moins bâtie).
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

/// Données écrites par la perception. L'état est un champ, pas un marqueur
/// (il change souvent, cf. F-02).
#[derive(Component)]
struct Cerveau {
    etat: u8,
    /// Cible en coordonnées locales à la tile.
    cible: [f32; 2],
    /// POURSUITE et RELAIS : tick (+1) de la dernière observation directe de
    /// la cible, par ce zombie ou par la source de l'alerte. Un relais le
    /// recopie sans le rafraîchir : seule une observation directe le
    /// rajeunit (principe des numéros de séquence de DSDV ou AODV).
    alerte: u32,
    /// PERDU : tick de la perte de la trace.
    perdu_depuis: u32,
}

/// Identité, jamais modifiée.
#[derive(Component)]
struct Ident {
    tile: u32,
    /// Rang du zombie dans sa tile.
    rang: u32,
}

// ---------------------------------------------------------------- Ressources

struct Tile {
    origine: [f64; 2],
    ville: Ville,
    /// Centre du groupe d'humains (et des bruits).
    centre: [f32; 2],
    /// Distance à pied (m, 8-connexité approchée) de chaque case de 1 m au
    /// centre, calculée une fois : u16::MAX = inaccessible. Version minimale
    /// d'un flow field (P-03), pour que la foule contourne les îlots.
    champ: Vec<u16>,
    humains: Grille,
    bruits: Vec<[f32; 2]>,
}

#[derive(Resource)]
struct Tiles {
    tiles: Vec<Tile>,
    zombies_par_tile: usize,
}

#[derive(Resource)]
struct GrillesZombies(Vec<Grille>);

/// Positions locales de tous les zombies à la fin du tick précédent (deux
/// `f32` dans un `u64` atomique, pour l'écriture parallèle sans `unsafe`).
#[derive(Resource)]
struct Instantane(Vec<AtomicU64>);

/// Alerte publiée par chaque zombie à la fin de sa perception : horodatage
/// (0 = aucune) et cible. Lue au tick suivant, via la grille.
#[derive(Resource)]
struct Alertes {
    horodatage: Vec<AtomicU32>,
    cible: Vec<AtomicU64>,
}

#[derive(Resource)]
struct Horloge(u64);

#[derive(Resource)]
struct Reglages {
    p: Params,
    cos_demi_angle: f32,
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

/// Case libre la plus proche de `p` appartenant à la plus grande zone
/// connexe de rues de la tile. Sur une tile réelle, la case libre la plus
/// proche du centre peut être une cour fermée (porche absent de la BD TOPO) :
/// un groupe placé là s'entasserait dans quelques dizaines de m².
fn centre_accessible(v: &Ville, p: [f32; 2]) -> [f32; 2] {
    use crate::ville::COTE;
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
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
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
    use crate::ville::COTE;
    let mut d = vec![u16::MAX; COTE * COTE];
    let mut file = std::collections::VecDeque::new();
    let (cx, cy) = (centre[0] as i32, centre[1] as i32);
    d[cy as usize * COTE + cx as usize] = 0;
    file.push_back((cx, cy));
    while let Some((x, y)) = file.pop_front() {
        let dc = d[y as usize * COTE + x as usize];
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            // Diagonale : seulement si les deux cases adjacentes sont libres.
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

/// Direction de descente du champ depuis `pos`, si la case est accessible.
#[inline(always)]
fn descente(t: &Tile, pos: [f32; 2]) -> Option<[f32; 2]> {
    use crate::ville::COTE;
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
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
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

fn construire_grilles(
    mut g: ResMut<GrillesZombies>,
    inst: Res<Instantane>,
    al: Res<Alertes>,
    t: Res<Tiles>,
) {
    let n = t.zombies_par_tile;
    let inst = &inst.0;
    let al = &*al;
    // Une tâche par tile : les tiles sont indépendantes.
    ComputeTaskPool::get().scope(|s| {
        for (k, grille) in g.0.iter_mut().enumerate() {
            s.spawn(async move {
                let base = k * n;
                grille.reconstruire_avec(
                    n,
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
            // Étalement : chaque zombie perçoit un tick sur `periode` (sur
            // `periode_veille` s'il est en veille).
            let per = if r.p.veille && c.etat == ERRANCE {
                r.p.periode_veille.max(1) as u64
            } else {
                periode
            };
            let due = (h.0 + id.rang as u64) % per == 0;
            let maintenant = h.0 as u32 + 1;
            decider(pos, corps.cap, id.rang, tile, &g.0[id.tile as usize], &r, c, due, maintenant);
            // Publication de l'alerte pour les voisins, lue au tick suivant.
            let k = id.tile as usize * n + id.rang as usize;
            let relayable = c.etat == POURSUITE || c.etat == RELAIS;
            al.horodatage[k].store(if relayable { c.alerte } else { 0 }, Relaxed);
            if relayable {
                al.cible[k].store(coder(c.cible), Relaxed);
            }
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
    let mut meilleur: Option<(u32, [f32; 2])> = None;
    let mut examines = 0u32;
    for y in (cy - 1).max(0)..=(cy + 1).min(m) {
        for x in (cx - 1).max(0)..=(cx + 1).min(m) {
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
/// position connue > errance.
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
) {
    let p = &r.p;
    // Relais : alerte la plus fraîche des voisins (tick précédent).
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
        // Entre deux perceptions, l'alerte se propage à chaque tick.
        if c.etat != POURSUITE {
            if let Some(rel) = relais {
                suivre(c, rel);
            } else if c.etat == RELAIS {
                // Plus aucune alerte fraîche : la source a perdu la cible.
                perdre(c, maintenant);
            }
        }
        return;
    }
    // Un voisin vient de voir la cible directement : on le suit sans
    // percevoir (économie). Les sources tournent ainsi d'elles-mêmes.
    if let Some(rel) = relais.filter(|(a, _)| maintenant - a <= p.seuil_source) {
        suivre(c, rel);
        return;
    }
    if let Some(q) = percevoir_humain(pos, cap, tile, r) {
        c.etat = POURSUITE;
        c.cible = q;
        c.alerte = maintenant;
        return;
    }
    // Ne voit pas la cible, mais la horde la voit encore.
    if let Some(rel) = relais {
        suivre(c, rel);
        return;
    }
    if c.etat == POURSUITE || c.etat == RELAIS {
        perdre(c, maintenant);
    }
    // Bruits : le plus proche qui porte jusqu'au zombie.
    let rb2 = p.rayon_bruit * p.rayon_bruit;
    let arrivee2 = ARRIVEE_BRUIT * ARRIVEE_BRUIT;
    let mut bruit: Option<(f32, [f32; 2])> = None;
    for &b in &tile.bruits {
        let dd = d2(b, pos);
        if dd <= rb2 && dd > arrivee2 && bruit.is_none_or(|(x, _)| dd < x) {
            bruit = Some((dd, b));
        }
    }
    if let Some((_, b)) = bruit {
        c.etat = ATTIRE;
        c.cible = b;
        return;
    }
    // Dernière position connue, pendant « une petite minute ».
    if c.etat == PERDU
        && maintenant - c.perdu_depuis < p.duree_derniere_position
        && d2(c.cible, pos) > 4.0
    {
        return;
    }
    c.etat = ERRANCE;
}

/// Humain perçu (ouïe ou vue) le plus proche, s'il y en a un.
fn percevoir_humain(pos: [f32; 2], cap: f32, tile: &Tile, r: &Reglages) -> Option<[f32; 2]> {
    let p = &r.p;
    let dir = [cap.cos(), cap.sin()];
    let ouie2 = p.rayon_ouie * p.rayon_ouie;
    let vue2 = p.distance_vue * p.distance_vue;
    let k_max = p.rayons.min(RAYONS_MAX);

    // Humain entendu le plus proche (ouïe : omnidirectionnelle, à travers
    // les murs), et les k humains les plus proches dans le cône de vue.
    let mut entendu: Option<(f32, [f32; 2])> = None;
    let mut vus = [(f32::INFINITY, [0f32; 2]); RAYONS_MAX];
    let mut nb_vus = 0usize;

    let g = &tile.humains;
    let rc = (p.distance_vue / CASE_HUMAINS).ceil() as i32;
    let (cx, cy) = g.coord(pos);
    let m = g.cote as i32 - 1;
    for y in (cy - rc).max(0)..=(cy + rc).min(m) {
        for x in (cx - rc).max(0)..=(cx + rc).min(m) {
            for e in g.case(x, y) {
                let d = [e.p[0] - pos[0], e.p[1] - pos[1]];
                let dd = d[0] * d[0] + d[1] * d[1];
                if dd <= ouie2 {
                    if entendu.is_none_or(|(b, _)| dd < b) {
                        entendu = Some((dd, e.p));
                    }
                } else if dd <= vue2
                    && k_max > 0
                    && d[0] * dir[0] + d[1] * dir[1] >= r.cos_demi_angle * dd.sqrt()
                {
                    // Insertion dans les k plus proches (tableau trié).
                    if nb_vus < k_max || dd < vus[nb_vus - 1].0 {
                        let mut i = nb_vus.min(k_max - 1);
                        while i > 0 && vus[i - 1].0 > dd {
                            vus[i] = vus[i - 1];
                            i -= 1;
                        }
                        vus[i] = (dd, e.p);
                        nb_vus = (nb_vus + 1).min(k_max);
                    }
                }
            }
        }
    }

    let mut cible = entendu;
    for &(dd, q) in &vus[..nb_vus] {
        if cible.is_some_and(|(b, _)| b <= dd) {
            break;
        }
        if tile.ville.vue_degagee(pos, q) {
            cible = Some((dd, q));
            break;
        }
    }
    cible.map(|(_, q)| q)
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
            let grille = &g.0[id.tile as usize];
            let pos = local(corps.pos, tile.origine);
            let np = deplacer_un(pos, corps, id, cerveau, tile, grille, &r.p, h.0);
            corps.pos = [
                tile.origine[0] + np[0] as f64,
                tile.origine[1] + np[1] as f64,
            ];
            inst.0[id.tile as usize * n + id.rang as usize].store(coder(np), Relaxed);
        });
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
    // Veille : un tick sur `pas_veille`, avec un pas d'autant plus long.
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
        // Changement de direction toutes les 2 s environ.
        if (tick + id.rang as u64) % 40 < facteur as u64 {
            cap += (unite(hacher(graine, tick)) - 0.5) * 2.0;
        }
        (p.vitesse_errance, cap)
    } else {
        let d = [c.cible[0] - pos[0], c.cible[1] - pos[1]];
        let d2c = d[0] * d[0] + d[1] * d[1];
        // Cible du groupe central (bruit ou humain du groupe), hors de
        // portée immédiate : on suit le champ pour contourner les îlots.
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

    // Séparation : on repousse les voisins trop proches (positions du tick
    // précédent), en examinant au plus VOISINS_MAX voisins.
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

    // Bâtiments et foule : une case est franchissable si elle n'est pas
    // bâtie et, si on y entre, si elle n'est pas pleine (capacité, cf. A-05).
    // Glissement le long de l'obstacle, sinon arrêt.
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
        // Admission progressive : une case à moitié pleine laisse entrer un
        // candidat sur deux. Évite que des dizaines de zombies entrent au
        // même tick dans une case presque pleine.
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
    /// Plus grand nombre de zombies dans une case de 2 m × 2 m.
    pub densite_max: u32,
}

pub struct Simulation {
    world: World,
    phases: [Schedule; 3],
    pub couverture: f32,
}

pub const NOMS_PHASES: [&str; 3] = ["grilles", "perception", "deplacement"];

impl Simulation {
    pub fn new(c: &ConfigSim) -> Self {
        ComputeTaskPool::get_or_init(|| TaskPoolBuilder::new().num_threads(c.threads).build());
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
            // Le centre peut être bâti, ou dans une cour fermée, sur une tile
            // réelle : on prend la rue la plus proche du réseau principal.
            let centre = centre_accessible(&ville, centre);
            // Espace accessible depuis le centre seulement : les cours
            // fermées (sans porche dans la BD TOPO) sont exclues.
            let champ = champ_distance(&ville, centre);
            let accessible = |q: [f32; 2]| {
                champ[q[1] as usize * crate::ville::COTE + q[0] as usize] != u16::MAX
            };
            let tirer = |a: &mut Alea, rayon: f32| loop {
                let q = ville.point_libre(a, centre, rayon);
                if accessible(q) {
                    return q;
                }
            };
            let humains: Vec<[f32; 2]> = (0..c.humains)
                .map(|_| match c.scenario {
                    Scenario::Calme | Scenario::Assaut => tirer(&mut a, RAYON_GROUPE),
                    Scenario::Disperse => tirer(&mut a, 710.0),
                })
                .collect();
            let mut gh = Grille::new(COTE_F, CASE_HUMAINS);
            gh.reconstruire(humains.len(), |i| humains[i]);
            let bruits = match c.scenario {
                Scenario::Assaut => vec![centre],
                _ => vec![],
            };
            for rang in 0..c.zombies {
                let p = tirer(&mut a, 710.0);
                inst.push(AtomicU64::new(coder(p)));
                zombies.push((
                    Corps {
                        pos: [origine[0] + p[0] as f64, origine[1] + p[1] as f64],
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
                humains: gh,
                bruits,
            });
        }
        world.spawn_batch(zombies);
        let couverture = tiles.iter().map(|t| t.ville.couverture).sum::<f32>() / c.tiles as f32;
        world.insert_resource(Tiles {
            tiles,
            zombies_par_tile: c.zombies,
        });
        world.insert_resource(GrillesZombies(
            (0..c.tiles).map(|_| Grille::new(COTE_F, CASE_ZOMBIES)).collect(),
        ));
        let total = inst.len();
        world.insert_resource(Instantane(inst));
        world.insert_resource(Alertes {
            horodatage: (0..total).map(|_| AtomicU32::new(0)).collect(),
            cible: (0..total).map(|_| AtomicU64::new(0)).collect(),
        });
        world.insert_resource(Horloge(0));
        world.insert_resource(Reglages {
            cos_demi_angle: c.params.demi_angle_vue_deg.to_radians().cos(),
            p: c.params.clone(),
            lots: BatchingStrategy::new().min_batch_size(c.lot_min.max(1)),
        });

        let mut phases = [Schedule::default(), Schedule::default(), Schedule::default()];
        phases[0].add_systems(construire_grilles);
        phases[1].add_systems(percevoir);
        phases[2].add_systems(deplacer);
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

    /// Un tick ; renvoie la durée de chaque phase.
    pub fn tick(&mut self) -> [Duration; 3] {
        let mut d = [Duration::ZERO; 3];
        for (i, s) in self.phases.iter_mut().enumerate() {
            let t = Instant::now();
            s.run(&mut self.world);
            d[i] = t.elapsed();
        }
        self.world.resource_mut::<Horloge>().0 += 1;
        d
    }

    /// Empreinte de l'état (positions et états), indépendante de l'ordre.
    pub fn somme_controle(&mut self) -> u64 {
        let mut h = 0u64;
        let mut q = self.world.query::<(&Corps, &Cerveau, &Ident)>();
        for (c, cv, id) in q.iter(&self.world) {
            let x = hacher(c.pos[0].to_bits(), c.pos[1].to_bits() ^ c.cap.to_bits() as u64);
            let y = hacher(cv.etat as u64, (id.tile as u64) << 32 | id.rang as u64);
            h = h.wrapping_add(hacher(x, y));
        }
        h
    }

    /// Retire tous les humains (test de la perte de trace).
    #[cfg(test)]
    pub fn retirer_humains(&mut self) {
        for t in &mut self.world.resource_mut::<Tiles>().tiles {
            t.humains.reconstruire(0, |_| [0.0; 2]);
        }
    }

    /// Nombre de zombies dont la position est dans une case bâtie.
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
            densite_max: 0,
        };
        let mut q = self.world.query::<&Cerveau>();
        for c in q.iter(&self.world) {
            match c.etat {
                ERRANCE => b.errance += 1,
                POURSUITE => b.poursuite += 1,
                ATTIRE => b.attire += 1,
                RELAIS => b.relais += 1,
                _ => b.perdu += 1,
            }
        }
        // Les grilles décrivent le début du tick écoulé.
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
}

