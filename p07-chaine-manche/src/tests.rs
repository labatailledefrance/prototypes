//! Tests et micro-bancs (`cargo test --release`, bancs :
//! `cargo test --release banc -- --ignored --nocapture`).

use crate::chaine::{couper, longueur};
use crate::geo::TileId;
use crate::population::plus_fort_reste;
use crate::raster::{self, Fenetre};
use crate::relief::{self, Noeuds};
use crate::zone::UnionFind;
use std::time::Instant;

#[test]
fn decoupe_sans_perte_ni_doublon() {
    let l = vec![[349_500.3, 6_879_200.7, 10.0], [351_700.1, 6_881_900.2, 20.0], [350_200.0, 6_878_100.0, 5.0]];
    let m = couper(&l);
    let total: f64 = m.iter().map(|(_, p)| longueur(p)).sum();
    assert!((total - longueur(&l)).abs() < 1e-6);
    // Points de coupe partagés exactement, et chaque morceau dans sa tile.
    for w in m.windows(2) {
        assert_eq!(*w[0].1.last().unwrap(), w[1].1[0]);
    }
    for (t, p) in &m {
        for s in p.windows(2) {
            let mid = ((s[0][0] + s[1][0]) / 2.0, (s[0][1] + s[1][1]) / 2.0);
            assert_eq!(TileId::de(mid.0, mid.1), *t);
        }
    }
}

#[test]
fn raster_partage_exact_entre_tiles() {
    // Un polygone à cheval sur deux tiles : même nombre de cases que dans une
    // fenêtre qui couvre les deux.
    let p = vec![vec![[999_000.0 - 37.3, 5_000_100.2, 0.0], [999_000.0 + 51.7, 5_000_130.9, 0.0], [999_000.0 + 12.1, 5_000_420.4, 0.0], [999_000.0 - 80.0, 5_000_300.0, 0.0]]];
    let (mut a, mut b, mut tout) = (0, 0, 0);
    raster::polygone(Fenetre::tile(998_000.0, 5_000_000.0, 1.0), &p, |_, _| a += 1);
    raster::polygone(Fenetre::tile(999_000.0, 5_000_000.0, 1.0), &p, |_, _| b += 1);
    raster::polygone(Fenetre { x0: 998_000.0, y0: 5_000_000.0, res: 1.0, nx: 2000, ny: 1000 }, &p, |_, _| tout += 1);
    assert_eq!(a + b, tout);
    assert!(a > 0 && b > 0);
}

fn grille_test(r: usize) -> Noeuds {
    let mut g = Noeuds::new(r);
    for j in 0..g.m {
        for i in 0..g.m {
            let k = j * g.m + i;
            g.cm[k] = if (i + j) % 97 == 0 { relief::SANS } else { 1000 + (i as i32 * 37 + j as i32 * 11) % 5000 - (i * j % 13) as i32 };
        }
    }
    g
}

#[test]
fn relief_aller_retour() {
    for r in [1, 2, 5] {
        let g = grille_test(r);
        let d = relief::decoder(&relief::encoder(&g, 1), r);
        assert_eq!(d.cm, g.cm);
        // Quantification idempotente : encoder le décodé redonne les mêmes octets.
        let e10 = relief::encoder(&g, 10);
        let d10 = relief::decoder(&e10, r);
        assert_eq!(relief::encoder(&d10, 10), e10);
    }
}

#[test]
fn asc_en_centimetres_exacts() {
    let t = b"ncols 2\nnrows 2\nxllcorner 349999.5\nyllcorner 6879998.5\ncellsize 1.0\nNODATA_value -99999.00\n 12.34 -0.05\n-99999.00 7\n";
    let d = relief::lire_asc(t);
    assert_eq!((d.x_no, d.y_no, d.pas), (350_000, 6_880_000, 1));
    assert_eq!(d.cm, vec![1234, -5, relief::SANS, 700]);
}

#[test]
fn plus_fort_reste_exact() {
    let p: Vec<(u64, f64)> = (0..37).map(|k| (k, 1.0 + (k % 5) as f64 * 0.37)).collect();
    for total in [0u64, 1, 7, 36, 37, 1000, 12345] {
        assert_eq!(plus_fort_reste(total, &p).iter().sum::<u64>(), total);
    }
    // Ne dépend pas de l'ordre des poids.
    let mut q = p.clone();
    q.reverse();
    let a = plus_fort_reste(100, &p);
    let mut b = plus_fort_reste(100, &q);
    b.reverse();
    assert_eq!(a, b);
}

#[test]
fn union_find_independante_de_l_ordre() {
    let paires: Vec<(u32, u32)> = (0..500u32).map(|k| ((k * 7919) % 1000, (k * 104_729) % 1000)).collect();
    let mut a = UnionFind::new(1000);
    paires.iter().for_each(|&(x, y)| a.unir(x, y));
    let mut b = UnionFind::new(1000);
    paires.iter().rev().for_each(|&(x, y)| b.unir(y, x));
    for k in 0..1000 {
        assert_eq!(a.trouver(k), b.trouver(k));
    }
}

#[test]
#[ignore]
fn banc_asc() {
    // Analyse d'une dalle ASC synthétique de 1 000 × 1 000 valeurs.
    let mut t = String::from("ncols 1000\nnrows 1000\nxllcorner 349999.5\nyllcorner 6879000.5\ncellsize 1.0\nNODATA_value -99999.00\n");
    for k in 0..1_000_000u32 {
        t.push_str(&format!(" {}.{:02}", 50 + k % 200, k % 100));
        if k % 1000 == 999 {
            t.push('\n');
        }
    }
    let b = t.into_bytes();
    let t0 = Instant::now();
    for _ in 0..10 {
        std::hint::black_box(relief::lire_asc(&b));
    }
    let s = t0.elapsed().as_secs_f64() / 10.0;
    println!("banc asc : {:.2} ms par dalle, {:.0} Mo/s", s * 1e3, b.len() as f64 / s / 1e6);
}

#[test]
#[ignore]
fn banc_relief() {
    for r in [1, 2, 5] {
        let g = grille_test(r);
        let e = relief::encoder(&g, 10);
        let z = zstd::bulk::compress(&e, 9).unwrap();
        let t0 = Instant::now();
        for _ in 0..20 {
            let b = zstd::bulk::decompress(&z, e.len()).unwrap();
            std::hint::black_box(relief::decoder(&b, r));
        }
        println!("banc relief : pas {r} m, décodage {:.3} ms", t0.elapsed().as_secs_f64() * 1e3 / 20.0);
    }
}
