//! Données d'entrée de la génération : tiles de P-07 et attributs
//! complémentaires de la BD TOPO.
//!
//! - Les **paquets** de P-07 (16 × 16 tiles, index en tête) sont lus par
//!   `pread`, comme le ferait le jeu (copié de P-07, `charge.rs`).
//! - Les sections `bati`, `relief`, `franchissable`, `zone` et `reseau` sont
//!   décodées en coordonnées **entières** (1/16 m, locales au centre de la
//!   tile) : la génération part des mêmes entiers sur toutes les machines.
//! - Les attributs absents des tiles de P-07 (altitudes du toit, matériaux)
//!   sont lus directement dans la BD TOPO (SQLite), par identifiant : c'est
//!   ce que mesure la proposition d'ajout au format.
//! - Le **contexte** d'une tile est la tile et ses 8 voisines : relief,
//!   franchissable, zone et routes des bâtiments qui débordent.

use crate::geo::TileId;
use crate::raster::Bitmap;
use crate::relief::{self, Noeuds, SANS};
use crate::tuile::{self, LecteurV, Sec};
use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Énumérations de P-07 (`objets.rs`), dans le même ordre.
pub const USAGES: &[&str] = &[
    "Résidentiel", "Indifférencié", "Annexe", "Commercial et services", "Industriel", "Sportif", "Religieux", "Agricole",
];
pub const NATURES: &[&str] = &[
    "Indifférenciée", "Industriel, agricole ou commercial", "Eglise", "Chapelle", "Monument", "Serre", "Silo",
    "Château", "Tour, donjon", "Tribune", "Moulin à vent", "Fort, blockhaus, casemate", "Arc de triomphe",
    "Arène ou théâtre antique", "Phare",
];
pub mod nat {
    pub const INDIFF: u8 = 0;
    pub const INDUS: u8 = 1;
    pub const EGLISE: u8 = 2;
    pub const CHAPELLE: u8 = 3;
    pub const MONUMENT: u8 = 4;
    pub const SERRE: u8 = 5;
    pub const SILO: u8 = 6;
    pub const CHATEAU: u8 = 7;
    pub const TOUR: u8 = 8;
    pub const TRIBUNE: u8 = 9;
    pub const MOULIN: u8 = 10;
    pub const FORT: u8 = 11;
    pub const ARC: u8 = 12;
    pub const ARENE: u8 = 13;
    pub const PHARE: u8 = 14;
}
pub mod usa {
    pub const RESID: u8 = 0;
    pub const INDIFF: u8 = 1;
    pub const ANNEXE: u8 = 2;
    pub const COMMERCE: u8 = 3;
    pub const INDUS: u8 = 4;
    pub const SPORT: u8 = 5;
    pub const RELIG: u8 = 6;
    pub const AGRI: u8 = 7;
}

pub fn nom_nature(n: u8) -> &'static str {
    NATURES.get(n as usize).copied().unwrap_or("?")
}
pub fn nom_usage(n: u8) -> &'static str {
    USAGES.get(n as usize).copied().unwrap_or("?")
}

/// Attributs de la BD TOPO absents des tiles de P-07.
#[derive(Clone, Copy, Debug, Default)]
pub struct Extra {
    /// Hauteur du toit : altitude maximale − altitude minimale du toit (dm).
    pub toit_dm: Option<u16>,
    /// Altitude minimale du toit − altitude minimale du sol (dm) : repli de
    /// la hauteur quand `hauteur` manque.
    pub gout_dm: Option<u16>,
    /// Matériaux (codes des fichiers fonciers, deux chiffres) ; 255 = absent.
    pub murs: u8,
    pub toiture: u8,
}

/// Un bâtiment possédé par la tile (section `bati` de P-07).
#[derive(Clone, Debug)]
pub struct Bat {
    pub id: u64,
    pub nature: u8,
    pub usage1: u8,
    pub usage2: u8,
    pub leger: bool,
    pub hauteur_dm: Option<u32>,
    pub etages: Option<u32>,
    pub logements: Option<u32>,
    /// Polygones (anneau extérieur puis trous), en 1/16 m, locaux au centre
    /// de la tile propriétaire. Emprise complète, même si elle déborde.
    pub polys: Vec<Vec<Vec<[i32; 2]>>>,
    pub extra: Extra,
}

impl Bat {
    pub fn aire(&self) -> f64 {
        self.polys.iter().map(|p| p.iter().enumerate().map(|(k, a)| if k == 0 { aire_anneau(a).abs() } else { -aire_anneau(a).abs() }).sum::<f64>()).sum()
    }
    pub fn nb_sommets(&self) -> usize {
        self.polys.iter().flatten().map(|a| a.len()).sum()
    }
    pub fn bbox(&self) -> [i32; 4] {
        let mut b = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        for a in self.polys.iter().flatten() {
            for p in a {
                b[0] = b[0].min(p[0]);
                b[1] = b[1].min(p[1]);
                b[2] = b[2].max(p[0]);
                b[3] = b[3].max(p[1]);
            }
        }
        b
    }
}

/// Aire signée d'un anneau en 1/16 m (m²), positive dans le sens direct.
pub fn aire_anneau(a: &[[i32; 2]]) -> f64 {
    let mut s = 0i64;
    let n = a.len();
    for i in 0..n {
        let (p, q) = (a[i], a[(i + 1) % n]);
        s += p[0] as i64 * q[1] as i64 - q[0] as i64 * p[1] as i64;
    }
    s as f64 / 2.0 / 256.0
}

pub const QUANT: f64 = tuile::QUANT;

fn poly(l: &mut LecteurV) -> Vec<Vec<[i32; 2]>> {
    let na = l.u() as usize;
    (0..na)
        .map(|_| {
            let n = l.u() as usize;
            let (mut x, mut y) = (0i64, 0i64);
            let mut v: Vec<[i32; 2]> = Vec::with_capacity(n);
            for _ in 0..n {
                x += l.i();
                y += l.i();
                let p = [x as i32, y as i32];
                // Sommets répétés après quantification : retirés.
                if v.last() != Some(&p) {
                    v.push(p);
                }
            }
            while v.len() > 1 && v[0] == v[v.len() - 1] {
                v.pop();
            }
            v
        })
        .collect()
}

pub fn decoder_bati(b: &[u8]) -> Vec<Bat> {
    let mut l = LecteurV::new(b);
    let n = l.u() as usize;
    let mut out = Vec::with_capacity(n);
    let mut id = 0u64;
    for _ in 0..n {
        id += l.u();
        let nature = l.o();
        let usage1 = l.o();
        let usage2 = l.o();
        let f = l.o();
        let hauteur_dm = (f & 2 != 0).then(|| l.u() as u32);
        let etages = (f & 4 != 0).then(|| l.u() as u32);
        let logements = (f & 8 != 0).then(|| l.u() as u32);
        let np = l.u() as usize;
        let polys = (0..np).map(|_| poly(&mut l)).filter(|p: &Vec<Vec<[i32; 2]>>| !p.is_empty() && p[0].len() >= 3).collect();
        out.push(Bat { id, nature, usage1, usage2, leger: f & 1 != 0, hauteur_dm, etages, logements, polys, extra: Extra { murs: 255, toiture: 255, ..Default::default() } });
    }
    out
}

/// Morceau de route décodé (section `reseau`) : seulement ce qui sert à la
/// porte (au sol, praticable à pied ou en véhicule).
pub struct Route {
    pub nature: u8,
    pub pos_sol: i8,
    pub pts: Vec<[f32; 2]>,
}

pub fn decoder_reseau(b: &[u8]) -> Vec<Route> {
    let mut l = LecteurV::new(b);
    let mut out = Vec::new();
    let n = l.u() as usize;
    for _ in 0..n {
        let _id = l.i();
        let _rang = l.u();
        let nature = l.o();
        let _importance = l.o();
        let pos_sol = l.o() as i8;
        let _largeur = l.u();
        let _ = (l.o(), l.o(), l.o(), l.o());
        let _poids = l.u();
        let _h = l.u();
        let _lr = l.u();
        let pts = ligne(&mut l, pos_sol != 0);
        out.push(Route { nature, pos_sol, pts });
    }
    out
}

fn ligne(l: &mut LecteurV, z: bool) -> Vec<[f32; 2]> {
    let n = l.u() as usize;
    let (mut x, mut y) = (0i64, 0i64);
    (0..n)
        .map(|_| {
            x += l.i();
            y += l.i();
            if z {
                let _ = l.i();
            }
            [(x as f64 / QUANT) as f32, (y as f64 / QUANT) as f32]
        })
        .collect()
}

/// Section `zone` : (cases hors zone, ou None ; cases percées, indices j·500+i).
pub fn decoder_zone(b: &[u8]) -> (Option<Bitmap>, Vec<u32>) {
    let a = b[0] == 1;
    let (ex, p) = if a { (Some(Bitmap::de_octets(500, &b[1..1 + 31_256])), 1 + 31_256) } else { (None, 1) };
    let mut l = LecteurV { b, p };
    let n = l.u() as usize;
    let mut k = 0u32;
    let cases = (0..n)
        .map(|_| {
            k += l.u() as u32;
            k
        })
        .collect();
    (ex, cases)
}

// ------------------------------------------------------------ Paquets

pub struct Paquets {
    pub index: BTreeMap<TileId, (PathBuf, u64, usize)>,
}

pub fn paquets(sortie: &Path) -> Paquets {
    let mut index = BTreeMap::new();
    let mut fichiers: Vec<_> = std::fs::read_dir(sortie.join("paquets")).expect("paquets de P-07").map(|e| e.unwrap().path()).collect();
    fichiers.sort();
    for f in fichiers {
        let b = std::fs::read(&f).unwrap();
        for (t, off, n) in tuile::index_paquet(&b) {
            index.insert(t, (f.clone(), off, n));
        }
    }
    Paquets { index }
}

impl Paquets {
    pub fn lire(&self, t: TileId) -> Option<Vec<u8>> {
        let (f, off, n) = self.index.get(&t)?;
        let fic = std::fs::File::open(f).unwrap();
        let mut b = vec![0u8; *n];
        fic.read_exact_at(&mut b, *off).unwrap();
        Some(b)
    }
}

/// Une tile décodée, pour la génération.
pub struct TileD {
    pub t: TileId,
    pub nb_bat: u32,
    pub bats: Vec<Bat>,
    pub relief: Option<Noeuds>,
    pub franch: Bitmap,
    pub exclus: Option<Bitmap>,
    pub percees: Vec<u32>,
    pub routes: Vec<Route>,
    /// Taille compressée de la section `bati` de P-07 (octets).
    pub octets_bati: usize,
    pub octets_tile: usize,
}

/// Sections à décoder (le relief et le réseau ne servent pas à tous les modes).
#[derive(Clone, Copy)]
pub struct Quoi {
    pub relief: bool,
    pub reseau: bool,
}

pub fn decoder_tile(t: TileId, b: &[u8], quoi: Quoi, extras: &Extras) -> TileD {
    let tl = tuile::lire(b, true);
    let octets_bati = tl.sections.iter().find(|s| s.0 == Sec::Bati).map(|s| s.2.len()).unwrap_or(0);
    let mut bats = tl.section(Sec::Bati).map(|s| decoder_bati(&s)).unwrap_or_default();
    for bt in &mut bats {
        if let Some(e) = extras.get(&bt.id) {
            bt.extra = *e;
        }
    }
    let relief = if quoi.relief { tl.section(Sec::Relief).map(|s| relief::decoder(&s, 2)) } else { None };
    let franch = tl.section(Sec::Franch).map(|s| Bitmap::de_octets(500, &s)).unwrap_or_else(|| {
        // Pas de section (tile de mer) : tout est franchissable.
        let mut b = Bitmap::new(500);
        b.mots.iter_mut().for_each(|m| *m = u64::MAX);
        b
    });
    let (exclus, percees) = tl.section(Sec::Zone).map(|s| decoder_zone(&s)).unwrap_or((None, Vec::new()));
    let routes = if quoi.reseau { tl.section(Sec::Reseau).map(|s| decoder_reseau(&s)).unwrap_or_default() } else { Vec::new() };
    TileD { t, nb_bat: tl.entete.nb_bat, bats, relief, franch, exclus, percees, routes, octets_bati, octets_tile: b.len() }
}

// ------------------------------------------------------------ Attributs BD TOPO

pub type Extras = HashMap<u64, Extra>;

fn code_mat(v: &rusqlite::types::ValueRef) -> u8 {
    match v {
        rusqlite::types::ValueRef::Text(t) => std::str::from_utf8(t).ok().and_then(|s| s.trim().parse::<u8>().ok()).unwrap_or(255),
        rusqlite::types::ValueRef::Integer(i) => (*i).clamp(0, 254) as u8,
        _ => 255,
    }
}

/// Lit les altitudes du toit et les matériaux de tous les bâtiments en
/// service d'un GeoPackage de la BD TOPO.
pub fn lire_extras(gpkg: &Path) -> Extras {
    use rusqlite::{Connection, OpenFlags, types::ValueRef};
    let c = Connection::open_with_flags(gpkg, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).expect("gpkg");
    let mut st = c
        .prepare(
            "select cleabs, altitude_minimale_sol, altitude_minimale_toit, altitude_maximale_toit, materiaux_des_murs, materiaux_de_la_toiture \
             from batiment where etat_de_l_objet = 'En service' or etat_de_l_objet is null",
        )
        .unwrap();
    let mut rows = st.query([]).unwrap();
    let mut out = HashMap::new();
    let f = |v: ValueRef| match v {
        ValueRef::Real(x) => Some(x),
        ValueRef::Integer(i) => Some(i as f64),
        _ => None,
    };
    while let Some(r) = rows.next().unwrap() {
        let cle: String = r.get(0).unwrap();
        let d: String = cle.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
        let id: u64 = d.parse().unwrap_or(0);
        let sol = f(r.get_ref(1).unwrap());
        let tmin = f(r.get_ref(2).unwrap());
        let tmax = f(r.get_ref(3).unwrap());
        let dm = |x: f64| (x * 10.0).round().clamp(0.0, 65_000.0) as u16;
        let toit_dm = match (tmin, tmax) {
            (Some(a), Some(b)) if b >= a => Some(dm(b - a)),
            _ => None,
        };
        let gout_dm = match (sol, tmin) {
            (Some(s), Some(t)) if t > s => Some(dm(t - s)),
            _ => None,
        };
        out.insert(id, Extra { toit_dm, gout_dm, murs: code_mat(&r.get_ref(4).unwrap()), toiture: code_mat(&r.get_ref(5).unwrap()) });
    }
    out
}

// ------------------------------------------------------------ Contexte 3 × 3

/// Une tile et ses 8 voisines (celles qui existent). Coordonnées : 1/16 m ou
/// m, locales au centre de la tile **centrale**.
pub struct Contexte {
    pub centre: TileId,
    /// Indice (dy + 1) · 3 + (dx + 1).
    pub tiles: [Option<Arc<TileD>>; 9],
}

impl Contexte {
    pub fn tile(&self) -> &TileD {
        self.tiles[4].as_ref().unwrap()
    }
    fn voisine_de_case(&self, gi: i32, gj: i32) -> Option<(&TileD, usize, usize)> {
        let (dx, dy) = (gi.div_euclid(500), gj.div_euclid(500));
        if !(-1..=1).contains(&dx) || !(-1..=1).contains(&dy) {
            return None;
        }
        let t = self.tiles[((dy + 1) * 3 + dx + 1) as usize].as_deref()?;
        Some((t, gi.rem_euclid(500) as usize, gj.rem_euclid(500) as usize))
    }
    /// Case de 2 m (gi, gj), comptée depuis le coin sud-ouest de la tile
    /// centrale : franchissable et dans la zone accessible (P-04, P-07).
    pub fn accessible(&self, gi: i32, gj: i32) -> bool {
        match self.voisine_de_case(gi, gj) {
            Some((t, i, j)) => t.franch.get(i, j) && !t.exclus.as_ref().is_some_and(|e| e.get(i, j)),
            None => false,
        }
    }
    /// Altitude (m) au point local (x, y) en m : interpolation bilinéaire des
    /// nœuds de 2 m de la tile qui contient le point.
    pub fn sol(&self, x: f64, y: f64) -> Option<f64> {
        let (gx, gy) = (x + 500.0, y + 500.0);
        let (dx, dy) = ((gx / 1000.0).floor() as i32, (gy / 1000.0).floor() as i32);
        if !(-1..=1).contains(&dx) || !(-1..=1).contains(&dy) {
            return None;
        }
        let t = self.tiles[((dy + 1) * 3 + dx + 1) as usize].as_deref()?;
        let r = t.relief.as_ref()?;
        let (lx, ly) = ((gx - dx as f64 * 1000.0) / 2.0, (gy - dy as f64 * 1000.0) / 2.0);
        let (i, j) = ((lx.floor() as usize).min(499), (ly.floor() as usize).min(499));
        let (fx, fy) = (lx - i as f64, ly - j as f64);
        let v = [r.get(i, j), r.get(i + 1, j), r.get(i, j + 1), r.get(i + 1, j + 1)];
        if v.iter().any(|&c| c == SANS) {
            // Nœud sans donnée (mer, bord) : le plus proche renseigné.
            return v.iter().find(|&&c| c != SANS).map(|&c| c as f64 / 100.0);
        }
        let a = v[0] as f64 * (1.0 - fx) + v[1] as f64 * fx;
        let b = v[2] as f64 * (1.0 - fx) + v[3] as f64 * fx;
        Some((a * (1.0 - fy) + b * fy) / 100.0)
    }
    /// Cases percées (P-07) du contexte, en cases globales (gi, gj).
    pub fn percees(&self) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(t) = &self.tiles[((dy + 1) * 3 + dx + 1) as usize] {
                    for &k in &t.percees {
                        v.push((dx * 500 + (k % 500) as i32, dy * 500 + (k / 500) as i32));
                    }
                }
            }
        }
        v.sort_unstable();
        v
    }
}

/// Cache de tiles décodées, partagé entre les contextes (une tile sert de
/// voisine à 8 autres).
pub struct Cache {
    pub pq: Paquets,
    pub extras: Extras,
    pub quoi: Quoi,
    pub tiles: std::sync::Mutex<HashMap<TileId, Option<Arc<TileD>>>>,
}

impl Cache {
    pub fn new(sortie: &Path, gpkg: Option<&Path>, quoi: Quoi) -> Cache {
        let extras = gpkg.map(lire_extras).unwrap_or_default();
        Cache { pq: paquets(sortie), extras, quoi, tiles: Default::default() }
    }
    pub fn tile(&self, t: TileId) -> Option<Arc<TileD>> {
        if let Some(v) = self.tiles.lock().unwrap().get(&t) {
            return v.clone();
        }
        let v = self.pq.lire(t).map(|b| Arc::new(decoder_tile(t, &b, self.quoi, &self.extras)));
        self.tiles.lock().unwrap().insert(t, v.clone());
        v
    }
    pub fn contexte(&self, t: TileId) -> Option<Contexte> {
        self.tile(t)?;
        let tiles = std::array::from_fn(|k| self.tile(t.voisine(k as i32 % 3 - 1, k as i32 / 3 - 1)));
        Some(Contexte { centre: t, tiles })
    }
    pub fn oublier(&self, garder: impl Fn(TileId) -> bool) {
        self.tiles.lock().unwrap().retain(|t, _| garder(*t));
    }
    pub fn liste(&self) -> Vec<TileId> {
        self.pq.index.keys().copied().collect()
    }
}
