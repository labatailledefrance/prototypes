//! Tests : toit couvrant exactement l'emprise, pignons raccordés au toit,
//! déterminisme, simplification, porches.

use crate::batiment::{self, Entree, Etat, Forme, Lod, Params};
use crate::donnees::{Bat, Extra};
use crate::geom;

fn bat(polys: Vec<Vec<Vec<[i32; 2]>>>, h: u32, toit: Option<u16>) -> Bat {
    Bat {
        id: 12345,
        nature: 0,
        usage1: 0,
        usage2: 254,
        leger: false,
        hauteur_dm: Some(h),
        etages: None,
        logements: None,
        polys,
        extra: Extra { toit_dm: toit, gout_dm: None, murs: 255, toiture: 255 },
    }
}

fn sq(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<[i32; 2]> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

fn generer_f(b: &Bat, porches: &[(i32, i32)], forme: Option<Forme>) -> batiment::Sortie {
    let p = Params::default();
    let r = batiment::repli(std::slice::from_ref(b), None);
    let mut m = batiment::modele(b, &r, &p);
    if let Some(f) = forme {
        m.forme = f;
        m.dz = 4.0;
    }
    let sol = |_x: f64, _y: f64| Some(10.0);
    batiment::generer(&Entree { b, m: &m, sol: &sol, porte: None, porches, etat: Etat::Intact, lod: Lod::Iso, p: &p })
}

/// Un L avec une cour : pour chaque forme de toit, l'aire projetée du toit
/// égale l'aire de l'emprise.
#[test]
fn toit_couvre_emprise() {
    let l = vec![vec![[0, 0], [16 * 30, 0], [16 * 30, 16 * 12], [16 * 12, 16 * 12], [16 * 12, 16 * 25], [0, 16 * 25]], sq(16 * 3, 16 * 3, 16 * 8, 16 * 8)];
    for f in [Forme::Plat, Forme::DeuxPans, Forme::QuatrePans, Forme::Mansarde] {
        let b = bat(vec![l.clone()], 120, Some(40));
        let s = generer_f(&b, &[], Some(f));
        let err = (s.stats.aire_toit - s.stats.aire_emprise).abs() / s.stats.aire_emprise;
        assert!(err < 1e-9, "{f:?} : écart {err}");
        assert_eq!(s.stats.echecs_toit, 0);
    }
}

/// Le haut des murs suit exactement la fonction du toit.
#[test]
fn pignons_raccordes() {
    let b = bat(vec![vec![sq(0, 0, 16 * 20, 16 * 9)]], 60, Some(30));
    let s = generer_f(&b, &[], None);
    let m = &s.mesh;
    // Tout sommet de mur (normale horizontale) au-dessus de la gouttière est
    // un sommet du toit (même position, au f32 près).
    let toit: Vec<[f32; 3]> = (0..m.pos.len()).filter(|&k| m.nor[k][1] > 0.01).map(|k| m.pos[k]).collect();
    for k in 0..m.pos.len() {
        if m.nor[k][1].abs() < 1e-6 && m.pos[k][1] > s.stats.z_gout as f32 + 0.01 {
            assert!(toit.iter().any(|q| (q[0] - m.pos[k][0]).abs() < 1e-4 && (q[1] - m.pos[k][1]).abs() < 1e-4 && (q[2] - m.pos[k][2]).abs() < 1e-4), "sommet de pignon sans toit : {:?}", m.pos[k]);
        }
    }
}

#[test]
fn memes_octets() {
    let b = bat(vec![vec![vec![[0, 0], [400, 13], [389, 300], [120, 280], [7, 190]]]], 95, None);
    assert_eq!(generer_f(&b, &[], None).mesh.octets(), generer_f(&b, &[], None).mesh.octets());
}

#[test]
fn simplification_garde_le_rectangle() {
    let mut a = Vec::new();
    for k in 0..50 {
        a.push([k * 10, (k % 2) * 3]);
    }
    a.push([500, 300]);
    a.push([0, 300]);
    let s = geom::simplifier(&a, 16);
    assert_eq!(s.len(), 4, "{s:?}");
}

/// Un porche à travers un bâtiment : le toit reste entier, le bas est percé.
#[test]
fn porche() {
    // Bâtiment de 20 × 12 m centré sur la case (250, 250) de la tile.
    let b = bat(vec![vec![sq(-160, -96, 160, 96)]], 150, Some(5));
    let cases: Vec<(i32, i32)> = (247..253).map(|j| (250, j)).collect();
    let s = generer_f(&b, &cases, None);
    assert!(s.stats.porche);
    assert_eq!(s.stats.cases_porche, 6);
    let err = (s.stats.aire_toit - s.stats.aire_emprise).abs() / s.stats.aire_emprise;
    assert!(err < 1e-9);
}

/// Micro-banc (perf est interdit sur la machine) : coût par poste, sur la
/// tile du centre de Paris et sur Cherbourg. `cargo test --release banc --
/// --ignored --nocapture`.
#[test]
#[ignore]
fn banc_postes() {
    use crate::donnees::{Cache, Quoi};
    use crate::generation;
    use std::time::Instant;
    let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    for (nom, sortie, gpkg, t) in [
        ("paris", "generated/p07-paris", "sources/bdtopo-d075-2026-09-15/BDT_3-5_GPKG_LAMB93_D075-ED2026-09-15.gpkg", "x651_y6862"),
        ("cherbourg", "generated/p07-manche", "sources/bdtopo-d050-2026-09-15/D050.gpkg", "x366_y6958"),
    ] {
        let cache = Cache::new(&d.join(sortie), Some(&d.join(gpkg)), Quoi { relief: true, reseau: true });
        let ctx = cache.contexte(crate::geo::TileId::lire(t)).unwrap();
        let p = Params::default();
        let prep = generation::preparer(&ctx, &p, true);
        let bats = &ctx.tile().bats;
        let n = bats.len() as f64;
        let sol = |x: f64, y: f64| ctx.sol(x, y);
        let mesure = |lib: &str, f: &dyn Fn(usize)| {
            let mut v = Vec::new();
            for _ in 0..7 {
                let t0 = Instant::now();
                for k in 0..bats.len() {
                    f(k);
                }
                v.push(t0.elapsed().as_secs_f64() * 1e6 / n);
            }
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!("banc {nom} {lib} {:.2} µs/bâtiment", v[3]);
        };
        let gen_v = |k: usize, forme: Option<Forme>, porches: bool, lod: Lod| {
            let mut m = prep.modeles[k].clone();
            if let Some(f) = forme {
                m.forme = f;
            }
            let e = Entree { b: &bats[k], m: &m, sol: &sol, porte: None, porches: if porches { &prep.porches[k] } else { &[] }, etat: Etat::Intact, lod, p: &p };
            std::hint::black_box(batiment::generer(&e));
        };
        mesure("complet", &|k| gen_v(k, None, true, Lod::Iso));
        mesure("sans_porche", &|k| gen_v(k, None, false, Lod::Iso));
        mesure("toits_plats_sans_porche", &|k| gen_v(k, Some(Forme::Plat), false, Lod::Iso));
        mesure("vue_lointaine", &|k| gen_v(k, None, false, Lod::Loin));
        mesure("carte", &|k| gen_v(k, None, false, Lod::Carte));
        mesure("earcut_seul", &|k| {
            for poly in &bats[k].polys {
                let an = batiment::anneaux_m(poly, None);
                if !an.is_empty() {
                    std::hint::black_box(geom::trianguler(&an));
                }
            }
        });
        mesure("modele_seul", &|k| {
            std::hint::black_box(batiment::modele(&bats[k], &prep.repli, &p));
        });
        let t0 = Instant::now();
        let g = crate::porte::grille(&ctx);
        println!("banc {nom} grille_portes {:.2} ms", t0.elapsed().as_secs_f64() * 1e3);
        mesure("porte", &|k| {
            std::hint::black_box(crate::porte::choisir(&bats[k], &g));
        });
    }
}
