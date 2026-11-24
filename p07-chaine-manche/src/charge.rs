//! Chargement d'une tile en jeu (C-04) et contrôles sur les tiles écrites.
//!
//! Pour chaque tile, sur un seul thread, page cache chaud (les paquets
//! viennent d'être écrits ; vider le cache demande les droits root) :
//! lecture (`pread`), empreinte, décompression et décodage de chaque section,
//! puis ce que le jeu **recalcule** au lieu de le stocker :
//! - le bâti à 1 m (vue, P-01 à P-04), rastérisé depuis les emprises de la
//!   tile et de ses voisines ;
//! - la capacité de chaque bâtiment (P-05) ;
//! - les arbres (B-04), tirés des surfaces boisées par hachage.
//!
//! Contrôles : marches du relief entre tiles voisines (même valeur exigée sur
//! le bord commun), franchissable recalculé contre stocké.

use crate::chaine::type_tile;
use crate::decodage;
use crate::geo::TileId;
use crate::mesure::quantile;
use crate::raster::{self, Bitmap, Fenetre};
use crate::relief;
use crate::tuile::{self, Sec};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn ns(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

#[inline]
pub fn hacher(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Densité d'arbres par case de 2 m selon la classe de végétation
/// (hypothèses de mesure : forêt fermée un arbre pour 25 m², bois 1/33,
/// haie 1/16, verger 1/100, lande 1/200, forêt ouverte 1/80, peupleraie 1/64).
pub const DENSITE: [f64; 11] = [0.0, 0.16, 0.16, 0.16, 0.12, 0.25, 0.02, 0.04, 0.0, 0.0625, 0.05];

/// Arbres d'une tile : (x, y local en dm, classe) ; déterministes.
pub fn arbres(t: TileId, occ: &[u8]) -> Vec<(i16, i16, u8)> {
    let mut v = Vec::new();
    let graine = (t.ix as u64) << 32 ^ (t.iy as u32 as u64);
    for k in 0..250_000usize {
        let c = occ[k] & 0x0F;
        if c == 0 || occ[k] & 0x40 != 0 {
            continue;
        }
        let h = hacher(graine ^ (k as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93));
        if ((h >> 11) as f64 / (1u64 << 53) as f64) < DENSITE[c as usize] {
            let (i, j) = ((k % 500) as i32, (k / 500) as i32);
            let dx = (h & 0x1F) as i32 * 20 / 32;
            let dy = ((h >> 5) & 0x1F) as i32 * 20 / 32;
            v.push(((i * 20 + dx - 5000) as i16, (j * 20 + dy - 5000) as i16, c));
        }
    }
    v
}

pub struct Paquets {
    pub index: BTreeMap<TileId, (PathBuf, u64, usize)>,
}

pub fn paquets(sortie: &Path) -> Paquets {
    let mut index = BTreeMap::new();
    let mut fichiers: Vec<_> = std::fs::read_dir(sortie.join("paquets")).unwrap().map(|e| e.unwrap().path()).collect();
    fichiers.sort();
    for f in fichiers {
        let b = std::fs::read(&f).unwrap();
        for (t, off, n) in tuile::index_paquet(&b) {
            index.insert(t, (f.clone(), off, n));
        }
    }
    Paquets { index }
}

pub fn lire_tile(pq: &Paquets, t: TileId) -> Option<Vec<u8>> {
    let (f, off, n) = pq.index.get(&t)?;
    let fic = std::fs::File::open(f).unwrap();
    let mut b = vec![0u8; *n];
    fic.read_exact_at(&mut b, *off).unwrap();
    Some(b)
}

/// Bâtiments décodés d'une tile, en coordonnées globales.
fn bats_globaux(pq: &Paquets, t: TileId) -> Vec<Vec<Vec<[f64; 3]>>> {
    let Some(b) = lire_tile(pq, t) else { return Vec::new() };
    let tl = tuile::lire(&b, false);
    let Some(s) = tl.section(Sec::Bati) else { return Vec::new() };
    let (cx, cy) = t.centre();
    decodage::bati(&s)
        .into_iter()
        .flat_map(|bt| bt.polys.into_iter())
        .map(|p| p.into_iter().map(|a| a.into_iter().map(|v| [cx + v[0] as f64, cy + v[1] as f64, 0.0]).collect()).collect())
        .collect()
}

pub const ETAPES: &[&str] = &[
    "lecture", "empreinte", "relief", "bati", "franch", "zone", "reseau", "ponts", "occupation", "communes", "bati1m", "capacites", "arbres",
];

pub fn executer(sortie: &Path, etiquette: &str) {
    let pq = paquets(sortie);
    let tiles: Vec<TileId> = pq.index.keys().copied().collect();
    eprintln!("{} tiles dans {} paquets", tiles.len(), pq.index.values().map(|v| &v.0).collect::<std::collections::BTreeSet<_>>().len());

    // Bâtiments de toutes les tiles, pour rastériser le bâti avec les voisines
    // (préparé hors chrono : en jeu, les voisines sont déjà chargées).
    let bats: HashMap<TileId, Vec<Vec<Vec<[f64; 3]>>>> = tiles.par_iter().map(|&t| (t, bats_globaux(&pq, t))).collect();

    // Banc : un thread, tile par tile (comme un chargement étalé).
    let mut f = std::fs::File::create(sortie.join("charge.csv")).unwrap();
    write!(f, "etiquette,ix,iy,type,octets").unwrap();
    for e in ETAPES {
        write!(f, ",ms_{e}").unwrap();
    }
    writeln!(f, ",ms_total,nb_arbres,arbres_z,bati1m_z,bati1m_ms_stock,franch_ecart").unwrap();
    let mut par_type: BTreeMap<&str, Vec<Vec<f64>>> = BTreeMap::new();
    let mut grilles: HashMap<TileId, relief::Noeuds> = HashMap::new();
    for &t in &tiles {
        let mut c = [0f64; 13];
        let t0 = Instant::now();
        let b = lire_tile(&pq, t).unwrap();
        c[0] = ns(t0);
        let t0 = Instant::now();
        let tl = tuile::lire(&b, true);
        c[1] = ns(t0);
        let e = &tl.entete;
        let ty = type_tile(e.flags & 1 != 0, e.flags & 2 != 0, e.habitants, e.part_bati as f64 / 65535.0);
        let t0 = Instant::now();
        if let Some(s) = tl.section(Sec::Relief) {
            let pas = 1000 / (u16::from_le_bytes([s[0], s[1]]) as usize - 1);
            let g = relief::decoder(&s, pas);
            grilles.insert(t, g);
        }
        c[2] = ns(t0);
        let t0 = Instant::now();
        let bl = tl.section(Sec::Bati).map(|s| decodage::bati(&s)).unwrap_or_default();
        c[3] = ns(t0);
        let t0 = Instant::now();
        let franch = tl.section(Sec::Franch).map(|s| Bitmap::de_octets(500, &s));
        c[4] = ns(t0);
        let t0 = Instant::now();
        let _z = tl.section(Sec::Zone).map(|s| decodage::zone(&s));
        c[5] = ns(t0);
        let t0 = Instant::now();
        let _r = tl.section(Sec::Reseau).map(|s| decodage::reseau(&s));
        c[6] = ns(t0);
        let t0 = Instant::now();
        let _p = tl.section(Sec::Ponts).map(|s| decodage::ponts(&s));
        c[7] = ns(t0);
        let t0 = Instant::now();
        let occ = tl.section(Sec::Occup);
        c[8] = ns(t0);
        let t0 = Instant::now();
        let _c = tl.section(Sec::Communes).map(|s| decodage::communes(&s));
        c[9] = ns(t0);
        // Recalculé : bâti à 1 m depuis la tile et ses voisines.
        let t0 = Instant::now();
        let w1 = Fenetre::tile(t.x0(), t.y0(), 1.0);
        let mut bati1 = Bitmap::new(1000);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(ps) = bats.get(&TileId { ix: t.ix + dx, iy: t.iy + dy }) {
                    for p in ps {
                        let b = crate::chaine::bbox_poly_pub(p);
                        if b[2] < t.x0() || b[0] > t.x0() + 1000.0 || b[3] < t.y0() || b[1] > t.y0() + 1000.0 {
                            continue;
                        }
                        raster::polygone(w1, p, |a, bb| bati1.set(a, bb));
                    }
                }
            }
        }
        c[10] = ns(t0);
        let t0 = Instant::now();
        let mut cap = (0u64, 0u64);
        for bt in &bl {
            let (a, b) = bt.capacite();
            cap.0 += a as u64;
            cap.1 += b as u64;
        }
        c[11] = ns(t0);
        let t0 = Instant::now();
        let ar = occ.as_ref().map(|o| arbres(t, o)).unwrap_or_default();
        c[12] = ns(t0);
        let total: f64 = c.iter().sum();
        // Variantes stockées : bâti 1 m et arbres compressés.
        let b1 = bati1.octets();
        let t0 = Instant::now();
        let b1z = zstd::bulk::compress(&b1, 9).unwrap();
        let _ = zstd::bulk::decompress(&b1z, b1.len()).unwrap();
        let b1ms = ns(t0);
        let arz = if ar.is_empty() {
            0
        } else {
            let raw: Vec<u8> = ar.iter().flat_map(|a| [a.0.to_le_bytes()[0], a.0.to_le_bytes()[1], a.1.to_le_bytes()[0], a.1.to_le_bytes()[1], a.2]).collect();
            zstd::bulk::compress(&raw, 9).unwrap().len()
        };
        let ecart = franch.as_ref().map_or(0, |fr| {
            let m2 = raster::majorite_2m(&bati1);
            fr.mots.iter().zip(&m2.mots).map(|(a, b)| (a ^ b).count_ones()).sum::<u32>()
        });
        write!(f, "{etiquette},{},{},{ty},{}", t.ix, t.iy, b.len()).unwrap();
        for v in c {
            write!(f, ",{v:.4}").unwrap();
        }
        writeln!(f, ",{total:.4},{},{arz},{},{b1ms:.4},{ecart}", ar.len(), b1z.len()).unwrap();
        let mut ligne = c.to_vec();
        ligne.push(total);
        par_type.entry(ty).or_default().push(ligne);
    }

    // Résumé par type : médiane, p90, max.
    let mut fr = std::fs::File::create(sortie.join("charge-resume.csv")).unwrap();
    writeln!(fr, "etiquette,type,tiles,etape,mediane_ms,p90_ms,max_ms").unwrap();
    for (ty, lignes) in &par_type {
        for (k, e) in ETAPES.iter().chain(["total"].iter()).enumerate() {
            let mut v: Vec<f64> = lignes.iter().map(|l| l[k]).collect();
            let (a, b, c) = (quantile(&mut v, 0.5), quantile(&mut v, 0.9), quantile(&mut v, 1.0));
            writeln!(fr, "{etiquette},{ty},{},{e},{a:.4},{b:.4},{c:.4}", lignes.len()).unwrap();
        }
    }

    // Contrôle des marches : bords communs identiques.
    let (mut paires, mut ecarts, mut noeuds) = (0u64, 0u64, 0u64);
    for (t, g) in &grilles {
        let m = g.m;
        if let Some(ge) = grilles.get(&TileId { ix: t.ix + 1, iy: t.iy }) {
            paires += 1;
            for j in 0..m {
                noeuds += 1;
                if g.get(m - 1, j) != ge.get(0, j) {
                    ecarts += 1;
                }
            }
        }
        if let Some(gn) = grilles.get(&TileId { ix: t.ix, iy: t.iy + 1 }) {
            paires += 1;
            for i in 0..m {
                noeuds += 1;
                if g.get(i, m - 1) != gn.get(i, 0) {
                    ecarts += 1;
                }
            }
        }
    }
    let mut fk = std::fs::OpenOptions::new().create(true).append(true).open(sortie.join("controles.txt")).unwrap();
    writeln!(fk, "relief_paires_voisines {paires}").unwrap();
    writeln!(fk, "relief_noeuds_de_bord {noeuds}").unwrap();
    writeln!(fk, "relief_marches {ecarts}").unwrap();
    eprintln!("relief : {paires} paires de tiles voisines, {noeuds} nœuds de bord, {ecarts} marches");

    // Niveaux de zstd : recompression de toutes les sections d'un
    // échantillon (une tile sur 10), un thread.
    let mut fz = std::fs::File::create(sortie.join("niveaux.csv")).unwrap();
    writeln!(fz, "etiquette,niveau,tiles,octets,ms_compression,ms_decompression").unwrap();
    let ech: Vec<Vec<(Vec<u8>, usize)>> = tiles
        .iter()
        .step_by(10)
        .map(|&t| {
            let b = lire_tile(&pq, t).unwrap();
            let tl = tuile::lire(&b, false);
            tl.sections.iter().map(|(_, brut, d)| (zstd::bulk::decompress(d, *brut).unwrap(), *brut)).collect()
        })
        .collect();
    for niv in [1, 3, 9, 19] {
        let (mut oct, mut tc, mut td) = (0usize, 0f64, 0f64);
        for secs in &ech {
            for (brut, n) in secs {
                let t0 = Instant::now();
                let z = zstd::bulk::compress(brut, niv).unwrap();
                tc += ns(t0);
                let t0 = Instant::now();
                let _ = zstd::bulk::decompress(&z, *n).unwrap();
                td += ns(t0);
                oct += z.len();
            }
        }
        writeln!(fz, "{etiquette},{niv},{},{oct},{tc:.1},{td:.1}", ech.len()).unwrap();
    }
}
