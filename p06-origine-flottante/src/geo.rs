//! Géodésie : ellipsoïde GRS80, géocentrique (ECEF), repères locaux, projections
//! des territoires (Lambert conique conforme, Mercator transverse / UTM), tiles
//! de 1 km et leur repère affine.
//!
//! Tous les calculs sont en f64. Les projections sont validées contre PROJ
//! (`cs2cs`) par le script de campagne.

use bevy::math::{DAffine3, DMat3, DQuat, DVec2, DVec3};

pub const A: f64 = 6_378_137.0;
pub const F: f64 = 1.0 / 298.257_222_101;
pub const E2: f64 = F * (2.0 - F);
pub const TILE: f64 = 1000.0;

/// Latitude, longitude en degrés, hauteur ellipsoïdale en mètres.
#[derive(Clone, Copy, Debug)]
pub struct Geo {
    pub lat: f64,
    pub lon: f64,
    pub h: f64,
}

pub fn geo(lat: f64, lon: f64) -> Geo {
    Geo { lat, lon, h: 0.0 }
}

pub fn vers_ecef(g: Geo) -> DVec3 {
    let (sp, cp) = g.lat.to_radians().sin_cos();
    let (sl, cl) = g.lon.to_radians().sin_cos();
    let n = A / (1.0 - E2 * sp * sp).sqrt();
    DVec3::new((n + g.h) * cp * cl, (n + g.h) * cp * sl, (n * (1.0 - E2) + g.h) * sp)
}

pub fn depuis_ecef(p: DVec3) -> Geo {
    let lon = p.y.atan2(p.x);
    let r = (p.x * p.x + p.y * p.y).sqrt();
    let mut lat = p.z.atan2(r * (1.0 - E2));
    let mut h = 0.0;
    for _ in 0..6 {
        let s = lat.sin();
        let n = A / (1.0 - E2 * s * s).sqrt();
        h = r / lat.cos() - n;
        lat = p.z.atan2(r * (1.0 - E2 * n / (n + h)));
    }
    Geo { lat: lat.to_degrees(), lon: lon.to_degrees(), h }
}

/// Normale à l'ellipsoïde (verticale locale).
pub fn verticale(g: Geo) -> DVec3 {
    let (sp, cp) = g.lat.to_radians().sin_cos();
    let (sl, cl) = g.lon.to_radians().sin_cos();
    DVec3::new(cp * cl, cp * sl, sp)
}

/// Rotation « axes Bevy depuis ECEF » au point g : x = est, y = haut, z = sud.
pub fn bevy_depuis_ecef(g: Geo) -> DMat3 {
    let (sp, cp) = g.lat.to_radians().sin_cos();
    let (sl, cl) = g.lon.to_radians().sin_cos();
    let e = DVec3::new(-sl, cl, 0.0);
    let n = DVec3::new(-sp * cl, -sp * sl, cp);
    let u = DVec3::new(cp * cl, cp * sl, sp);
    DMat3::from_cols(e, u, -n).transpose()
}

/// Axes Bevy fixés au géocentre : x = X, y = Z (pôle nord en haut), z = −Y.
pub fn bevy_depuis_ecef_fixe() -> DMat3 {
    DMat3::from_cols(DVec3::X, -DVec3::Z, DVec3::Y)
}

pub fn distance_gc(a: Geo, b: Geo) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dl = (b.lon - a.lon).to_radians();
    let h = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * 6_371_008.8 * h.sqrt().asin()
}

// ------------------------------------------------------------------ Projections

#[derive(Clone, Copy, Debug)]
pub enum Projection {
    /// Lambert conique conforme sécant.
    Lcc { lat0: f64, lon0: f64, lat1: f64, lat2: f64, x0: f64, y0: f64 },
    /// Mercator transverse (série de Krüger à l'ordre 6).
    Tm { lon0: f64, k0: f64, x0: f64, y0: f64 },
}

pub fn utm(zone: u32, sud: bool) -> Projection {
    Projection::Tm {
        lon0: -183.0 + 6.0 * zone as f64,
        k0: 0.9996,
        x0: 500_000.0,
        y0: if sud { 10_000_000.0 } else { 0.0 },
    }
}

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
    (std::f64::consts::FRAC_PI_4 - phi / 2.0).tan() / ((1.0 - e * s) / (1.0 + e * s)).powf(e / 2.0)
}

struct LccK {
    n: f64,
    af: f64,
    rho0: f64,
}

fn lcc_k(lat0: f64, lat1: f64, lat2: f64) -> LccK {
    let (p0, p1, p2) = (lat0.to_radians(), lat1.to_radians(), lat2.to_radians());
    let (m1, m2) = (lcc_m(p1), lcc_m(p2));
    let (t0, t1, t2) = (lcc_t(p0), lcc_t(p1), lcc_t(p2));
    let n = (m1.ln() - m2.ln()) / (t1.ln() - t2.ln());
    let f = m1 / (n * t1.powf(n));
    LccK { n, af: A * f, rho0: A * f * t0.powf(n) }
}

// Coefficients de Krüger (n³ et au-delà).
fn kruger() -> (f64, [f64; 6], [f64; 6]) {
    let n = F / (2.0 - F);
    let (n2, n3, n4, n5, n6) = (n * n, n.powi(3), n.powi(4), n.powi(5), n.powi(6));
    let aa = A / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0 + n6 / 256.0);
    let al = [
        n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0 - 127.0 * n5 / 288.0 + 7891.0 * n6 / 37800.0,
        13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0 + 281.0 * n5 / 630.0 - 1983433.0 * n6 / 1935360.0,
        61.0 * n3 / 240.0 - 103.0 * n4 / 140.0 + 15061.0 * n5 / 26880.0 + 167603.0 * n6 / 181440.0,
        49561.0 * n4 / 161280.0 - 179.0 * n5 / 168.0 + 6601661.0 * n6 / 7257600.0,
        34729.0 * n5 / 80640.0 - 3418889.0 * n6 / 1995840.0,
        212378941.0 * n6 / 319334400.0,
    ];
    let be = [
        n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0 - 81.0 * n5 / 512.0 + 96199.0 * n6 / 604800.0,
        n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0 + 46.0 * n5 / 105.0 - 1118711.0 * n6 / 3870720.0,
        17.0 * n3 / 480.0 - 37.0 * n4 / 840.0 - 209.0 * n5 / 4480.0 + 5569.0 * n6 / 90720.0,
        4397.0 * n4 / 161280.0 - 11.0 * n5 / 504.0 - 830251.0 * n6 / 7257600.0,
        4583.0 * n5 / 161280.0 - 108847.0 * n6 / 3991680.0,
        20648693.0 * n6 / 638668800.0,
    ];
    (aa, al, be)
}

impl Projection {
    /// Géographique → grille (est, nord), en mètres.
    pub fn avant(&self, g: Geo) -> DVec2 {
        match *self {
            Projection::Lcc { lat0, lon0, lat1, lat2, x0, y0 } => {
                let k = lcc_k(lat0, lat1, lat2);
                let rho = k.af * lcc_t(g.lat.to_radians()).powf(k.n);
                let th = k.n * (g.lon - lon0).to_radians();
                DVec2::new(x0 + rho * th.sin(), y0 + k.rho0 - rho * th.cos())
            }
            Projection::Tm { lon0, k0, x0, y0 } => {
                let (aa, al, _) = kruger();
                let e = e();
                let phi = g.lat.to_radians();
                let mut dl = (g.lon - lon0).to_radians();
                // Ramène dans ]−π, π] (antiméridien).
                while dl > std::f64::consts::PI {
                    dl -= 2.0 * std::f64::consts::PI;
                }
                while dl <= -std::f64::consts::PI {
                    dl += 2.0 * std::f64::consts::PI;
                }
                let s = phi.sin();
                let t = (s.atanh() - e * (e * s).atanh()).sinh();
                let xi_p = t.atan2(dl.cos());
                let eta_p = (dl.sin() / (1.0 + t * t).sqrt()).atanh();
                let mut xi = xi_p;
                let mut eta = eta_p;
                for j in 0..6 {
                    let k2 = 2.0 * (j + 1) as f64;
                    xi += al[j] * (k2 * xi_p).sin() * (k2 * eta_p).cosh();
                    eta += al[j] * (k2 * xi_p).cos() * (k2 * eta_p).sinh();
                }
                DVec2::new(x0 + k0 * aa * eta, y0 + k0 * aa * xi)
            }
        }
    }

    /// Grille → géographique (hauteur nulle).
    pub fn inverse(&self, p: DVec2) -> Geo {
        match *self {
            Projection::Lcc { lat0, lon0, lat1, lat2, x0, y0 } => {
                let k = lcc_k(lat0, lat1, lat2);
                let (dx, dy) = (p.x - x0, k.rho0 - (p.y - y0));
                let sgn = k.n.signum();
                let rho = sgn * (dx * dx + dy * dy).sqrt();
                let th = (sgn * dx).atan2(sgn * dy);
                let t = (rho / k.af).powf(1.0 / k.n);
                let e = e();
                let mut phi = std::f64::consts::FRAC_PI_2 - 2.0 * t.atan();
                for _ in 0..15 {
                    let s = phi.sin();
                    phi = std::f64::consts::FRAC_PI_2 - 2.0 * (t * ((1.0 - e * s) / (1.0 + e * s)).powf(e / 2.0)).atan();
                }
                Geo { lat: phi.to_degrees(), lon: lon0 + (th / k.n).to_degrees(), h: 0.0 }
            }
            Projection::Tm { lon0, k0, x0, y0 } => {
                let (aa, _, be) = kruger();
                let e = e();
                let xi = (p.y - y0) / (k0 * aa);
                let eta = (p.x - x0) / (k0 * aa);
                let mut xi_p = xi;
                let mut eta_p = eta;
                for j in 0..6 {
                    let k2 = 2.0 * (j + 1) as f64;
                    xi_p -= be[j] * (k2 * xi).sin() * (k2 * eta).cosh();
                    eta_p -= be[j] * (k2 * xi).cos() * (k2 * eta).sinh();
                }
                let tau_p = xi_p.sin() / (eta_p.sinh().powi(2) + xi_p.cos().powi(2)).sqrt();
                let dl = eta_p.sinh().atan2(xi_p.cos());
                // τ depuis τ' (Newton, Karney 2011).
                let mut tau = tau_p;
                for _ in 0..8 {
                    let sig = (e * (e * tau / (1.0 + tau * tau).sqrt()).atanh()).sinh();
                    let tp = tau * (1.0 + sig * sig).sqrt() - sig * (1.0 + tau * tau).sqrt();
                    let d = (tau_p - tp) / (1.0 + tp * tp).sqrt() * (1.0 + (1.0 - E2) * tau * tau)
                        / ((1.0 - E2) * (1.0 + tau * tau).sqrt());
                    tau += d;
                    if d.abs() < 1e-15 {
                        break;
                    }
                }
                let mut lon = lon0 + dl.to_degrees();
                if lon > 180.0 {
                    lon -= 360.0;
                }
                if lon <= -180.0 {
                    lon += 360.0;
                }
                Geo { lat: tau.atan().to_degrees(), lon, h: 0.0 }
            }
        }
    }

    /// Grille + hauteur → ECEF exact.
    pub fn grille_vers_ecef(&self, p: DVec2, h: f64) -> DVec3 {
        let mut g = self.inverse(p);
        g.h = h;
        vers_ecef(g)
    }
}

// ------------------------------------------------------------------ Zones

#[derive(Clone, Copy, Debug)]
pub struct Zone {
    pub nom: &'static str,
    pub epsg: u32,
    pub proj: Projection,
}

pub fn zones() -> Vec<Zone> {
    let z = |nom, epsg, proj| Zone { nom, epsg, proj };
    vec![
        z(
            "Lambert-93",
            2154,
            Projection::Lcc { lat0: 46.5, lon0: 3.0, lat1: 49.0, lat2: 44.0, x0: 700_000.0, y0: 6_600_000.0 },
        ),
        z("UTM 20N (Antilles)", 5490, utm(20, false)),
        z("UTM 22N (Guyane)", 2972, utm(22, false)),
        z("UTM 40S (Réunion)", 2975, utm(40, true)),
        z("UTM 38S (Mayotte)", 4471, utm(38, true)),
        z("UTM 21N (Saint-Pierre-et-Miquelon)", 4467, utm(21, false)),
        z("UTM 5S (Polynésie ouest)", 3296, utm(5, true)),
        z("UTM 6S (Polynésie, Tahiti)", 3297, utm(6, true)),
        z("UTM 7S (Polynésie, Marquises)", 3298, utm(7, true)),
        z("UTM 8S (Polynésie, Gambier)", 3299, utm(8, true)),
        z(
            "Lambert Nouvelle-Calédonie",
            3163,
            Projection::Lcc {
                lat0: -21.5,
                lon0: 166.0,
                lat1: -20.0 - 2.0 / 3.0,
                lat2: -22.0 - 1.0 / 3.0,
                x0: 400_000.0,
                y0: 300_000.0,
            },
        ),
        z("UTM 1S (Wallis-et-Futuna)", 8903, utm(1, true)),
        z("UTM 42S (Kerguelen)", 7079, utm(42, true)),
        z("UTM 39S (Crozet)", 7076, utm(39, true)),
        z("UTM 43S (Amsterdam, Saint-Paul)", 7080, utm(43, true)),
    ]
}

pub fn zone_par_epsg(epsg: u32) -> Zone {
    zones().into_iter().find(|z| z.epsg == epsg).expect("zone inconnue")
}

#[derive(Clone, Copy, Debug)]
pub struct Lieu {
    pub nom: &'static str,
    pub g: Geo,
    pub epsg: u32,
}

/// Lieux de référence (coordonnées approchées, pour poser les scènes).
pub fn lieux() -> Vec<Lieu> {
    let l = |nom, lat, lon, epsg| Lieu { nom, g: geo(lat, lon), epsg };
    vec![
        l("Paris", 48.8530, 2.3499, 2154),
        l("Brest", 48.3904, -4.4861, 2154),
        l("Ouessant", 48.4584, -5.0880, 2154),
        l("Bray-Dunes", 51.0870, 2.5440, 2154),
        l("Lauterbourg", 48.9750, 8.1780, 2154),
        l("Bonifacio", 41.3870, 9.1590, 2154),
        l("Fort-de-France", 14.6037, -61.0730, 5490),
        l("Pointe-à-Pitre", 16.2410, -61.5330, 5490),
        l("Saint-Martin", 18.0700, -63.0830, 5490),
        l("Cayenne", 4.9220, -52.3130, 2972),
        l("Saint-Denis (Réunion)", -20.8820, 55.4500, 2975),
        l("Mamoudzou", -12.7800, 45.2280, 4471),
        l("Saint-Pierre", 46.7800, -56.1750, 4467),
        l("Papeete", -17.5350, -149.5696, 3297),
        l("Bora-Bora", -16.5000, -151.7400, 3296),
        l("Nuku Hiva", -8.9100, -140.1000, 3298),
        l("Rapa", -27.6100, -144.3300, 3297),
        l("Mangareva", -23.1200, -134.9700, 3299),
        l("Nouméa", -22.2758, 166.4580, 3163),
        l("Île des Pins", -22.6200, 167.4800, 3163),
        l("Île Walpole", -22.6000, 168.9500, 3163),
        l("Chesterfield", -19.9000, 158.3000, 3163),
        l("Mata-Utu", -13.2825, -176.1745, 8903),
        l("Futuna", -14.2933, -178.1583, 8903),
        l("Port-aux-Français", -49.3500, 70.2200, 7079),
        l("Crozet", -46.4300, 51.8600, 7076),
        l("Amsterdam", -37.8300, 77.5500, 7080),
    ]
}

pub fn lieu(nom: &str) -> Lieu {
    lieux().into_iter().find(|l| l.nom == nom).unwrap_or_else(|| panic!("lieu inconnu : {nom}"))
}

// ------------------------------------------------------------------ Tiles

/// Identifiant d'une tile : zone de projection et indices de la grille de 1 km.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub epsg: u32,
    pub ix: i32,
    pub iy: i32,
}

impl TileId {
    pub fn de_grille(epsg: u32, p: DVec2) -> TileId {
        TileId { epsg, ix: (p.x / TILE).floor() as i32, iy: (p.y / TILE).floor() as i32 }
    }
    pub fn coin(&self) -> DVec2 {
        DVec2::new(self.ix as f64 * TILE, self.iy as f64 * TILE)
    }
    pub fn voisine(&self, dx: i32, dy: i32) -> TileId {
        TileId { epsg: self.epsg, ix: self.ix + dx, iy: self.iy + dy }
    }
}

/// Repère affine d'une tile : coordonnées locales « Bevy » de la tile
/// (x = est de la grille depuis le coin sud-ouest, y = hauteur, z = −nord) vers
/// l'ECEF. Plan tangent au centre de la tile : jacobien de la projection
/// inverse et verticale de l'ellipsoïde.
#[derive(Clone, Copy, Debug)]
pub struct RepereTile {
    pub id: TileId,
    pub ecef_depuis_local: DAffine3,
    /// Rotation pure (orthonormalisée) local → ECEF.
    pub rotation: DQuat,
    /// Facteur d'échelle de la projection au centre (grille / terrain).
    pub k: f64,
    /// Termes du second ordre (dx², dx·dy, dy²) de la projection inverse au
    /// centre : corrigent la courbure (39 mm aux coins) et la variation de k.
    pub quad: [DVec3; 3],
}

impl RepereTile {
    /// Local → ECEF au second ordre (Taylor au centre de la tile).
    pub fn vers_ecef(&self, local: DVec3) -> DVec3 {
        let p = self.ecef_depuis_local.transform_point3(local);
        let (dx, dy) = (local.x - TILE / 2.0, -local.z - TILE / 2.0);
        p + 0.5 * (self.quad[0] * dx * dx + 2.0 * self.quad[1] * dx * dy + self.quad[2] * dy * dy)
    }
}

pub fn repere_tile(z: &Zone, id: TileId) -> RepereTile {
    let c = id.coin() + DVec2::splat(TILE / 2.0);
    let d = 0.5;
    let p0 = z.proj.grille_vers_ecef(c, 0.0);
    let dx = (z.proj.grille_vers_ecef(c + DVec2::new(d, 0.0), 0.0) - z.proj.grille_vers_ecef(c - DVec2::new(d, 0.0), 0.0)) / (2.0 * d);
    let dy = (z.proj.grille_vers_ecef(c + DVec2::new(0.0, d), 0.0) - z.proj.grille_vers_ecef(c - DVec2::new(0.0, d), 0.0)) / (2.0 * d);
    let up = verticale(z.proj.inverse(c));
    // Local (x, y, z) = (gx, h, −gy) : colonnes ∂/∂x = dx, ∂/∂y = up, ∂/∂z = −dy.
    let m = DMat3::from_cols(dx, up, -dy);
    let ecef_depuis_local = DAffine3 { matrix3: m, translation: p0 - m * DVec3::new(TILE / 2.0, 0.0, -TILE / 2.0) };
    let ex = dx.normalize();
    let ez = ex.cross(up).normalize(); // = −nord orthonormalisé
    let rot = DMat3::from_cols(ex, up, ez);
    // Dérivées secondes par différences finies (pas de 100 m).
    let s = 100.0;
    let fq = |a: f64, b: f64| z.proj.grille_vers_ecef(c + DVec2::new(a, b), 0.0);
    let qxx = (fq(s, 0.0) - 2.0 * p0 + fq(-s, 0.0)) / (s * s);
    let qyy = (fq(0.0, s) - 2.0 * p0 + fq(0.0, -s)) / (s * s);
    let qxy = (fq(s, s) - fq(s, -s) - fq(-s, s) + fq(-s, -s)) / (4.0 * s * s);
    RepereTile { id, ecef_depuis_local, rotation: DQuat::from_mat3(&rot), k: 1.0 / dx.length(), quad: [qxx, qxy, qyy] }
}

/// Position exacte d'un point local de tile (projection inverse complète).
pub fn local_vers_ecef_exact(z: &Zone, id: TileId, local: DVec3) -> DVec3 {
    let g = id.coin() + DVec2::new(local.x, -local.z);
    z.proj.grille_vers_ecef(g, local.y)
}

/// Repère local Bevy (est, haut, sud) en un point géographique.
pub fn pose_locale(g: Geo) -> (DVec3, DQuat) {
    let r = bevy_depuis_ecef(g).transpose(); // local → ECEF
    (vers_ecef(g), DQuat::from_mat3(&r))
}

pub fn ulp32(x: f64) -> f64 {
    let f = x.abs() as f32;
    if f == 0.0 {
        return f32::MIN_POSITIVE as f64;
    }
    (f32::from_bits(f.to_bits() + 1) - f) as f64
}

pub fn ulp64(x: f64) -> f64 {
    let f = x.abs();
    f64::from_bits(f.to_bits() + 1) - f
}
