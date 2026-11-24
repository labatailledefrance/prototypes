//! Individualisation et retour à l'agrégat (représentation B).
//!
//! Un lieu passe par quatre états : agrégé (dossier), individualisation en
//! cours, individualisé (ses colons sont des entités), retour en cours. Les
//! déclencheurs mesurés : caméra proche (rayon, puis rayon de retour plus
//! large, et délai), et chantiers extérieurs (« colon qui sort travailler »).
//! Le passage est étalé : au plus `lot_ind` colons créés et `lot_ret`
//! détruits par tick, comme l'activation de P-04.
//!
//! Un colon individualisé vit avec la même fonction `vivre`, à la même
//! phase : sans marche réelle, la simulation est identique à celle sans
//! caméra (somme de contrôle). Sous la caméra, un colon qui part marche
//! réellement (A* sur les cases de 2 m), ce qui change son heure d'arrivée.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use bevy_ecs::prelude::*;
use bevy_utils::Parallel;

use crate::carte::Carte;
use crate::civils::{Civils, Commun, Dossiers, Fiche, Horloge, Place, Position, Reglage, SortiesA, vivre_entite};
use crate::colon::*;
use crate::lieux::Genre;
use crate::marche::{Astar, Passage, porte};

pub const AGREGE: u8 = 0;
pub const EN_COURS: u8 = 1;
pub const INDIVIDUALISE: u8 = 2;
pub const RETOUR: u8 = 3;

#[derive(Component)]
pub struct Individu;

#[derive(Resource, Default)]
pub struct CollecteInd(pub Parallel<SortiesA>);

#[derive(Resource, Default)]
pub struct ResultatInd {
    pub s: Sorties,
    pub morts: Vec<(u32, Entity)>,
}

pub struct Marcheur {
    pub e: Entity,
    pub id: u32,
    pub vers: u32,
    pub chemin: Vec<[f64; 2]>,
    pub i: usize,
}

#[derive(Default, Clone, Debug)]
pub struct StatsInd {
    pub lieux_ind: u64,
    pub colons_ind: u64,
    pub lieux_ret: u64,
    pub colons_ret: u64,
    pub rebascules: u64,
    pub ticks_ind: Vec<u32>,
    pub ns_ind: u64,
    pub ns_ret: u64,
    pub ns_camera: u64,
    pub max_entites: u64,
    pub chemins: u64,
    pub chemins_echoues: u64,
    pub expansions: u64,
    pub ns_chemins: u64,
    pub ns_chemin_max: u64,
    pub marcheurs_max: u64,
    pub arrivees_marche: u64,
    pub ns_marche: u64,
    pub entites: u64,
    /// Attente au plus d'un colon devant sa porte, faute de budget de chemin
    /// (ticks).
    pub attente_max: u32,
}

#[derive(Resource)]
pub struct Indiv {
    pub etat: Vec<u8>,
    pub vu: Vec<u32>,
    debut: Vec<u32>,
    retourne: Vec<bool>,
    file_ind: VecDeque<u32>,
    file_ret: VecDeque<u32>,
    en_cours: Vec<u32>,
    pub camera: Option<[f64; 2]>,
    pub rayon: f64,
    pub rayon_retour: f64,
    pub delai: u32,
    pub lot_ind: usize,
    pub lot_ret: usize,
    grille: HashMap<(i64, i64), Vec<u32>>,
    pub portes: Vec<Option<[f64; 2]>>,
    pub passage: Option<Passage>,
    pub marche: bool,
    /// Entités individualisées de chaque lieu (destination si en trajet
    /// abstrait), comme la table des rangs d'un slot de P-04.
    pub membres: Vec<Vec<Entity>>,
    pub max_expansions: u64,
    astar: Astar,
    pub budget_expansions: u64,
    file_chemins: VecDeque<(Entity, u32, u32, u32, u32)>,
    pub marcheurs: Vec<Marcheur>,
    /// Lieux toujours individualisés (variante : chantiers).
    permanent: Vec<bool>,
    pub stats: StatsInd,
}

const CASE: f64 = 100.0;

impl Indiv {
    pub fn new(cm: &Commun, carte: Option<Carte>, marche: bool) -> Indiv {
        let n = cm.lieux.len();
        let mut grille: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, l) in cm.lieux.iter().enumerate() {
            grille.entry(((l.pos[0] / CASE).floor() as i64, (l.pos[1] / CASE).floor() as i64)).or_default().push(i as u32);
        }
        let portes = match &carte {
            Some(c) => cm.lieux.iter().map(|l| porte(c, l.pos)).collect(),
            None => vec![None; n],
        };
        Indiv {
            etat: vec![AGREGE; n],
            vu: vec![0; n],
            debut: vec![0; n],
            retourne: vec![false; n],
            file_ind: VecDeque::new(),
            file_ret: VecDeque::new(),
            en_cours: vec![],
            camera: None,
            rayon: 250.0,
            rayon_retour: 350.0,
            delai: 30 * TPS,
            lot_ind: 2_000,
            lot_ret: 2_000,
            grille,
            portes,
            passage: carte.as_ref().map(Passage::new),
            marche,
            membres: vec![vec![]; n],
            max_expansions: 60_000,
            astar: Astar::default(),
            budget_expansions: 20_000,
            file_chemins: VecDeque::new(),
            marcheurs: vec![],
            permanent: vec![false; n],
            stats: StatsInd::default(),
        }
    }

    /// Les colons qui arrivent dans ce lieu deviennent-ils des entités ?
    pub fn accueille_individus(&self, l: u32) -> bool {
        matches!(self.etat[l as usize], EN_COURS | INDIVIDUALISE)
    }

    /// Variante « colon qui sort travailler » : les chantiers extérieurs sont
    /// toujours individualisés.
    pub fn chantiers_permanents(&mut self, cm: &Commun) {
        for (i, l) in cm.lieux.iter().enumerate() {
            if l.genre == Genre::Chantier {
                self.permanent[i] = true;
                self.etat[i] = EN_COURS;
                self.file_ind.push_back(i as u32);
            }
        }
    }
}

fn retirer(v: &mut Vec<Entity>, e: Entity) {
    let k = v.iter().position(|&x| x == e).expect("entité absente de son lieu");
    v.swap_remove(k);
}

/// Crée les entités de colons arrivés dans un lieu individualisé (ou en
/// cours d'individualisation).
pub fn creer(w: &mut World, v: &[(Colon, u32)], t: u32) {
    let cm = w.resource::<Commun>();
    let lots: Vec<(Fiche, Place, Position, Individu)> = v
        .iter()
        .map(|(c, l)| {
            let li = &cm.lieux[*l as usize];
            (Fiche(*c), Place { lieu: *l, phase: li.phase, arrivee: t }, Position(li.pos), Individu)
        })
        .collect();
    let es: Vec<Entity> = w.spawn_batch(lots).collect();
    let mut ind = w.resource_mut::<Indiv>();
    for (e, (_, l)) in es.into_iter().zip(v) {
        ind.membres[*l as usize].push(e);
    }
}

/// Décès d'un colon individualisé.
pub fn mort(w: &mut World, e: Entity) {
    let l = w.get::<Place>(e).unwrap().lieu;
    retirer(&mut w.resource_mut::<Indiv>().membres[l as usize], e);
    w.despawn(e);
}

/// Mise à jour des colons individualisés (même logique, même phase).
#[allow(clippy::too_many_arguments)]
pub fn maj_individus(
    mut q: Query<(Entity, &mut Fiche, &mut Place, &mut Position), With<Individu>>,
    cm: Res<Commun>,
    h: Res<Horloge>,
    reg: Res<Reglage>,
    col: ResMut<CollecteInd>,
    mut res: ResMut<ResultatInd>,
) {
    let t = h.0;
    let g = cm.ctx_global(t);
    let k = t % cm.p.periode;
    let colonies = cm.colonies;
    let par = &col.0;
    let cm = &*cm;
    q.par_iter_mut().batching_strategy(reg.lots.clone()).for_each(|(e, mut fiche, mut place, mut pos)| {
        let pl = place.bypass_change_detection();
        if pl.phase != k || pl.arrivee > t {
            return;
        }
        let mut o = par.borrow_local_mut();
        o.s.preparer(colonies);
        vivre_entite(e, fiche.bypass_change_detection(), pl, pos.bypass_change_detection(), cm, &g, &mut o);
    });
    let mut col = col;
    res.s.preparer(colonies);
    let r = &mut *res;
    for o in col.0.iter_mut() {
        r.s.fusionner(&mut o.s);
        o.s.vider();
        o.changes.clear();
        r.morts.append(&mut o.morts);
    }
}

fn d2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// Départ d'un colon individualisé : marche réelle sous la caméra, reste une
/// entité vers un lieu individualisé, sinon retourne à l'agrégat (trajet
/// abstrait).
pub fn depart(c: &mut Civils, d: &Depart, t: u32) {
    let e = Entity::from_bits(d.e);
    let w = &mut c.world;
    let (marche, proche, portes) = {
        let ind = w.resource::<Indiv>();
        let cm = w.resource::<Commun>();
        let proche = ind
            .camera
            .is_some_and(|cam| d2(cam, cm.pos_lieux[d.de as usize]) <= ind.rayon_retour * ind.rayon_retour);
        let portes = ind.portes[d.de as usize].is_some() && ind.portes[d.vers as usize].is_some();
        (ind.marche, proche, portes)
    };
    retirer(&mut w.resource_mut::<Indiv>().membres[d.de as usize], e);
    if marche && proche && portes {
        w.resource_mut::<Commun>().occ[d.de as usize] -= 1;
        w.get_mut::<Place>(e).unwrap().arrivee = u32::MAX;
        w.resource_mut::<Indiv>().file_chemins.push_back((e, d.c.id, d.de, d.vers, t));
        return;
    }
    abstrait(w, e, d);
    let _ = t;
}

/// Trajet abstrait d'un individu : il se fait toujours à l'état agrégé (le
/// colon redevient une entité à l'arrivée si son lieu est individualisé).
/// Une entité n'existe donc que dans un lieu, ou en marche réelle.
fn abstrait(w: &mut World, e: Entity, d: &Depart) {
    w.despawn(e);
    w.resource_mut::<Commun>().partir(d, true);
}

/// Avant les mises à jour : caméra, individualisations et retours étalés,
/// chemins et marche.
pub fn avant(c: &mut Civils, t: u32) {
    let w = &mut c.world;
    let t0 = Instant::now();
    // 1. Caméra : lieux vus, nouveaux lieux à individualiser.
    w.resource_scope(|w, mut ind: Mut<Indiv>| {
        let ind = &mut *ind;
        if let Some(cam) = ind.camera {
            let cm = w.resource::<Commun>();
            let r = ind.rayon_retour;
            let (cx0, cx1) = (((cam[0] - r) / CASE).floor() as i64, ((cam[0] + r) / CASE).floor() as i64);
            let (cy0, cy1) = (((cam[1] - r) / CASE).floor() as i64, ((cam[1] + r) / CASE).floor() as i64);
            let mut nouveaux = vec![];
            for cy in cy0..=cy1 {
                for cx in cx0..=cx1 {
                    let Some(v) = ind.grille.get(&(cx, cy)) else { continue };
                    for &l in v {
                        let dd = d2(cam, cm.pos_lieux[l as usize]);
                        if dd > r * r {
                            continue;
                        }
                        let li = l as usize;
                        match ind.etat[li] {
                            AGREGE if dd <= ind.rayon * ind.rayon => nouveaux.push(l),
                            AGREGE => {}
                            RETOUR => {
                                if dd <= ind.rayon * ind.rayon {
                                    ind.etat[li] = INDIVIDUALISE;
                                    ind.vu[li] = t;
                                }
                            }
                            _ => ind.vu[li] = t,
                        }
                    }
                }
            }
            nouveaux.sort_unstable();
            for l in nouveaux {
                let li = l as usize;
                ind.etat[li] = EN_COURS;
                ind.vu[li] = t;
                ind.debut[li] = t;
                if ind.retourne[li] {
                    ind.stats.rebascules += 1;
                }
                ind.file_ind.push_back(l);
                ind.en_cours.push(l);
            }
        }
        // Délai écoulé : retour à l'agrégat.
        let mut k = 0;
        while k < ind.en_cours.len() {
            let l = ind.en_cours[k] as usize;
            if ind.etat[l] == INDIVIDUALISE && !ind.permanent[l] && t.saturating_sub(ind.vu[l]) > ind.delai {
                ind.etat[l] = RETOUR;
                ind.file_ret.push_back(l as u32);
            }
            if ind.etat[l] == AGREGE {
                ind.en_cours.swap_remove(k);
            } else {
                k += 1;
            }
        }
    });
    let t1 = Instant::now();
    // 2. Individualisation étalée.
    let mut budget = w.resource::<Indiv>().lot_ind;
    let mut lots: Vec<(Colon, u32)> = vec![];
    while budget > 0 {
        let Some(&l) = w.resource::<Indiv>().file_ind.front() else { break };
        let li = l as usize;
        if w.resource::<Indiv>().etat[li] != EN_COURS {
            w.resource_mut::<Indiv>().file_ind.pop_front();
            continue;
        }
        let mut ds = w.resource_mut::<Dossiers>();
        let d = &mut ds.0[li];
        let k = budget.min(d.len());
        lots.extend(d.drain(..k).map(|c| (c, l)));
        let vide = d.is_empty();
        budget -= k;
        if vide {
            let mut ind = w.resource_mut::<Indiv>();
            ind.etat[li] = INDIVIDUALISE;
            ind.file_ind.pop_front();
            ind.stats.lieux_ind += 1;
            let dt = t - ind.debut[li] + 1;
            ind.stats.ticks_ind.push(dt);
        }
    }
    if !lots.is_empty() {
        let cm = w.resource::<Commun>();
        let v: Vec<(Fiche, Place, Position, Individu)> = lots
            .iter()
            .map(|(c, l)| {
                let li = &cm.lieux[*l as usize];
                (Fiche(*c), Place { lieu: *l, phase: li.phase, arrivee: 0 }, Position(li.pos), Individu)
            })
            .collect();
        let n = v.len() as u64;
        let es: Vec<Entity> = w.spawn_batch(v).collect();
        let mut ind = w.resource_mut::<Indiv>();
        for (e, (_, l)) in es.into_iter().zip(&lots) {
            ind.membres[*l as usize].push(e);
        }
        ind.stats.colons_ind += n;
    }
    let t2 = Instant::now();
    // 3. Retour à l'agrégat étalé : entités arrivées des lieux en retour.
    //    Un lieu redevient agrégé quand il n'a plus aucune entité (celles en
    //    trajet vers lui sont attendues).
    let mut budget = w.resource::<Indiv>().lot_ret;
    let mut k = 0;
    while budget > 0 && k < w.resource::<Indiv>().file_ret.len() {
        let l = w.resource::<Indiv>().file_ret[k];
        let li = l as usize;
        if w.resource::<Indiv>().etat[li] != RETOUR {
            w.resource_mut::<Indiv>().file_ret.remove(k);
            continue;
        }
        let membres = std::mem::take(&mut w.resource_mut::<Indiv>().membres[li]);
        let mut gardes = vec![];
        let mut rendus = vec![];
        for e in membres {
            if budget > 0 && w.get::<Place>(e).unwrap().arrivee <= t {
                rendus.push((w.get::<Fiche>(e).unwrap().0, e));
                budget -= 1;
            } else {
                gardes.push(e);
            }
        }
        rendus.sort_unstable_by_key(|x| x.0.id);
        for &(c, e) in &rendus {
            w.despawn(e);
            w.resource_mut::<Dossiers>().0[li].push(c);
        }
        let mut ind = w.resource_mut::<Indiv>();
        ind.stats.colons_ret += rendus.len() as u64;
        if gardes.is_empty() {
            ind.etat[li] = AGREGE;
            ind.retourne[li] = true;
            ind.stats.lieux_ret += 1;
            ind.file_ret.remove(k);
        } else {
            ind.membres[li] = gardes;
            k += 1;
        }
    }
    let t3 = Instant::now();
    // 4. Chemins (budget d'expansions) et marche.
    marcher(w, t);
    let t4 = Instant::now();
    let mut ind = w.resource_mut::<Indiv>();
    let entites = ind.membres.iter().map(|v| v.len() as u64).sum::<u64>() + ind.marcheurs.len() as u64;
    ind.stats.entites = entites;
    ind.stats.ns_camera += (t1 - t0).as_nanos() as u64;
    ind.stats.ns_ind += (t2 - t1).as_nanos() as u64;
    ind.stats.ns_ret += (t3 - t2).as_nanos() as u64;
    ind.stats.ns_marche += (t4 - t3).as_nanos() as u64;
    ind.stats.max_entites = ind.stats.max_entites.max(ind.stats.entites);
}

const VITESSE: f64 = 1.4;

fn marcher(w: &mut World, t: u32) {
    w.resource_scope(|w, mut ind: Mut<Indiv>| {
        let ind = &mut *ind;
        // Chemins à calculer, dans l'ordre des départs.
        let mut echecs = vec![];
        let mut fait = 0u64;
        while let Some(&(e, id, de, vers, t_dep)) = ind.file_chemins.front() {
            if fait > 0 && fait >= ind.budget_expansions {
                break;
            }
            ind.file_chemins.pop_front();
            ind.stats.attente_max = ind.stats.attente_max.max(t - t_dep);
            let (a, b) = (ind.portes[de as usize].unwrap(), ind.portes[vers as usize].unwrap());
            let avant = ind.astar.expansions;
            let t0 = Instant::now();
            let ch = ind.astar.chemin(ind.passage.as_ref().unwrap(), a, b, ind.max_expansions);
            let ns = t0.elapsed().as_nanos() as u64;
            ind.stats.ns_chemins += ns;
            ind.stats.ns_chemin_max = ind.stats.ns_chemin_max.max(ns);
            let x = ind.astar.expansions - avant;
            fait += x;
            ind.stats.expansions += x;
            ind.stats.chemins += 1;
            match ch {
                Some(chemin) => ind.marcheurs.push(Marcheur { e, id, vers, chemin, i: 0 }),
                None => {
                    // Pas de chemin : trajet abstrait, durée à vol d'oiseau.
                    ind.stats.chemins_echoues += 1;
                    echecs.push((e, id, de, vers));
                }
            }
        }
        // Un pas par tick, à 1,4 m/s.
        let pas = VITESSE / TPS as f64;
        let mut arrives = vec![];
        for (k, m) in ind.marcheurs.iter_mut().enumerate() {
            let mut reste = pas;
            let mut p = w.get::<Position>(m.e).unwrap().0;
            while reste > 0.0 && m.i < m.chemin.len() {
                let q = m.chemin[m.i];
                let d = d2(p, q).sqrt();
                if d <= reste {
                    p = q;
                    reste -= d;
                    m.i += 1;
                } else {
                    p = [p[0] + (q[0] - p[0]) * reste / d, p[1] + (q[1] - p[1]) * reste / d];
                    reste = 0.0;
                }
            }
            w.get_mut::<Position>(m.e).unwrap().0 = p;
            if m.i >= m.chemin.len() {
                arrives.push(k);
            }
        }
        ind.stats.marcheurs_max = ind.stats.marcheurs_max.max(ind.marcheurs.len() as u64);
        // Arrivées : entité si le lieu est individualisé, sinon dossier.
        let mut fin: Vec<Marcheur> = vec![];
        for k in arrives.into_iter().rev() {
            fin.push(ind.marcheurs.swap_remove(k));
        }
        fin.sort_unstable_by_key(|m| m.id);
        for m in fin {
            ind.stats.arrivees_marche += 1;
            w.resource_mut::<Commun>().occ[m.vers as usize] += 1;
            if ind.accueille_individus(m.vers) {
                let mut pl = w.get_mut::<Place>(m.e).unwrap();
                pl.lieu = m.vers;
                pl.arrivee = t;
                ind.membres[m.vers as usize].push(m.e);
            } else {
                let c = w.get::<Fiche>(m.e).unwrap().0;
                w.despawn(m.e);
                w.resource_mut::<Dossiers>().0[m.vers as usize].push(c);
            }
        }
        for (e, id, de, vers) in echecs {
            let c = w.get::<Fiche>(e).unwrap().0;
            let cm = w.resource::<Commun>();
            let g = cm.ctx_global(t);
            let arrivee = t + trajet(&g, de, vers);
            // Le départ a déjà retiré le colon de son lieu (occupation).
            w.resource_mut::<Commun>().occ[de as usize] += 1;
            let d = Depart { c, de, vers, arrivee, e: e.to_bits() };
            let _ = id;
            w.despawn(e);
            w.resource_mut::<Commun>().partir(&d, true);
        }
    });
}

/// Colons « en marche » (ni dans un dossier, ni dans un trajet abstrait) :
/// déjà comptés comme entités.
#[allow(dead_code)]
pub fn marcheurs(c: &Civils) -> usize {
    c.world.resource::<Indiv>().marcheurs.len()
}
