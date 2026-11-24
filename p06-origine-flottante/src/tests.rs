//! Tests (`cargo test --release`) et micro-bancs (`-- --ignored --nocapture`).

use bevy::math::{DVec2, DVec3, Vec2};

use crate::geo::{self, TileId};
use crate::simu::{self, Champ, MarcheurB, MarcheurC};

#[test]
fn projections_aller_retour() {
    for l in geo::lieux() {
        let z = geo::zone_par_epsg(l.epsg);
        for dx in [-0.05, 0.0, 0.05] {
            for dy in [-0.05, 0.0, 0.05] {
                let g = geo::geo(l.g.lat + dy, l.g.lon + dx);
                let p = z.proj.avant(g);
                let e = (geo::vers_ecef(z.proj.inverse(p)) - geo::vers_ecef(g)).length();
                assert!(e < 1e-6, "{} : {e} m", l.nom);
            }
        }
    }
}

#[test]
fn lambert93_origine() {
    let z = geo::zone_par_epsg(2154);
    let p = z.proj.avant(geo::geo(46.5, 3.0));
    assert!((p - DVec2::new(700_000.0, 6_600_000.0)).length() < 1e-6);
}

#[test]
fn ecef_aller_retour() {
    for l in geo::lieux() {
        let mut g = l.g;
        g.h = 1234.5;
        let g2 = geo::depuis_ecef(geo::vers_ecef(g));
        assert!((geo::vers_ecef(g2) - geo::vers_ecef(g)).length() < 1e-6);
    }
}

#[test]
fn repere_quadratique_au_dixieme_de_millimetre() {
    for nom in ["Brest", "Bonifacio", "Papeete", "Île Walpole", "Mata-Utu"] {
        let l = geo::lieu(nom);
        let z = geo::zone_par_epsg(l.epsg);
        let id = TileId::de_grille(l.epsg, z.proj.avant(l.g));
        let r = geo::repere_tile(&z, id);
        for (x, zz) in [(0.0, 0.0), (1000.0, -1000.0), (0.0, -1000.0), (1000.0, 0.0), (500.0, -500.0)] {
            let loc = DVec3::new(x, 0.0, zz);
            let e = (r.vers_ecef(loc) - geo::local_vers_ecef_exact(&z, id, loc)).length();
            assert!(e < 1e-4, "{nom} : {e} m");
        }
    }
}

/// Coordonnées locales centrées : les deux passages de tile sont exacts
/// (lemme de Sterbenz). Depuis le coin, le passage vers l'ouest arrondit.
#[test]
fn passage_de_tile_exact() {
    let mut x = 499.99f32;
    for _ in 0..10_000 {
        let avant = x;
        let y = if x >= 500.0 { x - 1000.0 } else { x };
        if x >= 500.0 {
            assert_eq!((y + 1000.0).to_bits(), avant.to_bits());
        }
        x = f32::from_bits(x.to_bits() + 1);
    }
    // Côté ouest : de −500 vers −500,6 (les bits croissants d'un négatif s'éloignent de zéro).
    let mut x = -500.0f32;
    for _ in 0..10_000 {
        x = f32::from_bits(x.to_bits() + 1);
        let y = x + 1000.0;
        assert_eq!((y - 1000.0).to_bits(), x.to_bits(), "{x}");
    }
    // Depuis le coin : −0,03 + 1000 n'est pas représentable exactement.
    let w = -0.03f32;
    assert_ne!(((w + 1000.0) - 1000.0).to_bits(), w.to_bits());
}

#[test]
fn simulation_independante_des_threads_et_des_lots() {
    let a = simu::lancer('c', 20_000, 60, 1, 64, "Brest", None).somme;
    let b = simu::lancer('c', 20_000, 60, 4, 100_000, "Brest", None).somme;
    let c = simu::lancer('c', 20_000, 60, 16, 1024, "Brest", Some("Papeete")).somme;
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn simulation_invariante_par_translation() {
    let a = simu::lancer('c', 5_000, 60, 4, 1024, "Brest", None).somme;
    let b = simu::lancer('c', 5_000, 60, 4, 1024, "Île Walpole", None).somme;
    assert_eq!(a, b);
}

// ------------------------------------------------------------------ Micro-bancs

fn chrono<F: FnMut()>(n: usize, mut f: F) -> f64 {
    let t = std::time::Instant::now();
    f();
    t.elapsed().as_secs_f64() * 1e9 / n as f64
}

#[test]
#[ignore]
fn banc_marche() {
    let n = 1_000_000;
    let champ = Champ::nouveau();
    let (mut c, mut b, mut a) = simu::depart(n, "Brest");
    let mut b2: Vec<MarcheurB> = b.clone();
    let proj = geo::zone_par_epsg(2154).proj;
    for k in 0..3u64 {
        let tc = chrono(n, || c.iter_mut().for_each(|m: &mut MarcheurC| simu::pas_c(m, &champ.0, k)));
        let tb = chrono(n, || b.iter_mut().for_each(|m| simu::pas_b(m, &champ.0, k)));
        let tf = chrono(n, || b2.iter_mut().for_each(|m| simu::pas_b_floor(m, &champ.0, k)));
        let ta = chrono(n, || a.iter_mut().for_each(|m| simu::pas_a(m, &champ.0, k, &proj)));
        println!("marche (ns par marcheur, un thread) : C {tc:.1} ; B {tb:.1} ; B avec floor {tf:.1} ; A {ta:.1}");
    }
    let s: f32 = c.iter().map(|m| m.pos.x).sum::<f32>() + b.iter().map(|m| m.pos.x as f32).sum::<f32>();
    assert!(s.is_finite());
}

/// Coût du calcul d'une position relative à l'origine (f64 → f32) par modèle,
/// hors ECS : borne basse du recalage.
#[test]
#[ignore]
fn banc_recalage() {
    let n = 1_000_000;
    let l = geo::lieu("Brest");
    let z = geo::zone_par_epsg(2154);
    let id = TileId::de_grille(2154, z.proj.avant(l.g));
    let r = geo::repere_tile(&z, id);
    let locs: Vec<DVec3> = (0..n).map(|i| DVec3::new((i % 1000) as f64, 0.0, -(((i / 1000) % 1000) as f64))).collect();
    let o = geo::vers_ecef(l.g);
    let rot = geo::bevy_depuis_ecef(l.g);
    let mut sortie = vec![Vec2::ZERO; n];
    for _ in 0..3 {
        let t1 = chrono(n, || {
            for (s, p) in sortie.iter_mut().zip(&locs) {
                let e = r.ecef_depuis_local.transform_point3(*p);
                let v = rot * (e - o);
                *s = Vec2::new(v.x as f32, v.z as f32);
            }
        });
        let t2 = chrono(n, || {
            for (s, p) in sortie.iter_mut().zip(&locs) {
                let e = r.vers_ecef(*p);
                let v = rot * (e - o);
                *s = Vec2::new(v.x as f32, v.z as f32);
            }
        });
        println!("recalage (ns par modèle, un thread) : affine {t1:.1} ; quadratique {t2:.1}");
    }
    assert!(sortie[1].x.is_finite());
}
