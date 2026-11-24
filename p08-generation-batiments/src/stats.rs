//! Modes de mesure sans rendu : génération et contrôles par tile, débit,
//! déterminisme, étalement, destruction, repli de la hauteur, format,
//! monuments, images des portes.

use crate::batiment::{self, Etat, Forme, Lod, Params, SourceH, Sortie};
use crate::donnees::{self, Bat, Cache, Quoi, nat};
use crate::generation::{self, Gen, Ordre, ms};
use crate::geo::TileId;
use crate::geom::{self, P2};
use crate::porte::{self, Methode};
use crate::raster::{self, Fenetre};
use crate::stockage;
use crate::tuile::{self, Ecrivain, Sec};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

pub fn quantile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// Types de tiles de P-07 (tiles.csv) et nombre de bâtiments.
pub fn types(sortie: &Path) -> BTreeMap<TileId, (String, u32)> {
    let s = std::fs::read_to_string(sortie.join("tiles.csv")).expect("tiles.csv de P-07");
    let mut l = s.lines();
    let tete: Vec<&str> = l.next().unwrap().split(',').collect();
    let c = |n: &str| tete.iter().position(|x| *x == n).unwrap();
    let (ci, cj, ct, cb) = (c("ix"), c("iy"), c("type"), c("nb_bat"));
    l.map(|x| {
        let v: Vec<&str> = x.split(',').collect();
        (TileId { ix: v[ci].parse().unwrap(), iy: v[cj].parse().unwrap() }, (v[ct].to_string(), v[cb].parse().unwrap()))
    })
    .collect()
}

/// Tiles à traiter : la liste donnée, sinon toutes celles qui ont des
/// bâtiments, rangées par (iy, ix) pour pouvoir oublier les rangées passées.
pub fn a_traiter(ty: &BTreeMap<TileId, (String, u32)>, liste: &[TileId]) -> Vec<TileId> {
    let mut v: Vec<TileId> = if liste.is_empty() { ty.iter().filter(|(_, (_, n))| *n > 0).map(|(t, _)| *t).collect() } else { liste.to_vec() };
    v.sort_by_key(|t| (t.iy, t.ix));
    v
}

fn oublier_avant(c: &Cache, iy: i32) {
    c.oublier(|t| t.iy >= iy - 1);
}

pub fn nom_type(t: &str) -> &str {
    t
}

// ------------------------------------------------------------ Contrôle raster

/// Cases de 1 m de l'emprise (règle du centre) contre cases couvertes par
/// les triangles du toit : (bâti sans volume, volume hors emprise, total).
pub fn controle_raster(b: &Bat, s: &Sortie) -> (u32, u32, u32) {
    let bb = b.bbox();
    let x0 = (bb[0] as f64 / 16.0).floor() - 1.0;
    let y0 = (bb[1] as f64 / 16.0).floor() - 1.0;
    let nx = ((bb[2] as f64 / 16.0).ceil() - x0 + 2.0) as usize;
    let ny = ((bb[3] as f64 / 16.0).ceil() - y0 + 2.0) as usize;
    let w = Fenetre { x0, y0, res: 1.0, nx, ny };
    let mut f = vec![false; nx * ny];
    for poly in &b.polys {
        let an: Vec<Vec<P2>> = poly.iter().map(|a| a.iter().map(|q| geom::m(*q)).collect()).collect();
        raster::polygone(w, &an, |i, j| f[j * nx + i] = true);
    }
    let mut r = vec![false; nx * ny];
    let m = &s.mesh;
    for t in m.idx.chunks_exact(3) {
        if t.iter().all(|&i| m.nor[i as usize][1] > 0.01) {
            let tri: Vec<P2> = t.iter().map(|&i| [m.pos[i as usize][0] as f64, -(m.pos[i as usize][2] as f64)]).collect();
            raster::polygone(w, &[tri], |i, j| r[j * nx + i] = true);
        }
    }
    let (mut manque, mut hors, mut tot) = (0, 0, 0);
    for k in 0..nx * ny {
        tot += f[k] as u32;
        manque += (f[k] && !r[k]) as u32;
        hors += (!f[k] && r[k]) as u32;
    }
    (manque, hors, tot)
}

// ------------------------------------------------------------ Catégories pathologiques

pub const CATS: &[&str] = &["toutes", "tres_petite", "tres_grande", "concave", "a_cours", "multipolygone", "a_cheval", "sans_hauteur", "beaucoup_de_sommets"];

pub fn categories(b: &Bat) -> u16 {
    let mut c = 1u16;
    let a = b.aire();
    if a < 10.0 {
        c |= 1 << 1;
    }
    if a > 5000.0 {
        c |= 1 << 2;
    }
    let pts: Vec<[i32; 2]> = b.polys.iter().map(|p| &p[0]).flatten().copied().collect();
    let h = geom::enveloppe(&pts);
    let ah = geom::aire2_i(&h).abs() as f64 / 512.0;
    if ah > 0.0 && a / ah < 0.6 {
        c |= 1 << 3;
    }
    if b.polys.iter().any(|p| p.len() > 1) {
        c |= 1 << 4;
    }
    if b.polys.len() > 1 {
        c |= 1 << 5;
    }
    let bb = b.bbox();
    if bb[0] < -8000 || bb[1] < -8000 || bb[2] > 8000 || bb[3] > 8000 {
        c |= 1 << 6;
    }
    if b.hauteur_dm.is_none() {
        c |= 1 << 7;
    }
    if b.nb_sommets() > 200 {
        c |= 1 << 8;
    }
    c
}

/// Enregistrement par bâtiment, pour les résumés.
#[derive(Clone)]
pub struct Rec {
    pub ty: u8,
    pub cats: u16,
    pub us: f32,
    pub tri: u32,
    pub err_aire: f32,
    pub manque: u32,
    pub hors: u32,
    pub cases: u32,
    pub echec: bool,
    pub methode: u8,
    pub rue: f32,
    pub p05_mur: f32,
    pub p05_rue: f32,
    pub ecart: f32,
    pub forme: Forme,
    pub dz_donnee: bool,
    pub source: SourceH,
    pub porche: bool,
    pub denivele: f32,
}

pub const TYPES: &[&str] = &["vide", "cote", "rural", "bourg", "urbain", "dense", "mer"];
fn ty_idx(s: &str) -> u8 {
    TYPES.iter().position(|x| *x == s).unwrap_or(0) as u8
}

// ------------------------------------------------------------ Mode stats

pub struct Opt<'a> {
    pub sortie: &'a Path,
    pub gpkg: Option<&'a Path>,
    pub tiles: Vec<TileId>,
    pub res: &'a Path,
    pub etiquette: &'a str,
    pub raster: bool,
    pub zstd19_un_sur: usize,
}

pub fn stats(o: &Opt) {
    let p = Params::default();
    let ty = types(o.sortie);
    let t0 = Instant::now();
    let cache = Cache::new(o.sortie, o.gpkg, Quoi { relief: true, reseau: true });
    eprintln!("extras BD TOPO : {} bâtiments en {:.1} s", cache.extras.len(), t0.elapsed().as_secs_f64());
    let liste = a_traiter(&ty, &o.tiles);
    std::fs::create_dir_all(o.res).unwrap();
    let mut f = std::fs::File::create(o.res.join(format!("tiles-{}.csv", o.etiquette))).unwrap();
    writeln!(
        f,
        "etiquette,tile,type,nb_bat,sommets,ms_contexte,ms_repli,ms_grille,ms_portes,ms_porches,ms_gen,us_bat_med,us_bat_max,ms_fusion,\
         tri_iso,tri_loin,tri_carte,gpu_iso,gpu_iso_c,gpu_loin,gpu_loin_c,gpu_carte,compact_iso,z9_iso,z19_iso,compact_loin,z9_loin,\
         bati_p07,tile_p07,portes_mur1,portes_mur2,portes_mur3,portes_centroide,portes_aucune,bats_porche,cases_porche,percees_tile,\
         echecs_toit,err_aire_max,raster_manque,raster_hors,raster_total,fenetres,toits_plat,toits_2p,toits_4p,toits_mansarde,empreinte"
    )
    .unwrap();
    let mut recs: Vec<Rec> = Vec::new();
    let mut ids: Vec<u64> = Vec::new();
    let mut nb_entetes = 0u64;
    let mut iy_cour = i32::MIN;
    let mut remesures: Vec<(u64, f64, f64)> = Vec::new();
    let mut lents: Vec<(f64, u64, TileId, f64, usize, usize, usize)> = Vec::new();
    for (n, &t) in liste.iter().enumerate() {
        if t.iy != iy_cour {
            oublier_avant(&cache, t.iy);
            iy_cour = t.iy;
        }
        let tc = Instant::now();
        let Some(ctx) = cache.contexte(t) else { continue };
        let ms_ctx = ms(tc);
        let tile = ctx.tile();
        nb_entetes += tile.nb_bat as u64;
        if tile.bats.is_empty() {
            continue;
        }
        ids.extend(tile.bats.iter().map(|b| b.id));
        let tyn = ty.get(&t).map(|x| x.0.clone()).unwrap_or_default();
        let prep = generation::preparer(&ctx, &p, true);
        let etats = HashMap::new();
        let g = Gen { ctx: &ctx, prep: &prep, p: &p, lod: Lod::Iso, etats: &etats };
        let tg = Instant::now();
        let (s_iso, mut us) = generation::tout(&g, false, Ordre::Direct);
        let ms_gen = ms(tg);
        // Mesures suspectes (> 1 ms) : répétées trois fois, on garde la plus courte.
        for k in 0..us.len() {
            if us[k] > 1000.0 {
                let avant = us[k];
                for _ in 0..3 {
                    let t1 = Instant::now();
                    std::hint::black_box(g.bat(k));
                    us[k] = us[k].min(t1.elapsed().as_secs_f64() * 1e6);
                }
                remesures.push((tile.bats[k].id, avant, us[k]));
            }
        }
        for (k, b) in tile.bats.iter().enumerate() {
            lents.push((us[k], b.id, t, b.aire(), b.nb_sommets(), b.polys.len(), prep.porches[k].len()));
        }
        if lents.len() > 4000 {
            lents.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
            lents.truncate(50);
        }
        let tf = Instant::now();
        let (m_iso, _) = generation::fusionner(&s_iso);
        let ms_fus = ms(tf);
        let gl = Gen { lod: Lod::Loin, ..g };
        let (s_loin, _) = generation::tout(&gl, false, Ordre::Direct);
        let (m_loin, _) = generation::fusionner(&s_loin);
        let gc = Gen { lod: Lod::Carte, ..g };
        let (s_carte, _) = generation::tout(&gc, false, Ordre::Direct);
        let (m_carte, _) = generation::fusionner(&s_carte);
        let c_iso = stockage::compact(&m_iso);
        let c_loin = stockage::compact(&m_loin);
        let z9 = zstd::bulk::compress(&c_iso, 9).unwrap().len();
        let z9l = zstd::bulk::compress(&c_loin, 9).unwrap().len();
        let z19 = if o.zstd19_un_sur > 0 && n % o.zstd19_un_sur == 0 { zstd::bulk::compress(&c_iso, 19).unwrap().len() as i64 } else { -1 };
        let mut usv = us.clone();
        let usmed = quantile(&mut usv, 0.5);
        let usmax = usv.last().copied().unwrap_or(0.0);
        let mut portes = [0u32; 5];
        let (mut bats_porche, mut cases_porche, mut echecs, mut err_max, mut fen) = (0, 0, 0, 0f64, 0u32);
        let (mut rm, mut rh, mut rt) = (0u32, 0u32, 0u32);
        let mut formes = [0u32; 4];
        for (k, b) in tile.bats.iter().enumerate() {
            let s = &s_iso[k];
            let m = &prep.modeles[k];
            let ch = prep.portes[k].unwrap();
            let mi = match ch.methode {
                Methode::Mur(r) => r - 1,
                Methode::Centroide => 3,
                Methode::Aucune => 4,
            };
            portes[mi as usize] += 1;
            bats_porche += s.stats.porche as u32;
            cases_porche += s.stats.cases_porche;
            echecs += s.stats.echecs_toit;
            fen += s.stats.fenetres;
            let err = ((s.stats.aire_toit - s.stats.aire_emprise) / s.stats.aire_emprise.max(1e-9)).abs();
            err_max = err_max.max(err);
            if err > 1e-3 && std::env::var("P08_DEBUG").is_ok() {
                let an: Vec<String> = b.polys.iter().map(|p| p.iter().map(|a| format!("{}:{:.1}", a.len(), geom::aire2_i(a) as f64 / 512.0)).collect::<Vec<_>>().join("/")).collect();
                eprintln!("ecart {} id {} forme {:?} toit {:.1} emprise {:.1} anneaux {:?}", t.nom(), b.id, m.forme, s.stats.aire_toit, s.stats.aire_emprise, an);
            }
            formes[m.forme as usize] += 1;
            let (a, h, tt) = if o.raster { controle_raster(b, s) } else { (0, 0, 0) };
            (rm, rh, rt) = (rm + a, rh + h, rt + tt);
            recs.push(Rec {
                ty: ty_idx(&tyn),
                cats: categories(b),
                us: us[k] as f32,
                tri: s.mesh.triangles() as u32,
                err_aire: err as f32,
                manque: a,
                hors: h,
                cases: tt,
                echec: s.stats.echecs_toit > 0,
                methode: mi,
                rue: ch.rue_m.unwrap_or(f64::NAN) as f32,
                p05_mur: ch.p05_mur_m as f32,
                p05_rue: ch.p05_rue_m.unwrap_or(f64::NAN) as f32,
                ecart: ch.ecart_m as f32,
                forme: m.forme,
                dz_donnee: m.dz_donnee,
                source: m.source_h,
                porche: s.stats.porche,
                denivele: s.stats.denivele as f32,
            });
        }
        let em = generation::empreinte(&s_iso);
        writeln!(
            f,
            "{},{},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.2},{:.1},{:.3},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.2e},{},{},{},{},{},{},{},{},{}",
            o.etiquette,
            t.nom(),
            tyn,
            tile.bats.len(),
            tile.bats.iter().map(|b| b.nb_sommets()).sum::<usize>(),
            ms_ctx,
            prep.ms_repli,
            prep.ms_grille,
            prep.ms_portes,
            prep.ms_porches,
            ms_gen,
            usmed,
            usmax,
            ms_fus,
            m_iso.triangles(),
            m_loin.triangles(),
            m_carte.triangles(),
            m_iso.octets_gpu(),
            m_iso.octets_gpu_compresses(),
            m_loin.octets_gpu(),
            m_loin.octets_gpu_compresses(),
            m_carte.octets_gpu(),
            c_iso.len(),
            z9,
            z19,
            c_loin.len(),
            z9l,
            tile.octets_bati,
            tile.octets_tile,
            portes[0],
            portes[1],
            portes[2],
            portes[3],
            portes[4],
            bats_porche,
            cases_porche,
            tile.percees.len(),
            echecs,
            err_max,
            rm,
            rh,
            rt,
            fen,
            formes[0],
            formes[1],
            formes[2],
            formes[3],
            &em.to_hex()[..16]
        )
        .unwrap();
        if n % 200 == 0 {
            eprintln!("{} / {} tiles", n, liste.len());
        }
    }
    // Contrôle : aucun bâtiment généré deux fois.
    let n_ids = ids.len();
    ids.sort_unstable();
    ids.dedup();
    let mut c = std::fs::File::create(o.res.join(format!("controles-{}.txt", o.etiquette))).unwrap();
    writeln!(c, "batiments_generes {n_ids}\nidentifiants_distincts {}\nbatiments_dans_les_entetes {nb_entetes}", ids.len()).unwrap();
    let (m, h, t): (u64, u64, u64) = recs.iter().fold((0, 0, 0), |a, r| (a.0 + r.manque as u64, a.1 + r.hors as u64, a.2 + r.cases as u64));
    writeln!(c, "raster_1m_cases_emprise {t}\nraster_1m_bati_sans_volume {m}\nraster_1m_volume_hors_emprise {h}").unwrap();
    writeln!(c, "batiments_avec_ecart_raster {}", recs.iter().filter(|r| r.manque + r.hors > 0).count()).unwrap();
    writeln!(c, "batiments_echec_toit {}", recs.iter().filter(|r| r.echec).count()).unwrap();
    writeln!(c, "batiments_ecart_aire_sup_1e-3 {}", recs.iter().filter(|r| r.err_aire > 1e-3).count()).unwrap();
    for d in [1.0f32, 3.0, 10.0] {
        writeln!(c, "batiments_denivele_sup_{d}m {}", recs.iter().filter(|r| r.denivele > d).count()).unwrap();
    }
    writeln!(c, "mesures_remesurees {}", remesures.len()).unwrap();
    for (id, a, b) in &remesures {
        writeln!(c, "  remesure batiment {id} : {a:.0} µs puis {b:.0} µs").unwrap();
    }
    lents.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let mut fl = std::fs::File::create(o.res.join(format!("lents-{}.csv", o.etiquette))).unwrap();
    writeln!(fl, "etiquette,us,batiment,tile,aire_m2,sommets,polygones,cases_porche").unwrap();
    for x in lents.iter().take(20) {
        writeln!(fl, "{},{:.0},{},{},{:.0},{},{},{}", o.etiquette, x.0, x.1, x.2.nom(), x.3, x.4, x.5, x.6).unwrap();
    }
    resumes(o, &recs);
}

fn resumes(o: &Opt, recs: &[Rec]) {
    // Catégories pathologiques.
    let mut f = std::fs::File::create(o.res.join(format!("pathologiques-{}.csv", o.etiquette))).unwrap();
    writeln!(f, "etiquette,categorie,batiments,us_med,us_p99,us_max,tri_med,tri_max,echecs_toit,ecart_aire_max,raster_ecart_bats,raster_manque,raster_hors,raster_total").unwrap();
    for (k, nom) in CATS.iter().enumerate() {
        let v: Vec<&Rec> = recs.iter().filter(|r| r.cats >> k & 1 == 1).collect();
        if v.is_empty() {
            continue;
        }
        let mut us: Vec<f64> = v.iter().map(|r| r.us as f64).collect();
        let mut tr: Vec<f64> = v.iter().map(|r| r.tri as f64).collect();
        writeln!(
            f,
            "{},{},{},{:.2},{:.1},{:.1},{},{},{},{:.2e},{},{},{},{}",
            o.etiquette,
            nom,
            v.len(),
            quantile(&mut us, 0.5),
            quantile(&mut us, 0.99),
            us.last().unwrap(),
            quantile(&mut tr, 0.5),
            tr.last().unwrap(),
            v.iter().filter(|r| r.echec).count(),
            v.iter().map(|r| r.err_aire).fold(0f32, f32::max),
            v.iter().filter(|r| r.manque + r.hors > 0).count(),
            v.iter().map(|r| r.manque as u64).sum::<u64>(),
            v.iter().map(|r| r.hors as u64).sum::<u64>(),
            v.iter().map(|r| r.cases as u64).sum::<u64>()
        )
        .unwrap();
    }
    // Portes par type de tile.
    let mut f = std::fs::File::create(o.res.join(format!("portes-{}.csv", o.etiquette))).unwrap();
    writeln!(
        f,
        "etiquette,type,batiments,mur_1m,mur_2m,mur_3m,centroide,aucune,rue_med_m,rue_p90_m,sans_rue,p05_rue_med_m,p05_rue_p90_m,p05_loin_du_mur,p05_mur_med_m,ecart_med_m,ecart_p90_m,p08_plus_pres_rue,egal,p05_plus_pres_rue"
    )
    .unwrap();
    for ty in 0..TYPES.len() as u8 + 1 {
        let v: Vec<&Rec> = recs.iter().filter(|r| ty == TYPES.len() as u8 || r.ty == ty).collect();
        if v.is_empty() {
            continue;
        }
        let nom = if ty == TYPES.len() as u8 { "toutes" } else { TYPES[ty as usize] };
        let cnt = |m: u8| v.iter().filter(|r| r.methode == m).count();
        let mut rue: Vec<f64> = v.iter().filter(|r| !r.rue.is_nan()).map(|r| r.rue as f64).collect();
        let mut rue5: Vec<f64> = v.iter().filter(|r| !r.p05_rue.is_nan()).map(|r| r.p05_rue as f64).collect();
        let mut mur5: Vec<f64> = v.iter().filter(|r| !r.p05_mur.is_nan()).map(|r| r.p05_mur as f64).collect();
        let mut ec: Vec<f64> = v.iter().filter(|r| !r.ecart.is_nan() && r.methode < 3).map(|r| r.ecart as f64).collect();
        let both: Vec<&&Rec> = v.iter().filter(|r| !r.rue.is_nan() && !r.p05_rue.is_nan()).collect();
        writeln!(
            f,
            "{},{},{},{},{},{},{},{},{:.1},{:.1},{},{:.1},{:.1},{},{:.2},{:.1},{:.1},{},{},{}",
            o.etiquette,
            nom,
            v.len(),
            cnt(0),
            cnt(1),
            cnt(2),
            cnt(3),
            cnt(4),
            quantile(&mut rue, 0.5),
            quantile(&mut rue, 0.9),
            v.iter().filter(|r| r.rue.is_nan()).count(),
            quantile(&mut rue5, 0.5),
            quantile(&mut rue5, 0.9),
            v.iter().filter(|r| r.p05_mur > 3.0).count(),
            quantile(&mut mur5, 0.5),
            quantile(&mut ec, 0.5),
            quantile(&mut ec, 0.9),
            both.iter().filter(|r| r.rue < r.p05_rue).count(),
            both.iter().filter(|r| r.rue == r.p05_rue).count(),
            both.iter().filter(|r| r.rue > r.p05_rue).count()
        )
        .unwrap();
    }
    // Modèle : formes de toit et sources de la hauteur.
    let mut f = std::fs::File::create(o.res.join(format!("modele-{}.csv", o.etiquette))).unwrap();
    writeln!(f, "etiquette,type,batiments,plat,deux_pans,quatre_pans,mansarde,toit_donnee,h_donnee,h_etages,h_altitudes_toit,h_mediane_tile,h_constante,avec_porche").unwrap();
    for ty in 0..TYPES.len() as u8 + 1 {
        let v: Vec<&Rec> = recs.iter().filter(|r| ty == TYPES.len() as u8 || r.ty == ty).collect();
        if v.is_empty() {
            continue;
        }
        let nom = if ty == TYPES.len() as u8 { "toutes" } else { TYPES[ty as usize] };
        let fo = |x: Forme| v.iter().filter(|r| r.forme == x).count();
        let so = |x: SourceH| v.iter().filter(|r| r.source == x).count();
        writeln!(
            f,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            o.etiquette,
            nom,
            v.len(),
            fo(Forme::Plat),
            fo(Forme::DeuxPans),
            fo(Forme::QuatrePans),
            fo(Forme::Mansarde),
            v.iter().filter(|r| r.dz_donnee).count(),
            so(SourceH::Donnee),
            so(SourceH::Etages),
            so(SourceH::AltitudesToit),
            so(SourceH::MedianeTile),
            so(SourceH::Constante),
            v.iter().filter(|r| r.porche).count()
        )
        .unwrap();
    }
}

// ------------------------------------------------------------ Débit

/// Bâtiments par seconde : un thread, puis N threads (parallèle sur les
/// tiles, chaque tile séquentielle), puis une tile seule parallèle sur ses
/// bâtiments. Les contextes sont préparés avant (seule la génération est
/// chronométrée).
pub fn debit(o: &Opt, threads: &[usize]) {
    let p = Params::default();
    let ty = types(o.sortie);
    let cache = Cache::new(o.sortie, o.gpkg, Quoi { relief: true, reseau: true });
    let liste = a_traiter(&ty, &o.tiles);
    let mut f = std::fs::File::create(o.res.join(format!("debit-{}.csv", o.etiquette))).unwrap();
    writeln!(f, "etiquette,threads,mode,tiles,batiments,s,batiments_par_s").unwrap();
    // Préparation (contextes, portes) une fois, par rangées.
    let mut preps = Vec::new();
    let mut iy = i32::MIN;
    for &t in &liste {
        if t.iy != iy {
            oublier_avant(&cache, t.iy);
            iy = t.iy;
        }
        if let Some(ctx) = cache.contexte(t) {
            if ctx.tile().bats.is_empty() {
                continue;
            }
            let pr = generation::preparer(&ctx, &p, true);
            preps.push((ctx, pr));
        }
        // Mémoire : on garde seulement 400 tiles préparées au plus.
        if preps.len() >= 400 {
            break;
        }
    }
    let nb: usize = preps.iter().map(|x| x.0.tile().bats.len()).sum();
    let etats = HashMap::new();
    for &th in threads {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(th).build().unwrap();
        for rep in 0..2 {
            let t = Instant::now();
            pool.install(|| {
                preps.par_iter().with_min_len(1).for_each(|(ctx, pr)| {
                    let g = Gen { ctx, prep: pr, p: &p, lod: Lod::Iso, etats: &etats };
                    let (s, _) = generation::tout(&g, false, Ordre::Direct);
                    let _ = generation::fusionner(&s);
                });
            });
            let s = t.elapsed().as_secs_f64();
            writeln!(f, "{},{},tiles_en_parallele_r{},{},{},{:.3},{:.0}", o.etiquette, th, rep, preps.len(), nb, s, nb as f64 / s).unwrap();
        }
        // Tile la plus chargée, parallèle sur ses bâtiments.
        let (ctx, pr) = preps.iter().max_by_key(|x| x.0.tile().bats.iter().map(|b| b.nb_sommets()).sum::<usize>()).unwrap();
        let g = Gen { ctx, prep: pr, p: &p, lod: Lod::Iso, etats: &etats };
        let mut v = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            pool.install(|| {
                let (s, _) = generation::tout(&g, th > 1, Ordre::Direct);
                let _ = generation::fusionner(&s);
            });
            v.push(t.elapsed().as_secs_f64());
        }
        let s = quantile(&mut v, 0.5);
        writeln!(f, "{},{},tile_seule_{},1,{},{:.4},{:.0}", o.etiquette, th, ctx.centre.nom(), g.n(), s, g.n() as f64 / s).unwrap();
    }
}

// ------------------------------------------------------------ Déterminisme

pub fn determinisme(o: &Opt) {
    let p = Params::default();
    let ty = types(o.sortie);
    let cache = Cache::new(o.sortie, o.gpkg, Quoi { relief: true, reseau: true });
    let liste = a_traiter(&ty, &o.tiles);
    let variantes: Vec<(usize, Ordre)> = vec![(1, Ordre::Direct), (16, Ordre::Direct), (16, Ordre::Inverse), (16, Ordre::Melange(7)), (4, Ordre::Melange(42))];
    let mut glob: Vec<blake3::Hasher> = variantes.iter().map(|_| blake3::Hasher::new()).collect();
    let mut diff = vec![0usize; variantes.len()];
    let mut iy = i32::MIN;
    let mut n = 0;
    let etats = HashMap::new();
    let pools: Vec<rayon::ThreadPool> = variantes.iter().map(|v| rayon::ThreadPoolBuilder::new().num_threads(v.0).build().unwrap()).collect();
    for &t in &liste {
        if t.iy != iy {
            oublier_avant(&cache, t.iy);
            iy = t.iy;
        }
        let Some(ctx) = cache.contexte(t) else { continue };
        if ctx.tile().bats.is_empty() {
            continue;
        }
        n += 1;
        let pr = generation::preparer(&ctx, &p, true);
        let g = Gen { ctx: &ctx, prep: &pr, p: &p, lod: Lod::Iso, etats: &etats };
        let mut ref_h = None;
        for (k, (th, ord)) in variantes.iter().enumerate() {
            let s = pools[k].install(|| generation::tout(&g, *th > 1, *ord).0);
            let (m, _) = generation::fusionner(&s);
            let h = blake3::hash(&m.octets());
            glob[k].update(&t.ix.to_le_bytes());
            glob[k].update(&t.iy.to_le_bytes());
            glob[k].update(h.as_bytes());
            if *ref_h.get_or_insert(h) != h {
                diff[k] += 1;
            }
        }
    }
    let mut f = std::fs::File::create(o.res.join(format!("determinisme-{}.csv", o.etiquette))).unwrap();
    writeln!(f, "etiquette,threads,ordre,tiles,tiles_differentes,empreinte").unwrap();
    for (k, (th, ord)) in variantes.iter().enumerate() {
        writeln!(f, "{},{},{:?},{},{},{}", o.etiquette, th, ord, n, diff[k], glob[k].finalize().to_hex()).unwrap();
    }
}

// ------------------------------------------------------------ Étalement

pub fn etalement(o: &Opt, budget: f64) {
    let p = Params::default();
    let cache = Cache::new(o.sortie, o.gpkg, Quoi { relief: true, reseau: true });
    let mut f = std::fs::File::create(o.res.join(format!("etale-{}.csv", o.etiquette))).unwrap();
    writeln!(f, "etiquette,tile,batiments,ms_preparation,ms_portes,ms_un_seul_tenant,ticks,tick_med_ms,tick_max_ms,ms_fusion,ticks_total").unwrap();
    let etats = HashMap::new();
    for &t in &o.tiles {
        let Some(ctx) = cache.contexte(t) else { continue };
        let mut v1 = Vec::new();
        let mut last = None;
        for _ in 0..5 {
            let t0 = Instant::now();
            let pr = generation::preparer(&ctx, &p, true);
            let g = Gen { ctx: &ctx, prep: &pr, p: &p, lod: Lod::Iso, etats: &etats };
            let (s, _) = generation::tout(&g, false, Ordre::Direct);
            let _ = generation::fusionner(&s);
            v1.push(ms(t0));
            last = Some(pr);
        }
        let pr = last.unwrap();
        let g = Gen { ctx: &ctx, prep: &pr, p: &p, lod: Lod::Iso, etats: &etats };
        let (s, mut ticks) = generation::etale(&g, budget);
        let tf = Instant::now();
        let _ = generation::fusionner(&s);
        let msf = ms(tf);
        let n = ticks.len();
        let mx = ticks.iter().copied().fold(0.0, f64::max);
        // Préparation (repli, grille, portes) : un tick à part par étape, ou
        // précalculée par la chaîne (portes).
        let tp = pr.ms_repli + pr.ms_grille + pr.ms_portes + pr.ms_porches;
        let tt = n + (pr.ms_grille / budget).ceil() as usize + (pr.ms_portes / budget).ceil() as usize + 1 + (msf / budget).ceil() as usize;
        writeln!(f, "{},{},{},{:.2},{:.2},{:.2},{},{:.2},{:.2},{:.2},{}", o.etiquette, t.nom(), g.n(), tp, pr.ms_grille + pr.ms_portes, quantile(&mut v1, 0.5), n, quantile(&mut ticks, 0.5), mx, msf, tt).unwrap();
    }
}

// ------------------------------------------------------------ Destruction (CPU)

pub fn destruction(o: &Opt) {
    let p = Params::default();
    let cache = Cache::new(o.sortie, o.gpkg, Quoi { relief: true, reseau: true });
    let t = o.tiles[0];
    let ctx = cache.contexte(t).unwrap();
    let pr = generation::preparer(&ctx, &p, true);
    let mut etats: HashMap<u64, Etat> = HashMap::new();
    let bats = &ctx.tile().bats;
    let mut f = std::fs::File::create(o.res.join(format!("destruction-{}.csv", o.etiquette))).unwrap();
    writeln!(f, "etiquette,tile,batiment,aire_m2,h_m,etape,us_batiment,ms_fusion_tile,ms_fusion_morceau,tri_avant,tri_apres,cases_franch_bloquees,us_franch").unwrap();
    // Bâtiments étudiés : médian, et le plus grand, près du centre.
    let mut par_aire: Vec<usize> = (0..bats.len()).filter(|&k| { let c = porte::centroide(&bats[k]); c[0].abs() < 300.0 && c[1].abs() < 300.0 }).collect();
    par_aire.sort_by(|a, b| bats[*a].aire().partial_cmp(&bats[*b].aire()).unwrap());
    let choix = [par_aire[par_aire.len() / 2], *par_aire.last().unwrap()];
    let g0 = Gen { ctx: &ctx, prep: &pr, p: &p, lod: Lod::Iso, etats: &etats };
    let (mut s, _) = generation::tout(&g0, false, Ordre::Direct);
    let morceaux: Vec<(i32, i32)> = bats.iter().map(|b| generation::morceau_de(b, 250.0)).collect();
    for &k in &choix {
        let b = &bats[k];
        for etape in [Etat::Endommage, Etat::Effondre] {
            etats.insert(b.id, etape);
            let g = Gen { ctx: &ctx, prep: &pr, p: &p, lod: Lod::Iso, etats: &etats };
            let tri_avant = s[k].mesh.triangles();
            let mut v = Vec::new();
            let mut nouv = None;
            for _ in 0..20 {
                let t0 = Instant::now();
                nouv = Some(g.bat(k));
                v.push(t0.elapsed().as_secs_f64() * 1e6);
            }
            let nouv = nouv.unwrap();
            let tri_apres = nouv.mesh.triangles();
            // Franchissable : les cases des gravats deviennent obstacles.
            let mut fr = ctx.tile().franch.clone();
            let tf = Instant::now();
            let mut bloquees = 0;
            for &(gi, gj) in &nouv.gravats {
                if (0..500).contains(&gi) && (0..500).contains(&gj) && fr.get(gi as usize, gj as usize) {
                    fr.clear(gi as usize, gj as usize);
                    bloquees += 1;
                }
            }
            let us_fr = tf.elapsed().as_secs_f64() * 1e6;
            s[k] = nouv;
            let mut vf = Vec::new();
            for _ in 0..5 {
                let t0 = Instant::now();
                let _ = generation::fusionner(&s);
                vf.push(ms(t0));
            }
            let sel: Vec<usize> = (0..bats.len()).filter(|&j| morceaux[j] == morceaux[k]).collect();
            let sub: Vec<Sortie> = Vec::new();
            let _ = sub;
            let mut vm = Vec::new();
            for _ in 0..5 {
                let t0 = Instant::now();
                let mut m = batiment::Maillage::default();
                for &j in &sel {
                    m.ajouter(&s[j].mesh);
                }
                vm.push(ms(t0));
            }
            writeln!(
                f,
                "{},{},{},{:.0},{:.1},{:?},{:.1},{:.3},{:.3},{},{},{},{:.1}",
                o.etiquette,
                t.nom(),
                b.id,
                b.aire(),
                pr.modeles[k].h,
                etape,
                quantile(&mut v, 0.5),
                quantile(&mut vf, 0.5),
                quantile(&mut vm, 0.5),
                tri_avant,
                tri_apres,
                bloquees,
                us_fr
            )
            .unwrap();
        }
    }
    // Sauvegarde : seulement les bâtiments modifiés (A-13).
    let mut f = std::fs::File::create(o.res.join(format!("sauvegarde-{}.csv", o.etiquette))).unwrap();
    writeln!(f, "etiquette,tile,batiments,part_modifies,modifies,octets_bruts,octets_zstd").unwrap();
    for part in [0.01, 0.1, 1.0] {
        let mut e = Ecrivain::default();
        let mut pid = 0u64;
        let mut n = 0;
        for b in bats {
            let h = batiment::hacher(b.id ^ 0x5A5A);
            if (h % 10_000) as f64 >= part * 10_000.0 {
                continue;
            }
            n += 1;
            e.u(b.id - pid);
            pid = b.id;
            let etat = (h >> 20) % 3; // intact réparé, endommagé, effondré
            e.o(etat as u8);
            if etat == 1 {
                // barre de vie sur 16 bits
                e.b.extend_from_slice(&((h >> 32) as u16).to_le_bytes());
            }
        }
        let z = zstd::bulk::compress(&e.b, 19).unwrap().len();
        writeln!(f, "{},{},{},{},{},{},{}", o.etiquette, t.nom(), bats.len(), part, n, e.b.len(), z).unwrap();
    }
}

// ------------------------------------------------------------ Repli de la hauteur

pub fn repli(o: &Opt) {
    let p = Params::default();
    let ty = types(o.sortie);
    let cache = Cache::new(o.sortie, o.gpkg, Quoi { relief: false, reseau: false });
    let liste = a_traiter(&ty, &o.tiles);
    // Erreurs (m) par règle, sur les bâtiments dont la hauteur est connue.
    let regles = ["constante_6m", "constante_usage", "mediane_tile", "etages_x3", "regle_p08"];
    let mut err: Vec<Vec<f64>> = vec![Vec::new(); regles.len()];
    let mut signe: Vec<f64> = vec![0.0; regles.len()];
    for &t in &liste {
        let Some(tile) = cache.tile(t) else { continue };
        cache.oublier(|_| false);
        let r = batiment::repli(&tile.bats, None);
        for b in &tile.bats {
            let Some(h) = b.hauteur_dm.filter(|h| *h >= 10).map(|h| h as f64 / 10.0) else { continue };
            let med = r.par_classe.get(&(b.usage1, batiment::classe_emprise(b.aire()))).or_else(|| r.par_usage.get(&b.usage1)).copied();
            let preds = [
                Some(6.0),
                Some(batiment::constante_usage(b)),
                Some(med.unwrap_or(batiment::constante_usage(b))),
                b.etages.filter(|e| *e > 0).map(|e| e as f64 * p.etage),
                Some(batiment::hauteur(b, &r, &p, false).0),
            ];
            for (k, pr) in preds.iter().enumerate() {
                if let Some(x) = pr {
                    err[k].push((x - h).abs());
                    signe[k] += x - h;
                }
            }
        }
    }
    let mut f = std::fs::File::create(o.res.join(format!("repli-{}.csv", o.etiquette))).unwrap();
    writeln!(f, "etiquette,regle,batiments,erreur_med_m,erreur_p90_m,biais_moyen_m,part_sous_2m").unwrap();
    for (k, nom) in regles.iter().enumerate() {
        let n = err[k].len();
        let s2 = err[k].iter().filter(|e| **e < 2.0).count() as f64 / n.max(1) as f64;
        writeln!(f, "{},{},{},{:.2},{:.2},{:.2},{:.3}", o.etiquette, nom, n, quantile(&mut err[k], 0.5), quantile(&mut err[k], 0.9), signe[k] / n.max(1) as f64, s2).unwrap();
    }
}

// ------------------------------------------------------------ Format : attributs à ajouter

/// Taille de la section `bati` avec les attributs proposés, contre celle de
/// P-07 (zstd 19 dans les deux cas). Colonnes ajoutées en fin de section,
/// pour tous les bâtiments dans l'ordre : hauteur du toit (dm + 1, 0 si
/// absente), matériaux des murs et du toit (1 octet chacun), porte (rang de
/// l'arête extérieure et abscisse au 1/16 m), cases percées non nécessaires
/// (déjà dans `zone`).
pub fn format(o: &Opt) {
    let p = Params::default();
    let ty = types(o.sortie);
    let cache = Cache::new(o.sortie, o.gpkg, Quoi { relief: true, reseau: true });
    let liste = a_traiter(&ty, &o.tiles);
    let mut f = std::fs::File::create(o.res.join(format!("format-{}.csv", o.etiquette))).unwrap();
    writeln!(f, "etiquette,tile,type,batiments,bati_p07_z19,plus_toit_materiaux_z19,plus_porte_z19,ms_portes_calculees").unwrap();
    let mut iy = i32::MIN;
    for &t in &liste {
        if t.iy != iy {
            oublier_avant(&cache, t.iy);
            iy = t.iy;
        }
        let Some(ctx) = cache.contexte(t) else { continue };
        let tile = ctx.tile();
        if tile.bats.is_empty() {
            continue;
        }
        let b = cache.pq.lire(t).unwrap();
        let tl = tuile::lire(&b, false);
        let brut = tl.section(Sec::Bati).unwrap();
        let z0 = zstd::bulk::compress(&brut, 19).unwrap().len();
        let mut e = Ecrivain { b: brut.clone() };
        for bt in &tile.bats {
            e.u(bt.extra.toit_dm.map(|d| d as u64 + 1).unwrap_or(0));
        }
        for bt in &tile.bats {
            e.o(bt.extra.murs);
        }
        for bt in &tile.bats {
            e.o(bt.extra.toiture);
        }
        let z1 = zstd::bulk::compress(&e.b, 19).unwrap().len();
        let t0 = Instant::now();
        let pr = generation::preparer(&ctx, &p, true);
        let msp = pr.ms_grille + pr.ms_portes;
        let _ = t0;
        for (k, bt) in tile.bats.iter().enumerate() {
            // Rang de l'arête et abscisse : retrouvés depuis le point de la porte.
            let (mut rang, mut absc) = (0u64, 0u64);
            if let Some(po) = pr.portes[k].and_then(|c| c.porte) {
                let mut ka = 0u64;
                let mut best = f64::MAX;
                for poly in &bt.polys {
                    let an = batiment::anneaux_m(poly, None);
                    if an.is_empty() {
                        continue;
                    }
                    let a = &an[0];
                    for i in 0..a.len() {
                        let d = geom::dist_seg(po.p, a[i], a[(i + 1) % a.len()]);
                        if d < best {
                            best = d;
                            rang = ka;
                            let (dx, dy) = (po.p[0] - a[i][0], po.p[1] - a[i][1]);
                            absc = ((dx * dx + dy * dy).sqrt() * 16.0).round() as u64;
                        }
                        ka += 1;
                    }
                }
            }
            e.u(rang);
            e.u(absc);
        }
        let z2 = zstd::bulk::compress(&e.b, 19).unwrap().len();
        writeln!(f, "{},{},{},{},{},{},{},{:.2}", o.etiquette, t.nom(), ty.get(&t).map(|x| x.0.as_str()).unwrap_or(""), tile.bats.len(), z0, z1, z2, msp).unwrap();
    }
}

// ------------------------------------------------------------ Monuments

pub fn monuments(o: &Opt) {
    let ty = types(o.sortie);
    let cache = Cache::new(o.sortie, None, Quoi { relief: false, reseau: false });
    let liste = a_traiter(&ty, &[]);
    let mut par: BTreeMap<u8, Vec<f64>> = BTreeMap::new();
    let mut tiles_par: BTreeMap<u8, std::collections::BTreeSet<TileId>> = BTreeMap::new();
    for &t in &liste {
        let Some(tile) = cache.tile(t) else { continue };
        cache.oublier(|_| false);
        for b in &tile.bats {
            par.entry(b.nature).or_default().push(b.aire());
            tiles_par.entry(b.nature).or_default().insert(t);
        }
    }
    let mut f = std::fs::File::create(o.res.join(format!("monuments-{}.csv", o.etiquette))).unwrap();
    writeln!(f, "etiquette,nature,batiments,tiles,aire_med_m2,aire_p90_m2,aire_max_m2,plus_de_500m2,plus_de_2000m2").unwrap();
    for (n, v) in par.iter_mut() {
        let k = v.len();
        let a500 = v.iter().filter(|a| **a > 500.0).count();
        let a2000 = v.iter().filter(|a| **a > 2000.0).count();
        writeln!(
            f,
            "{},{},{},{},{:.0},{:.0},{:.0},{},{}",
            o.etiquette,
            donnees::nom_nature(*n).replace(',', ""),
            k,
            tiles_par[n].len(),
            quantile(v, 0.5),
            quantile(v, 0.9),
            v.last().unwrap(),
            a500,
            a2000
        )
        .unwrap();
    }
    let _ = nat::MONUMENT;
}

// ------------------------------------------------------------ Images des portes (vue de dessus)

pub fn images(o: &Opt) {
    let p = Params::default();
    let cache = Cache::new(o.sortie, o.gpkg, Quoi { relief: false, reseau: true });
    std::fs::create_dir_all(o.res).unwrap();
    for &t in &o.tiles {
        let Some(ctx) = cache.contexte(t) else { continue };
        let pr = generation::preparer(&ctx, &p, true);
        let g = pr.grille.as_ref().unwrap();
        let n = 1000usize;
        let mut img = vec![0u8; n * n * 3];
        let px = |img: &mut Vec<u8>, i: i64, j: i64, c: [u8; 3]| {
            if i >= 0 && j >= 0 && (i as usize) < n && (j as usize) < n {
                let k = ((n - 1 - j as usize) * n + i as usize) * 3;
                img[k..k + 3].copy_from_slice(&c);
            }
        };
        for j in 0..n as i64 {
            for i in 0..n as i64 {
                let (gi, gj) = ((i / 2) as i32, (j / 2) as i32);
                let k = porte::Grille::k(gi, gj).unwrap();
                let c = if g.route[k] && g.acc[k] {
                    [170, 200, 230]
                } else if g.acc[k] {
                    let d = g.dist[k].min(60) as u8;
                    [235 - d, 235 - d, 230 - d]
                } else {
                    [70, 70, 75]
                };
                px(&mut img, i, j, c);
            }
        }
        // Emprises.
        let w = Fenetre { x0: -500.0, y0: -500.0, res: 1.0, nx: n, ny: n };
        for dy in -1..=1 {
            for dx in -1..=1 {
                let Some(tt) = &ctx.tiles[((dy + 1) * 3 + dx + 1) as usize] else { continue };
                for b in &tt.bats {
                    for poly in &b.polys {
                        let an: Vec<Vec<P2>> = poly.iter().map(|a| a.iter().map(|q| [q[0] as f64 / 16.0 + dx as f64 * 1000.0, q[1] as f64 / 16.0 + dy as f64 * 1000.0]).collect()).collect();
                        raster::polygone(w, &an, |i, j| px(&mut img, i as i64, j as i64, [190, 150, 110]));
                    }
                }
            }
        }
        for &k in &ctx.tile().percees {
            let (i, j) = ((k % 500) as i64 * 2, (k / 500) as i64 * 2);
            for (a, b) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                px(&mut img, i + a, j + b, [255, 140, 0]);
            }
        }
        for c in pr.portes.iter().flatten() {
            if let Some(q) = c.p05_case {
                let (i, j) = (q.0 as i64 * 2, q.1 as i64 * 2);
                for (a, b) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    px(&mut img, i + a, j + b, [30, 80, 230]);
                }
            }
            if let Some(po) = c.porte {
                let (i, j) = ((po.p[0] + 500.0).floor() as i64, (po.p[1] + 500.0).floor() as i64);
                for a in -1..=1 {
                    for b in -1..=1 {
                        px(&mut img, i + a, j + b, [220, 20, 30]);
                    }
                }
            }
        }
        let nom = o.res.join(format!("portes-{}.png", t.nom()));
        let fic = std::fs::File::create(&nom).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(fic), n as u32, n as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&img).unwrap();
        eprintln!("{}", nom.display());
    }
}
