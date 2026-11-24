//! Relief : décodage des nœuds de 2 m (copié de P-07, `relief.rs`, sans la lecture des dalles ni l'encodeur).

pub const SANS: i32 = i32::MIN;

/// Grille de nœuds d'une tile au pas `r` : (n+1)² valeurs en cm (ou SANS),
/// ligne j = 0 au **sud** (y = y0), comme les autres couches.
#[derive(Clone)]
pub struct Noeuds {
    pub r: usize,
    pub m: usize, // n + 1
    pub cm: Vec<i32>,
}

impl Noeuds {
    pub fn new(r: usize) -> Noeuds {
        let m = 1000 / r + 1;
        Noeuds { r, m, cm: vec![SANS; m * m] }
    }
    #[inline]
    pub fn get(&self, i: usize, j: usize) -> i32 {
        self.cm[j * self.m + i]
    }
}

/// Décode une grille encodée : altitudes quantifiées (× q cm), SANS hors
/// données.
pub fn decoder(b: &[u8], r: usize) -> Noeuds {
    let m = u16::from_le_bytes([b[0], b[1]]) as usize;
    let q = u16::from_le_bytes([b[2], b[3]]) as i32;
    let ne = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let mut g = Noeuds { r, m, cm: vec![SANS; m * m] };
    if b[8] == 1 {
        return g;
    }
    let ns = (m * m).div_ceil(8);
    let sans = &b[9..9 + ns];
    let bas = &b[9 + ns..9 + ns + m * m];
    let haut = &b[9 + ns + m * m..9 + ns + 2 * m * m];
    let esc = &b[9 + ns + 2 * m * m..9 + ns + 2 * m * m + ne];
    let mut ke = 0;
    let mut rec = vec![0i32; m * m];
    for j in 0..m {
        for i in 0..m {
            let k = j * m + i;
            let a = if i > 0 { rec[k - 1] } else if j > 0 { rec[k - m] } else { 0 };
            let bb = if j > 0 { rec[k - m] } else { a };
            let c = if i > 0 && j > 0 { rec[k - m - 1] } else { bb };
            let pred = med(a, bb, c);
            let mut z = bas[k] as u32 | (haut[k] as u32) << 8;
            if z == 0xFFFF {
                z = u32::from_le_bytes(esc[ke..ke + 4].try_into().unwrap());
                ke += 4;
            }
            let res = ((z >> 1) as i32) ^ -((z & 1) as i32);
            let v = pred.wrapping_add(res);
            rec[k] = v;
            if sans[k >> 3] >> (k & 7) & 1 == 0 {
                g.cm[k] = v * q;
            }
        }
    }
    g
}

#[inline]
fn med(a: i32, b: i32, c: i32) -> i32 {
    if c >= a.max(b) {
        a.min(b)
    } else if c <= a.min(b) {
        a.max(b)
    } else {
        a + b - c
    }
}

