//! Décodage des sections d'une tile, comme le ferait le jeu au chargement.

use crate::tuile::{LecteurV, QUANT};

pub struct BatLu {
    pub id: u64,
    pub nature: u8,
    pub usage1: u8,
    pub usage2: u8,
    pub leger: bool,
    pub hauteur_dm: Option<u32>,
    pub etages: Option<u32>,
    pub logements: Option<u32>,
    /// Polygones en coordonnées locales centrées (m, f32 comme C-01).
    pub polys: Vec<Vec<Vec<[f32; 2]>>>,
}

impl BatLu {
    pub fn emprise(&self) -> f64 {
        let mut a = 0.0;
        for p in &self.polys {
            for anneau in p {
                let k = anneau.len();
                for i in 0..k {
                    let (u, v) = (anneau[i], anneau[(i + 1) % k]);
                    a += u[0] as f64 * v[1] as f64 - v[0] as f64 * u[1] as f64;
                }
            }
        }
        (a / 2.0).abs()
    }
    /// Capacité recalculée au chargement (P-05 : 35 m² par colon logé,
    /// 20 m² par poste de travail) : niveaux = étages, sinon hauteur / 3 m.
    pub fn capacite(&self) -> (u32, u32) {
        let niv = match (self.etages, self.hauteur_dm) {
            (Some(e), _) if e > 0 => e as f64,
            (_, Some(h)) => (h as f64 / 30.0).round().max(1.0),
            _ => 1.0,
        };
        let plancher = self.emprise() * niv;
        if self.usage1 == 2 || self.leger {
            return (0, 0);
        }
        let loge = self.usage1 == 0 || self.logements.unwrap_or(0) > 0;
        let travail = matches!(self.usage1, 1 | 3 | 4) && plancher >= 100.0 && !loge;
        ((if loge { plancher / 35.0 } else { 0.0 }) as u32, (if travail { plancher / 20.0 } else { 0.0 }) as u32)
    }
}

fn poly(l: &mut LecteurV) -> Vec<Vec<[f32; 2]>> {
    let na = l.u() as usize;
    (0..na)
        .map(|_| {
            let n = l.u() as usize;
            let (mut x, mut y) = (0i64, 0i64);
            (0..n)
                .map(|_| {
                    x += l.i();
                    y += l.i();
                    [(x as f64 / QUANT) as f32, (y as f64 / QUANT) as f32]
                })
                .collect()
        })
        .collect()
}

pub fn bati(b: &[u8]) -> Vec<BatLu> {
    let mut l = LecteurV::new(b);
    let n = l.u() as usize;
    let mut out = Vec::with_capacity(n);
    let mut id = 0u64;
    for _ in 0..n {
        id += l.u();
        let nature = l.o();
        let usage1 = l.o();
        let usage2 = l.o();
        let f = l.o();
        let hauteur_dm = (f & 2 != 0).then(|| l.u() as u32);
        let etages = (f & 4 != 0).then(|| l.u() as u32);
        let logements = (f & 8 != 0).then(|| l.u() as u32);
        let np = l.u() as usize;
        let polys = (0..np).map(|_| poly(&mut l)).collect();
        out.push(BatLu { id, nature, usage1, usage2, leger: f & 1 != 0, hauteur_dm, etages, logements, polys });
    }
    out
}

pub struct MorceauLu {
    pub id: u64,
    pub rang: u16,
    pub nature: u8,
    pub importance: u8,
    pub pos_sol: i8,
    pub poids_dt: u16,
    pub largeur_dm: u16,
    pub pts: Vec<[f32; 3]>,
    pub ferre: bool,
}

fn ligne(l: &mut LecteurV, z: bool) -> Vec<[f32; 3]> {
    let n = l.u() as usize;
    let (mut x, mut y, mut zz) = (0i64, 0i64, 0i64);
    (0..n)
        .map(|_| {
            x += l.i();
            y += l.i();
            if z {
                zz += l.i();
            }
            [(x as f64 / QUANT) as f32, (y as f64 / QUANT) as f32, zz as f32 / 10.0]
        })
        .collect()
}

pub fn reseau(b: &[u8]) -> Vec<MorceauLu> {
    let mut l = LecteurV::new(b);
    let mut out = Vec::new();
    let n = l.u() as usize;
    let mut id = 0i64;
    for _ in 0..n {
        id += l.i();
        let rang = l.u() as u16;
        let nature = l.o();
        let importance = l.o();
        let pos_sol = l.o() as i8;
        let largeur_dm = l.u() as u16;
        let _voies = l.o();
        let _sens = l.o();
        let _acces = l.o();
        let _vitesse = l.o();
        let poids_dt = l.u() as u16;
        let _h = l.u();
        let _lr = l.u();
        let pts = ligne(&mut l, pos_sol != 0);
        out.push(MorceauLu { id: id as u64, rang, nature, importance, pos_sol, poids_dt, largeur_dm, pts, ferre: false });
    }
    let n = l.u() as usize;
    let mut id = 0i64;
    for _ in 0..n {
        id += l.i();
        let rang = l.u() as u16;
        let nature = l.o();
        let pos_sol = l.o() as i8;
        let _el = l.o();
        let _v = l.o();
        let pts = ligne(&mut l, pos_sol != 0);
        out.push(MorceauLu { id: id as u64, rang, nature, importance: 0, pos_sol, poids_dt: 0, largeur_dm: 0, pts, ferre: true });
    }
    out
}

pub enum GeomLue {
    Vide,
    Point([f32; 2]),
    Lignes(Vec<Vec<[f32; 3]>>),
    Polygones(Vec<Vec<Vec<[f32; 2]>>>),
}

pub fn geom(l: &mut LecteurV) -> GeomLue {
    match l.o() {
        0 => GeomLue::Vide,
        1 => {
            let x = l.i();
            let y = l.i();
            GeomLue::Point([(x as f64 / QUANT) as f32, (y as f64 / QUANT) as f32])
        }
        2 => {
            let n = l.u() as usize;
            GeomLue::Lignes((0..n).map(|_| ligne(l, false)).collect())
        }
        _ => {
            let n = l.u() as usize;
            GeomLue::Polygones((0..n).map(|_| poly(l)).collect())
        }
    }
}

pub struct PontLu {
    pub id: u64,
    pub franchit: u8,
    pub geom: GeomLue,
}

pub fn ponts(b: &[u8]) -> Vec<PontLu> {
    let mut l = LecteurV::new(b);
    let n = l.u() as usize;
    (0..n)
        .map(|_| {
            let id = l.u();
            let _long = l.u();
            let _larg = l.u();
            let _imp = l.o();
            let _nat = l.o();
            let _poids = l.u();
            let franchit = l.o();
            let _r = l.u();
            let _v = l.u();
            let _t = l.s();
            PontLu { id, franchit, geom: geom(&mut l) }
        })
        .collect()
}

/// Zone : (bitmap des cases exclues, ou None ; cases percées).
pub fn zone(b: &[u8]) -> (Option<Vec<u8>>, Vec<u32>) {
    let a = b[0] == 1;
    let (ex, mut p) = if a { (Some(b[1..1 + 31_256].to_vec()), 1 + 31_256) } else { (None, 1) };
    let mut l = LecteurV { b, p };
    let n = l.u() as usize;
    let mut k = 0u32;
    let cases = (0..n)
        .map(|_| {
            k += l.u() as u32;
            k
        })
        .collect();
    p = l.p;
    let _ = p;
    (ex, cases)
}

/// Fragments : (code INSEE, cases de 2 m, habitants).
pub fn communes(b: &[u8]) -> Vec<(u32, u32, u32)> {
    let mut l = LecteurV::new(b);
    let n = l.u() as usize;
    (0..n).map(|_| (l.u() as u32, l.u() as u32, l.u() as u32)).collect()
}
