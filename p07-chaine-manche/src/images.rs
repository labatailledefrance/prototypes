//! Images de vérification, relues depuis les paquets écrits (donc aussi un
//! test du décodage). Une image par tile, cinq panneaux de 500 × 500 px
//! (2 m par pixel), nord en haut, de gauche à droite :
//!
//! 1. **relief** : ombrage (soleil au nord-ouest) et teinte d'altitude ; mer
//!    en bleu ;
//! 2. **bâti** : bâtiments possédés par la tile en noir, ceux des voisines qui
//!    débordent en gris (attribution sans doublon) ;
//! 3. **zone accessible** : vert clair = zone, gris = bâti et murs, rouge =
//!    cases exclues (cours fermées sans passage), magenta = passages percés,
//!    bleu = mer ;
//! 4. **réseau** : routes par importance (rouge 1-2, orange 3-4, jaune 5,
//!    gris 6 et chemins), voies ferrées en noir, ponts en violet ;
//! 5. **eau et végétation** : forêts en vert sombre, bois en vert, haies en
//!    vert vif, landes et vergers en olive, eau douce en bleu clair, estran
//!    en sable, mer en bleu.
//!
//! Plus deux cartes du département : types de tiles et habitants.

use crate::chaine::{OCC_ESTRAN, OCC_EAU, OCC_MER, OCC_SALEE, type_tile};
use crate::charge::{Paquets, lire_tile, paquets};
use crate::decodage::{self, GeomLue};
use crate::geo::TileId;
use crate::raster::{self, Bitmap, Fenetre};
use crate::relief;
use crate::tuile::{self, Sec};
use std::path::Path;

type Img = Vec<[u8; 3]>;

fn ecrire_png(chemin: &Path, w: usize, h: usize, px: &[[u8; 3]]) {
    let f = std::fs::File::create(chemin).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    let mut wr = e.write_header().unwrap();
    let data: Vec<u8> = px.iter().flat_map(|p| p.iter().copied()).collect();
    wr.write_image_data(&data).unwrap();
}

const N: usize = 500;

fn panneau() -> Img {
    vec![[255, 255, 255]; N * N]
}

/// Pixel (i, j) de la grille de 2 m (j vers le nord) → indice image (nord en haut).
#[inline]
fn px(i: usize, j: usize) -> usize {
    (N - 1 - j) * N + i
}

fn relief_png(g: &relief::Noeuds) -> Img {
    let mut im = panneau();
    let r = g.r as f64;
    let z = |x: f64, y: f64| -> Option<f64> {
        let (i, j) = ((x / r).floor() as usize, (y / r).floor() as usize);
        let (i, j) = (i.min(g.m - 2), j.min(g.m - 2));
        let (fx, fy) = (x / r - i as f64, y / r - j as f64);
        let v = [g.get(i, j), g.get(i + 1, j), g.get(i, j + 1), g.get(i + 1, j + 1)];
        if v.iter().any(|v| *v == relief::SANS) {
            return None;
        }
        let v: Vec<f64> = v.iter().map(|v| *v as f64 / 100.0).collect();
        Some((v[0] * (1.0 - fx) + v[1] * fx) * (1.0 - fy) + (v[2] * (1.0 - fx) + v[3] * fx) * fy)
    };
    for j in 0..N {
        for i in 0..N {
            let (x, y) = (i as f64 * 2.0 + 1.0, j as f64 * 2.0 + 1.0);
            let Some(h) = z(x, y) else {
                im[px(i, j)] = [70, 110, 170];
                continue;
            };
            let gx = z((x + 2.0).min(999.0), y).unwrap_or(h) - z((x - 2.0).max(0.0), y).unwrap_or(h);
            let gy = z(x, (y + 2.0).min(999.0)).unwrap_or(h) - z(x, (y - 2.0).max(0.0)).unwrap_or(h);
            // Normale, soleil au nord-ouest à 45°.
            let (nx, ny, nz) = (-gx / 4.0 * 3.0, -gy / 4.0 * 3.0, 1.0);
            let l = (nx * nx + ny * ny + nz * nz).sqrt();
            let s = ((-0.5 * nx + 0.5 * ny + 0.707 * nz) / l).clamp(0.0, 1.0);
            let t = (h / 180.0).clamp(0.0, 1.0);
            let base = [120.0 + 110.0 * t, 160.0 - 30.0 * t, 90.0 + 30.0 * t];
            let k = 0.35 + 0.75 * s;
            im[px(i, j)] = [(base[0] * k).min(255.0) as u8, (base[1] * k).min(255.0) as u8, (base[2] * k).min(255.0) as u8];
        }
    }
    im
}

fn bati_png(pq: &Paquets, t: TileId) -> Img {
    let mut im = panneau();
    let w = Fenetre::tile(t.x0(), t.y0(), 2.0);
    for dy in -1..=1 {
        for dx in -1..=1 {
            let v = TileId { ix: t.ix + dx, iy: t.iy + dy };
            let Some(b) = lire_tile(pq, v) else { continue };
            let tl = tuile::lire(&b, false);
            let Some(s) = tl.section(Sec::Bati) else { continue };
            let (cx, cy) = v.centre();
            let coul = if dx == 0 && dy == 0 { [20, 20, 20] } else { [170, 170, 170] };
            for bt in decodage::bati(&s) {
                for p in bt.polys {
                    let pg: Vec<Vec<[f64; 3]>> = p.into_iter().map(|a| a.into_iter().map(|q| [cx + q[0] as f64, cy + q[1] as f64, 0.0]).collect()).collect();
                    raster::polygone(w, &pg, |i, j| {
                        if dx == 0 && dy == 0 || im[px(i, j)] == [255, 255, 255] {
                            im[px(i, j)] = coul;
                        }
                    });
                }
            }
        }
    }
    im
}

fn zone_png(franch: &Bitmap, occ: &[u8], zone: &(Option<Vec<u8>>, Vec<u32>)) -> Img {
    let mut im = panneau();
    let ex = zone.0.as_ref().map(|b| Bitmap::de_octets(500, b));
    for j in 0..N {
        for i in 0..N {
            let k = j * N + i;
            im[px(i, j)] = if occ[k] & OCC_MER != 0 {
                [70, 110, 170]
            } else if !franch.get(i, j) {
                [110, 110, 110]
            } else if ex.as_ref().is_some_and(|e| e.get(i, j)) {
                [220, 30, 30]
            } else {
                [200, 235, 190]
            };
        }
    }
    for &k in &zone.1 {
        let (i, j) = (k as usize % N, k as usize / N);
        im[px(i, j)] = [230, 0, 230];
    }
    im
}

fn reseau_png(t: TileId, reseau: &[decodage::MorceauLu], ponts: &[decodage::PontLu], occ: &[u8]) -> Img {
    let mut im = panneau();
    for k in 0..N * N {
        if occ[k] & OCC_MER != 0 {
            im[px(k % N, k / N)] = [200, 215, 235];
        }
    }
    let (cx, cy) = t.centre();
    let w = Fenetre::tile(t.x0(), t.y0(), 2.0);
    let glob = |l: &[[f32; 3]]| -> Vec<[f64; 3]> { l.iter().map(|v| [cx + v[0] as f64, cy + v[1] as f64, 0.0]).collect() };
    for p in ponts {
        let c = [130, 40, 180];
        match &p.geom {
            GeomLue::Lignes(ls) => ls.iter().for_each(|l| raster::ligne(w, &glob(l), 5.0, |i, j| im[px(i, j)] = c)),
            GeomLue::Polygones(ps) => {
                for pp in ps {
                    let pg: Vec<Vec<[f64; 3]>> = pp.iter().map(|a| a.iter().map(|q| [cx + q[0] as f64, cy + q[1] as f64, 0.0]).collect()).collect();
                    raster::polygone(w, &pg, |i, j| im[px(i, j)] = c);
                }
            }
            _ => {}
        }
    }
    let mut ordre: Vec<&decodage::MorceauLu> = reseau.iter().collect();
    ordre.sort_by_key(|m| std::cmp::Reverse(if m.ferre { 0 } else { m.importance }));
    for m in ordre {
        let (c, d) = if m.ferre {
            ([0, 0, 0], 1.5)
        } else {
            match m.importance {
                1 | 2 => ([210, 30, 30], 4.0),
                3 | 4 => ([240, 140, 20], 3.0),
                5 => ([220, 200, 0], 2.0),
                _ => ([150, 150, 150], 1.0),
            }
        };
        raster::ligne(w, &glob(&m.pts), d, |i, j| im[px(i, j)] = c);
    }
    im
}

fn occ_png(occ: &[u8]) -> Img {
    let mut im = panneau();
    for k in 0..N * N {
        let o = occ[k];
        let c = if o & OCC_MER != 0 {
            [70, 110, 170]
        } else if o & (OCC_EAU | OCC_SALEE) != 0 {
            [120, 180, 230]
        } else if o & OCC_ESTRAN != 0 {
            [225, 210, 160]
        } else {
            match o & 0x0F {
                1..=3 | 10 => [30, 100, 40],
                4 | 9 => [70, 150, 60],
                5 => [110, 200, 60],
                6..=8 => [160, 160, 80],
                _ => [245, 245, 235],
            }
        };
        im[px(k % N, k / N)] = c;
    }
    im
}

pub fn image_tile(pq: &Paquets, t: TileId, chemin: &Path) -> bool {
    let Some(b) = lire_tile(pq, t) else { return false };
    let tl = tuile::lire(&b, true);
    let occ = tl.section(Sec::Occup).unwrap_or(vec![0; N * N]);
    let rel = tl
        .section(Sec::Relief)
        .map(|s| {
            let pas = 1000 / (u16::from_le_bytes([s[0], s[1]]) as usize - 1);
            relief_png(&relief::decoder(&s, pas))
        })
        .unwrap_or_else(|| vec![[70, 110, 170]; N * N]);
    let franch = tl.section(Sec::Franch).map(|s| Bitmap::de_octets(500, &s)).unwrap_or(Bitmap::new(500));
    let zone = tl.section(Sec::Zone).map(|s| decodage::zone(&s)).unwrap_or((None, vec![]));
    let reseau = tl.section(Sec::Reseau).map(|s| decodage::reseau(&s)).unwrap_or_default();
    let ponts = tl.section(Sec::Ponts).map(|s| decodage::ponts(&s)).unwrap_or_default();
    let panneaux = [rel, bati_png(pq, t), zone_png(&franch, &occ, &zone), reseau_png(t, &reseau, &ponts, &occ), occ_png(&occ)];
    let (w, sep) = (N * 5 + 4 * 6, 6);
    let mut im = vec![[255u8, 255, 255]; w * N];
    for (k, p) in panneaux.iter().enumerate() {
        for j in 0..N {
            for i in 0..N {
                im[j * w + k * (N + sep) + i] = p[j * N + i];
            }
        }
    }
    ecrire_png(chemin, w, N, &im);
    true
}

pub fn executer(sortie: &Path, tiles: &[(TileId, String)]) {
    let pq = paquets(sortie);
    let dossier = sortie.join("images");
    std::fs::create_dir_all(&dossier).unwrap();
    for (t, nom) in tiles {
        let n = if nom.is_empty() { t.nom() } else { format!("{nom}-{}", t.nom()) };
        let c = dossier.join(format!("{n}.png"));
        if image_tile(&pq, *t, &c) {
            println!("{}", c.display());
        } else {
            eprintln!("tile absente : {}", t.nom());
        }
    }
    // Cartes du département : 3 px par tile.
    let ts: Vec<TileId> = pq.index.keys().copied().collect();
    if ts.is_empty() {
        return;
    }
    let (x0, x1) = (ts.iter().map(|t| t.ix).min().unwrap(), ts.iter().map(|t| t.ix).max().unwrap());
    let (y0, y1) = (ts.iter().map(|t| t.iy).min().unwrap(), ts.iter().map(|t| t.iy).max().unwrap());
    let k = 3usize;
    let (w, h) = (((x1 - x0 + 1) as usize) * k, ((y1 - y0 + 1) as usize) * k);
    let mut types = vec![[255u8, 255, 255]; w * h];
    let mut hab = vec![[255u8, 255, 255]; w * h];
    for t in &ts {
        let b = lire_tile(&pq, *t).unwrap();
        let e = tuile::lire(&b, false).entete;
        let ty = type_tile(e.flags & 1 != 0, e.flags & 2 != 0, e.habitants, e.part_bati as f64 / 65535.0);
        let c = match ty {
            "mer" => [70, 110, 170],
            "cote" => [150, 190, 220],
            "vide" => [225, 230, 200],
            "rural" => [180, 210, 140],
            "bourg" => [240, 200, 80],
            "urbain" => [230, 110, 40],
            _ => [170, 20, 20],
        };
        let v = if e.flags & 1 == 0 { 0.0 } else { ((e.habitants as f64 + 1.0).ln() / (20000f64).ln()).clamp(0.0, 1.0) };
        let ch = if e.flags & 1 == 0 { [70, 110, 170] } else { [(255.0 - 200.0 * v) as u8, (255.0 - 230.0 * v) as u8, (255.0 - 120.0 * v) as u8] };
        let (i0, j0) = (((t.ix - x0) as usize) * k, ((y1 - t.iy) as usize) * k);
        for dj in 0..k {
            for di in 0..k {
                types[(j0 + dj) * w + i0 + di] = c;
                hab[(j0 + dj) * w + i0 + di] = ch;
            }
        }
    }
    ecrire_png(&dossier.join("carte-types.png"), w, h, &types);
    ecrire_png(&dossier.join("carte-habitants.png"), w, h, &hab);
    println!("{}", dossier.join("carte-types.png").display());
}
