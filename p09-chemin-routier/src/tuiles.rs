//! Lecture des tiles de P-07 : paquets, sections `reseau` et `ponts`.
//!
//! Copie réduite de `p07-chaine-manche/src/tuile.rs` et `decodage.rs`, avec
//! tous les attributs des morceaux de route (sens, accès, vitesse,
//! restrictions), que le décodeur de P-07 ignorait. Les coordonnées sont
//! rendues en **entiers globaux au 1/16 m** : coordonnée locale quantifiée +
//! centre de la tile × 16. Le centre est un entier de mètres, donc deux
//! tiles voisines donnent exactement le même entier pour un même point de
//! coupe (la chaîne calcule les coupes une fois, en coordonnées globales).

use std::path::Path;

pub const QUANT: i64 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileId {
    pub ix: i32,
    pub iy: i32,
}

impl TileId {
    /// Centre de la tile, en 1/16 m.
    pub fn centre_q(&self) -> (i64, i64) {
        ((self.ix as i64 * 1000 + 500) * QUANT, (self.iy as i64 * 1000 + 500) * QUANT)
    }
    pub fn de_q(x: i64, y: i64) -> TileId {
        TileId { ix: x.div_euclid(1000 * QUANT) as i32, iy: y.div_euclid(1000 * QUANT) as i32 }
    }
}

pub struct LecteurV<'a> {
    pub b: &'a [u8],
    pub p: usize,
}

impl<'a> LecteurV<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        LecteurV { b, p: 0 }
    }
    pub fn u(&mut self) -> u64 {
        let (mut v, mut s) = (0u64, 0);
        loop {
            let o = self.b[self.p];
            self.p += 1;
            v |= ((o & 0x7F) as u64) << s;
            if o & 0x80 == 0 {
                return v;
            }
            s += 7;
        }
    }
    pub fn i(&mut self) -> i64 {
        let z = self.u();
        ((z >> 1) as i64) ^ -((z & 1) as i64)
    }
    pub fn o(&mut self) -> u8 {
        self.p += 1;
        self.b[self.p - 1]
    }
    pub fn s(&mut self) -> String {
        let n = self.u() as usize;
        self.p += n;
        String::from_utf8_lossy(&self.b[self.p - n..self.p]).into_owned()
    }
}

/// Sections utiles d'une tile (décompressées).
pub struct TileBrute {
    pub id: TileId,
    pub reseau: Option<Vec<u8>>,
    pub ponts: Option<Vec<u8>>,
    pub octets_reseau_z: usize,
    pub octets_ponts_z: usize,
}

/// Lit une tile (format P07T version 1) et décompresse `reseau` et `ponts`.
fn lire_tile(b: &[u8]) -> TileBrute {
    assert_eq!(&b[..4], b"P07T");
    let n = b.len() - 32;
    assert_eq!(blake3::hash(&b[..n]).as_bytes(), &b[n..], "empreinte de tile fausse");
    let ix = i32::from_le_bytes(b[6..10].try_into().unwrap());
    let iy = i32::from_le_bytes(b[10..14].try_into().unwrap());
    let mut p = 14 + 1 + 2 + 4 + 4;
    let ns = b[p] as usize;
    p += 1;
    let mut table = Vec::new();
    for _ in 0..ns {
        let sec = b[p];
        let brut = u32::from_le_bytes(b[p + 1..p + 5].try_into().unwrap()) as usize;
        let st = u32::from_le_bytes(b[p + 5..p + 9].try_into().unwrap()) as usize;
        table.push((sec, brut, st));
        p += 9;
    }
    let mut t = TileBrute { id: TileId { ix, iy }, reseau: None, ponts: None, octets_reseau_z: 0, octets_ponts_z: 0 };
    for (sec, brut, st) in table {
        let d = &b[p..p + st];
        p += st;
        match sec {
            5 => {
                t.reseau = Some(zstd::bulk::decompress(d, brut).unwrap());
                t.octets_reseau_z = st;
            }
            6 => {
                t.ponts = Some(zstd::bulk::decompress(d, brut).unwrap());
                t.octets_ponts_z = st;
            }
            _ => {}
        }
    }
    t
}

/// Toutes les tiles des paquets d'un dossier `paquets/`, dans l'ordre des
/// fichiers puis de l'index (l'ordre ne compte pas : le graphe est canonique).
pub fn lire_paquets(dossier: &Path) -> Vec<TileBrute> {
    let mut fichiers: Vec<_> = std::fs::read_dir(dossier).unwrap().map(|e| e.unwrap().path()).collect();
    fichiers.sort();
    use rayon::prelude::*;
    fichiers
        .par_iter()
        .flat_map_iter(|f| {
            let b = std::fs::read(f).unwrap();
            assert_eq!(&b[..4], b"P07P");
            let n = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
            let mut out = Vec::with_capacity(n);
            for k in 0..n {
                let p = 8 + k * 20;
                let off = u64::from_le_bytes(b[p + 8..p + 16].try_into().unwrap()) as usize;
                let t = u32::from_le_bytes(b[p + 16..p + 20].try_into().unwrap()) as usize;
                out.push(lire_tile(&b[off..off + t]));
            }
            out.into_iter()
        })
        .collect()
}

/// Morceau de route décodé (coordonnées globales au 1/16 m ; z en dm, 0 si
/// la route est au sol).
#[derive(Clone, Debug)]
pub struct Morceau {
    pub tile: TileId,
    pub id: u64,
    pub rang: u16,
    pub attr: Attr,
    pub pts: Vec<[i64; 3]>,
}

/// Attributs d'un tronçon de la BD TOPO, tels que la chaîne de P-07 les écrit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Attr {
    pub nature: u8,
    pub importance: u8,
    pub pos_sol: i8,
    pub largeur_dm: u16,
    pub voies: u8,
    pub sens: u8,   // 0 double, 1 direct, 2 inverse, 3 sans objet
    pub acces: u8,  // 0 libre, 1 péage, 2 restreint, 3 physiquement impossible, 4 saisonnier
    pub urbain: bool,
    pub vitesse: u8, // km/h, 0 = inconnue
    pub poids_dt: u16,
    pub hauteur_dm: u16,
    pub largeur_r_dm: u16,
}

fn ligne(l: &mut LecteurV, z: bool, cx: i64, cy: i64) -> Vec<[i64; 3]> {
    let n = l.u() as usize;
    let (mut x, mut y, mut zz) = (0i64, 0i64, 0i64);
    (0..n)
        .map(|_| {
            x += l.i();
            y += l.i();
            if z {
                zz += l.i();
            }
            [x + cx, y + cy, zz]
        })
        .collect()
}

/// Morceaux de route d'une section `reseau` (les voies ferrées sont lues
/// puis comptées à part).
pub fn reseau(t: TileId, b: &[u8]) -> (Vec<Morceau>, usize) {
    let (cx, cy) = t.centre_q();
    let mut l = LecteurV::new(b);
    let n = l.u() as usize;
    let mut out = Vec::with_capacity(n);
    let mut id = 0i64;
    for _ in 0..n {
        id += l.i();
        let rang = l.u() as u16;
        let nature = l.o();
        let importance = l.o();
        let pos_sol = l.o() as i8;
        let largeur_dm = l.u() as u16;
        let voies = l.o();
        let sens = l.o();
        let a = l.o();
        let vitesse = l.o();
        let poids_dt = l.u() as u16;
        let hauteur_dm = l.u() as u16;
        let largeur_r_dm = l.u() as u16;
        let pts = ligne(&mut l, pos_sol != 0, cx, cy);
        let attr = Attr { nature, importance, pos_sol, largeur_dm, voies, sens, acces: a & 0x7F, urbain: a & 0x80 != 0, vitesse, poids_dt, hauteur_dm, largeur_r_dm };
        out.push(Morceau { tile: t, id: id as u64, rang, attr, pts });
    }
    let nv = l.u() as usize;
    for _ in 0..nv {
        l.i();
        l.u();
        l.o();
        let pos_sol = l.o() as i8;
        l.o();
        l.o();
        ligne(&mut l, pos_sol != 0, cx, cy);
    }
    (out, nv)
}

/// Pont décodé (B-03) : attributs calculés par la chaîne de P-07.
#[derive(Clone, Debug)]
pub struct Pont {
    pub tile: TileId,
    pub id: u64,
    pub longueur_dm: u32,
    pub largeur_dm: u16,
    pub importance: u8, // de la route portée la plus importante ; 9 = aucune
    pub nature_route: u8,
    pub poids_dt: u16,
    pub franchit: u8, // 1 eau, 2 route, 4 voie ferrée
    pub routes: u16,
    pub voies: u16,
    pub toponyme: String,
    /// Points d'échantillonnage de la géométrie (sommets), globaux, 1/16 m.
    pub pts: Vec<[i64; 2]>,
}

fn poly(l: &mut LecteurV, cx: i64, cy: i64, pts: &mut Vec<[i64; 2]>) {
    let na = l.u() as usize;
    for _ in 0..na {
        let n = l.u() as usize;
        let (mut x, mut y) = (0i64, 0i64);
        for _ in 0..n {
            x += l.i();
            y += l.i();
            pts.push([x + cx, y + cy]);
        }
    }
}

pub fn ponts(t: TileId, b: &[u8]) -> Vec<Pont> {
    let (cx, cy) = t.centre_q();
    let mut l = LecteurV::new(b);
    let n = l.u() as usize;
    (0..n)
        .map(|_| {
            let id = l.u();
            let longueur_dm = l.u() as u32;
            let largeur_dm = l.u() as u16;
            let importance = l.o();
            let nature_route = l.o();
            let poids_dt = l.u() as u16;
            let franchit = l.o();
            let routes = l.u() as u16;
            let voies = l.u() as u16;
            let toponyme = l.s();
            let mut pts = Vec::new();
            match l.o() {
                0 => {}
                1 => {
                    let x = l.i();
                    let y = l.i();
                    pts.push([x + cx, y + cy]);
                }
                2 => {
                    let k = l.u() as usize;
                    for _ in 0..k {
                        pts.extend(ligne(&mut l, false, cx, cy).into_iter().map(|p| [p[0], p[1]]));
                    }
                }
                _ => {
                    let k = l.u() as usize;
                    for _ in 0..k {
                        poly(&mut l, cx, cy, &mut pts);
                    }
                }
            }
            Pont { tile: t, id, longueur_dm, largeur_dm, importance, nature_route, poids_dt, franchit, routes, voies, toponyme, pts }
        })
        .collect()
}
