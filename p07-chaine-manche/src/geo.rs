//! Projections et grille des tiles.
//!
//! - **Lambert-93** (EPSG 2154) : conique conforme sécante, copiée de P-06
//!   (validée contre PROJ). C'est la grille des tiles en métropole (C-01).
//! - **ETRS89-LAEA** (EPSG 3035) : azimutale équivalente de Lambert, grille
//!   européenne INSPIRE des carreaux de l'INSEE depuis Filosofi 2017. Formules
//!   ellipsoïdales de Snyder (1987, p. 187-190), validées contre `cs2cs`
//!   (mode `verif-geo`).
//!
//! RGF93 et ETRS89 sont confondus au centimètre près en métropole : aucune
//! transformation de datum (à vérifier pour l'outre-mer, sans objet ici).
//!
//! Déterminisme : ces fonctions utilisent les fonctions transcendantes de la
//! bibliothèque du système (`sin`, `powf`…). Sur une même machine, les
//! résultats sont identiques d'une exécution à l'autre ; entre plateformes,
//! il faudrait une bibliothèque mathématique en Rust pur (A-12, P-18).

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

/// GRS80.
const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_222_101;
const E2: f64 = F * (2.0 - F);

/// Côté d'une tile (m).
pub const TILE: f64 = 1000.0;

/// Identifiant d'une tile de métropole : (ix, iy) en km Lambert-93. La zone
/// de projection (C-01) est implicite ici (2154) ; elle occupe un octet dans
/// l'en-tête des fichiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub ix: i32,
    pub iy: i32,
}

impl TileId {
    pub fn de(x: f64, y: f64) -> TileId {
        TileId { ix: (x / TILE).floor() as i32, iy: (y / TILE).floor() as i32 }
    }
    pub fn x0(&self) -> f64 {
        self.ix as f64 * TILE
    }
    pub fn y0(&self) -> f64 {
        self.iy as f64 * TILE
    }
    /// Centre de la tile : origine des positions locales f32 centrées (C-01).
    pub fn centre(&self) -> (f64, f64) {
        (self.x0() + 500.0, self.y0() + 500.0)
    }
    pub fn nom(&self) -> String {
        format!("x{}_y{}", self.ix, self.iy)
    }
}

// ------------------------------------------------------------ Lambert-93

fn e() -> f64 {
    E2.sqrt()
}

fn lcc_m(phi: f64) -> f64 {
    let s = phi.sin();
    phi.cos() / (1.0 - E2 * s * s).sqrt()
}

fn lcc_t(phi: f64) -> f64 {
    let s = phi.sin();
    let e = e();
    (FRAC_PI_4 - phi / 2.0).tan() / ((1.0 - e * s) / (1.0 + e * s)).powf(e / 2.0)
}

pub struct Lambert93 {
    n: f64,
    af: f64,
    rho0: f64,
}

const L93_LON0: f64 = 3.0;
const L93_X0: f64 = 700_000.0;
const L93_Y0: f64 = 6_600_000.0;

impl Lambert93 {
    pub fn new() -> Self {
        let (p0, p1, p2) = (46.5f64.to_radians(), 44.0f64.to_radians(), 49.0f64.to_radians());
        let (m1, m2) = (lcc_m(p1), lcc_m(p2));
        let (t0, t1, t2) = (lcc_t(p0), lcc_t(p1), lcc_t(p2));
        let n = (m1.ln() - m2.ln()) / (t1.ln() - t2.ln());
        let f = m1 / (n * t1.powf(n));
        Lambert93 { n, af: A * f, rho0: A * f * t0.powf(n) }
    }

    /// (lat, lon) en degrés → (x, y) en mètres.
    pub fn avant(&self, lat: f64, lon: f64) -> (f64, f64) {
        let rho = self.af * lcc_t(lat.to_radians()).powf(self.n);
        let th = self.n * (lon - L93_LON0).to_radians();
        (L93_X0 + rho * th.sin(), L93_Y0 + self.rho0 - rho * th.cos())
    }

    pub fn inverse(&self, x: f64, y: f64) -> (f64, f64) {
        let (dx, dy) = (x - L93_X0, self.rho0 - (y - L93_Y0));
        let rho = (dx * dx + dy * dy).sqrt();
        let th = dx.atan2(dy);
        let t = (rho / self.af).powf(1.0 / self.n);
        let e = e();
        let mut phi = FRAC_PI_2 - 2.0 * t.atan();
        for _ in 0..15 {
            let s = phi.sin();
            phi = FRAC_PI_2 - 2.0 * (t * ((1.0 - e * s) / (1.0 + e * s)).powf(e / 2.0)).atan();
        }
        (phi.to_degrees(), L93_LON0 + (th / self.n).to_degrees())
    }
}

// ------------------------------------------------------------ LAEA (3035)

pub struct Laea {
    sin_b1: f64,
    cos_b1: f64,
    rq: f64,
    d: f64,
    qp: f64,
}

const LAEA_LAT0: f64 = 52.0;
const LAEA_LON0: f64 = 10.0;
const LAEA_X0: f64 = 4_321_000.0;
const LAEA_Y0: f64 = 3_210_000.0;

fn q(phi: f64) -> f64 {
    let e = e();
    let s = phi.sin();
    (1.0 - E2) * (s / (1.0 - E2 * s * s) - (1.0 / (2.0 * e)) * ((1.0 - e * s) / (1.0 + e * s)).ln())
}

impl Laea {
    pub fn new() -> Self {
        let p1 = LAEA_LAT0.to_radians();
        let qp = q(FRAC_PI_2);
        let q1 = q(p1);
        let rq = A * (qp / 2.0).sqrt();
        let b1 = (q1 / qp).asin();
        let m1 = lcc_m(p1);
        let d = A * m1 / (rq * b1.cos());
        Laea { sin_b1: b1.sin(), cos_b1: b1.cos(), rq, d, qp }
    }

    pub fn avant(&self, lat: f64, lon: f64) -> (f64, f64) {
        let b = (q(lat.to_radians()) / self.qp).asin();
        let dl = (lon - LAEA_LON0).to_radians();
        let bb = self.rq * (2.0 / (1.0 + self.sin_b1 * b.sin() + self.cos_b1 * b.cos() * dl.cos())).sqrt();
        let x = LAEA_X0 + bb * self.d * b.cos() * dl.sin();
        let y = LAEA_Y0 + (bb / self.d) * (self.cos_b1 * b.sin() - self.sin_b1 * b.cos() * dl.cos());
        (x, y)
    }

    pub fn inverse(&self, x: f64, y: f64) -> (f64, f64) {
        let (dx, dy) = (x - LAEA_X0, y - LAEA_Y0);
        let rho = ((dx / self.d).powi(2) + (self.d * dy).powi(2)).sqrt();
        if rho < 1e-9 {
            return (LAEA_LAT0, LAEA_LON0);
        }
        let ce = 2.0 * (rho / (2.0 * self.rq)).asin();
        let qq = self.qp * (ce.cos() * self.sin_b1 + self.d * dy * ce.sin() * self.cos_b1 / rho);
        let lon = LAEA_LON0
            + (dx * ce.sin())
                .atan2(self.d * rho * self.cos_b1 * ce.cos() - self.d * self.d * dy * self.sin_b1 * ce.sin())
                .to_degrees();
        // φ par itération (Snyder 3-16).
        let e = e();
        let mut phi = (qq / 2.0).asin();
        for _ in 0..20 {
            let s = phi.sin();
            let un = 1.0 - E2 * s * s;
            phi += un * un / (2.0 * phi.cos())
                * (qq / (1.0 - E2) - s / un + (1.0 / (2.0 * e)) * ((1.0 - e * s) / (1.0 + e * s)).ln());
        }
        (phi.to_degrees(), lon)
    }
}

/// Passage LAEA → Lambert-93 (via géographiques).
pub struct Laea2L93 {
    pub laea: Laea,
    pub l93: Lambert93,
}

impl Laea2L93 {
    pub fn new() -> Self {
        Laea2L93 { laea: Laea::new(), l93: Lambert93::new() }
    }
    pub fn conv(&self, x: f64, y: f64) -> (f64, f64) {
        let (lat, lon) = self.laea.inverse(x, y);
        self.l93.avant(lat, lon)
    }
}
