//! Encodage compact d'un maillage, pour mesurer ce que coûterait de
//! **stocker** les bâtiments au lieu de les générer.
//!
//! Positions quantifiées au 1/32 m, en différences entre sommets successifs
//! (entiers variables) ; normales en octaèdre sur 2 octets ; coordonnées de
//! texture au 1/64 ; couleurs par palette de la tile ; indices en
//! différences. C'est un encodage raisonnable (de l'ordre de Draco sans
//! prédiction de parallélogramme), pas un optimum : il donne un ordre de
//! grandeur, que zstd réduit encore.

use crate::batiment::Maillage;
use crate::tuile::Ecrivain;
use std::collections::BTreeMap;

fn oct(n: [f32; 3]) -> [u8; 2] {
    let s = n[0].abs() + n[1].abs() + n[2].abs();
    let (mut x, mut y) = (n[0] / s, n[2] / s);
    if n[1] < 0.0 {
        let (ax, ay) = (x, y);
        x = (1.0 - ay.abs()) * if ax >= 0.0 { 1.0 } else { -1.0 };
        y = (1.0 - ax.abs()) * if ay >= 0.0 { 1.0 } else { -1.0 };
    }
    [((x * 127.0).round() as i8) as u8, ((y * 127.0).round() as i8) as u8]
}

pub fn compact(m: &Maillage) -> Vec<u8> {
    let mut e = Ecrivain::default();
    e.u(m.pos.len() as u64);
    e.u(m.idx.len() as u64);
    // Palette des couleurs.
    let mut pal: BTreeMap<[u8; 4], u32> = BTreeMap::new();
    for c in &m.col {
        let n = pal.len() as u32;
        pal.entry(*c).or_insert(n);
    }
    let mut pv: Vec<([u8; 4], u32)> = pal.iter().map(|(c, k)| (*c, *k)).collect();
    pv.sort_by_key(|x| x.1);
    e.u(pv.len() as u64);
    for (c, _) in &pv {
        e.b.extend_from_slice(c);
    }
    // Flux séparés (meilleure compression) : positions, normales, uv, couleurs.
    let mut pr = [0i64; 3];
    for p in &m.pos {
        for k in 0..3 {
            let q = (p[k] as f64 * 32.0).round() as i64;
            e.i(q - pr[k]);
            pr[k] = q;
        }
    }
    for n in &m.nor {
        e.b.extend_from_slice(&oct(*n));
    }
    let mut pu = [0i64; 2];
    for u in &m.uv {
        for k in 0..2 {
            let q = (u[k] as f64 * 64.0).round() as i64;
            e.i(q - pu[k]);
            pu[k] = q;
        }
    }
    for c in &m.col {
        e.u(pal[c] as u64);
    }
    let mut pi = 0i64;
    for &i in &m.idx {
        e.i(i as i64 - pi);
        pi = i as i64;
    }
    e.b
}
