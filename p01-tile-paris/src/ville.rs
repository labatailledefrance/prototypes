//! Ville synthétique : une tile de 1 km × 1 km découpée en îlots
//! haussmanniens, stockée comme une grille d'occupation de 1 m (1 bit par m²).
//!
//! Analogie : un bitmap d'allocation de blocs d'un système de fichiers. Un bit
//! à 1 = un mètre carré bâti. 1 000 × 1 000 bits = 125 ko, ce qui tient dans
//! le cache L2 d'un cœur (512 ko sur le 5700X).

use crate::alea::Alea;

/// Côté d'une tile, en mètres (et en cases de 1 m).
pub const COTE: usize = 1000;
pub const COTE_F: f32 = COTE as f32;

pub struct Ville {
    bits: Vec<u64>,
    /// Part de la surface bâtie (0 à 1).
    pub couverture: f32,
}

/// Découpe un axe en alternance rue / îlot. Renvoie les intervalles bâtis.
fn decouper(a: &mut Alea) -> Vec<(usize, usize)> {
    let mut ilots = Vec::new();
    let mut x = 0usize;
    loop {
        // Rue : 8 à 20 m, parfois un boulevard de 30 à 40 m.
        let rue = if a.f32() < 0.12 {
            a.entre(30, 40)
        } else {
            a.entre(8, 20)
        };
        x += rue;
        let long = a.entre(50, 140);
        if x + long + 8 > COTE {
            break;
        }
        ilots.push((x, x + long));
        x += long;
    }
    ilots
}

impl Ville {
    pub fn generer(graine: u64) -> Ville {
        let mut a = Alea::new(graine);
        let mut v = Ville {
            bits: vec![0; COTE * COTE / 64 + 1],
            couverture: 0.0,
        };
        let xs = decouper(&mut a);
        let ys = decouper(&mut a);
        for &(y0, y1) in &ys {
            for &(x0, x1) in &xs {
                // Environ 5 % des îlots sont des places ou des squares.
                if a.f32() < 0.05 {
                    continue;
                }
                for y in y0..y1 {
                    for x in x0..x1 {
                        v.poser(x, y, true);
                    }
                }
            }
        }
        // Deux avenues diagonales de 25 m, qui se croisent au centre.
        for y in 0..COTE {
            for x in 0..COTE {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let d1 = (fx - fy).abs() / std::f32::consts::SQRT_2;
                let d2 = (fx + fy - COTE_F).abs() / std::f32::consts::SQRT_2;
                if d1 < 12.5 || d2 < 12.5 {
                    v.poser(x, y, false);
                }
            }
        }
        let bati: u32 = v.bits.iter().map(|w| w.count_ones()).sum();
        v.couverture = bati as f32 / (COTE * COTE) as f32;
        v
    }

    /// Charge une tile réelle produite par `outils/extraire_tiles.py`.
    pub fn charger(chemin: &std::path::Path) -> std::io::Result<Ville> {
        let octets = std::fs::read(chemin)?;
        let n = COTE * COTE / 64 + 1;
        assert_eq!(octets.len(), 8 * n, "taille inattendue : {}", chemin.display());
        let bits: Vec<u64> = octets
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let bati: u32 = bits.iter().map(|w| w.count_ones()).sum();
        Ok(Ville {
            bits,
            couverture: bati as f32 / (COTE * COTE) as f32,
        })
    }

    fn poser(&mut self, x: usize, y: usize, b: bool) {
        let i = y * COTE + x;
        if b {
            self.bits[i >> 6] |= 1 << (i & 63);
        } else {
            self.bits[i >> 6] &= !(1 << (i & 63));
        }
    }

    /// Case bâtie ? Hors de la tile : considéré comme bâti (mur).
    #[inline(always)]
    pub fn bati(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= COTE as i32 || y >= COTE as i32 {
            return true;
        }
        let i = y as usize * COTE + x as usize;
        (self.bits[i >> 6] >> (i & 63)) & 1 == 1
    }

    #[inline(always)]
    pub fn bati_p(&self, p: [f32; 2]) -> bool {
        self.bati(p[0].floor() as i32, p[1].floor() as i32)
    }

    /// Ligne de vue entre deux points : parcours des cases traversées
    /// (algorithme d'Amanatides et Woo, « DDA »). Analogie : un traceroute
    /// case par case, qui s'arrête au premier mur.
    pub fn vue_degagee(&self, a: [f32; 2], b: [f32; 2]) -> bool {
        let (mut x, mut y) = (a[0].floor() as i32, a[1].floor() as i32);
        let (xe, ye) = (b[0].floor() as i32, b[1].floor() as i32);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let sx = if dx > 0.0 { 1 } else { -1 };
        let sy = if dy > 0.0 { 1 } else { -1 };
        let tdx = if dx != 0.0 { 1.0 / dx.abs() } else { f32::INFINITY };
        let tdy = if dy != 0.0 { 1.0 / dy.abs() } else { f32::INFINITY };
        let bord_x = if sx > 0 { (x + 1) as f32 } else { x as f32 };
        let bord_y = if sy > 0 { (y + 1) as f32 } else { y as f32 };
        let mut tmx = if dx != 0.0 { (bord_x - a[0]).abs() * tdx } else { f32::INFINITY };
        let mut tmy = if dy != 0.0 { (bord_y - a[1]).abs() * tdy } else { f32::INFINITY };
        let pas_max = (xe - x).abs() + (ye - y).abs();
        for _ in 0..pas_max {
            if tmx < tmy {
                x += sx;
                tmx += tdx;
            } else {
                y += sy;
                tmy += tdy;
            }
            if self.bati(x, y) {
                return false;
            }
        }
        true
    }

    /// Point libre (rue) tiré au hasard dans un disque, par rejet.
    pub fn point_libre(&self, a: &mut Alea, centre: [f32; 2], rayon: f32) -> [f32; 2] {
        loop {
            let p = [
                centre[0] + (a.f32() * 2.0 - 1.0) * rayon,
                centre[1] + (a.f32() * 2.0 - 1.0) * rayon,
            ];
            let d2 = (p[0] - centre[0]).powi(2) + (p[1] - centre[1]).powi(2);
            if d2 <= rayon * rayon && !self.bati_p(p) {
                return p;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn couverture_parisienne() {
        let v = Ville::generer(1);
        // Paris intra-muros dense : environ 50 à 65 % de surface bâtie.
        assert!((0.40..0.70).contains(&v.couverture), "{}", v.couverture);
        // Le centre est le croisement des deux diagonales : libre.
        assert!(!v.bati(500, 500));
    }

    #[test]
    fn ligne_de_vue() {
        let v = Ville::generer(1);
        // Le long de la diagonale : dégagé.
        assert!(v.vue_degagee([100.5, 100.5], [900.5, 900.5]));
        // À travers toute la tile en ligne droite horizontale décalée de la
        // diagonale, on finit forcément par croiser un îlot.
        assert!(!v.vue_degagee([0.5, 300.5], [999.5, 300.5]) || !v.vue_degagee([0.5, 700.5], [999.5, 700.5]));
    }
}
