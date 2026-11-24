//! Analyses sans rendu : géodésie des territoires, horloge des shaders.

use bevy::math::{DVec2, DVec3};

use crate::geo::{self, TileId};
use crate::stats::f;

/// Une ligne par lieu : grandeurs qui fixent la précision selon le repère.
pub fn geo_csv(entete: bool) {
    if entete {
        println!(
            "lieu,zone,epsg,lat,lon,est,nord,dist_paris_km,corde_paris_km,ulp32_naif_paris_m,ecef_max_m,ulp32_ecef_m,grille_max_m,ulp32_grille_m,ulp64_grille_m,k,aller_retour_mm,plat_vertical_mm,plat_horizontal_mm,jointure_mm,quad_ecart_mm,quad_jointure_mm,quantif_164_mm"
        );
    }
    let paris = geo::lieu("Paris").g;
    let p_paris = geo::vers_ecef(paris);
    let r_paris = geo::bevy_depuis_ecef(paris);
    for l in geo::lieux() {
        let z = geo::zone_par_epsg(l.epsg);
        let g = z.proj.avant(l.g);
        let p = geo::vers_ecef(l.g);
        let enu = r_paris * (p - p_paris);
        let corde = (p - p_paris).length();
        let ulp_naif = geo::ulp32(enu.abs().max_element());
        let ecef_max = p.abs().max_element();
        let gmax = g.x.abs().max(g.y.abs());
        // Aller-retour géographique → grille → géographique.
        let g2 = z.proj.inverse(g);
        let ar = (geo::vers_ecef(g2) - p).length() * 1000.0;
        // Repère affine de la tile : écart au calcul exact et jointures.
        let id = TileId::de_grille(l.epsg, g);
        let rt = geo::repere_tile(&z, id);
        let up = geo::verticale(l.g);
        let (mut pv, mut ph, mut pq) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..=20 {
            for j in 0..=20 {
                let loc = DVec3::new(50.0 * i as f64, 0.0, -50.0 * j as f64);
                let ex = geo::local_vers_ecef_exact(&z, id, loc);
                let d = rt.ecef_depuis_local.transform_point3(loc) - ex;
                pq = pq.max((rt.vers_ecef(loc) - ex).length());
                let v = d.dot(up);
                pv = pv.max(v.abs());
                ph = ph.max((d - up * v).length());
            }
        }
        let (mut joint, mut jq) = (0.0f64, 0.0f64);
        for (dx, dy) in [(1, 0), (0, 1)] {
            let rv = geo::repere_tile(&z, id.voisine(dx, dy));
            for k in 0..=20 {
                let s = 50.0 * k as f64;
                let (a, b) = if dx == 1 {
                    (DVec3::new(1000.0, 0.0, -s), DVec3::new(0.0, 0.0, -s))
                } else {
                    (DVec3::new(s, 0.0, -1000.0), DVec3::new(s, 0.0, 0.0))
                };
                let e = (rt.ecef_depuis_local.transform_point3(a) - rv.ecef_depuis_local.transform_point3(b)).length();
                joint = joint.max(e);
                jq = jq.max((rt.vers_ecef(a) - rv.vers_ecef(b)).length());
            }
        }
        println!(
            "{},{},{},{:.4},{:.4},{:.1},{:.1},{:.0},{:.0},{},{:.0},{},{:.0},{},{},{:.6},{},{},{},{},{},{},{}",
            l.nom,
            z.nom.replace(',', ""),
            z.epsg,
            l.g.lat,
            l.g.lon,
            g.x,
            g.y,
            geo::distance_gc(paris, l.g) / 1000.0,
            corde / 1000.0,
            f(ulp_naif),
            ecef_max,
            f(geo::ulp32(ecef_max)),
            gmax,
            f(geo::ulp32(gmax)),
            f(geo::ulp64(gmax)),
            rt.k,
            f(ar),
            f(pv * 1000.0),
            f(ph * 1000.0),
            f(joint * 1000.0),
            f(pq * 1000.0),
            f(jq * 1000.0),
            f(1000.0 / 128.0),
        );
    }
}

/// Lieux pour la validation contre PROJ : « nom epsg lat lon est nord ».
pub fn geo_proj() {
    for l in geo::lieux() {
        let z = geo::zone_par_epsg(l.epsg);
        let g = z.proj.avant(l.g);
        println!("{}\t{}\t{:.9}\t{:.9}\t{:.4}\t{:.4}", l.nom.replace(' ', "_"), z.epsg, l.g.lat, l.g.lon, g.x, g.y);
    }
}

/// Point du territoire le plus éloigné de Paris.
pub fn plus_loin() -> (String, f64, f64) {
    let paris = geo::lieu("Paris").g;
    let p = geo::vers_ecef(paris);
    let mut best = (String::new(), 0.0, 0.0);
    for l in geo::lieux() {
        let d = geo::distance_gc(paris, l.g);
        let c = (geo::vers_ecef(l.g) - p).length();
        if d > best.1 {
            best = (l.nom.to_string(), d, c);
        }
    }
    best
}

// ------------------------------------------------------------------ Horloge des shaders

/// Représentations du temps envoyé aux shaders.
#[derive(Clone, Copy)]
enum Rep {
    /// f32 des secondes écoulées depuis le début de la partie.
    Absolu,
    /// Comme `globals.time` de Bevy : f32 du temps modulo 3 600 s.
    Bevy3600,
    /// Phase de chaque animation calculée sur le CPU depuis les ticks entiers,
    /// envoyée en f32 dans [0, 1).
    Phase,
}

fn nom(r: Rep) -> &'static str {
    match r {
        Rep::Absolu => "f32-absolu",
        Rep::Bevy3600 => "f32-modulo-3600",
        Rep::Phase => "phase-cpu",
    }
}

/// Temps exact à la frame k : ticks entiers + fraction (comme `Horloge`).
fn temps_exact(t0_ticks: u64, k: u64, acc: f64) -> (u64, f64) {
    let frames_ticks = k as f64 / 60.0 * acc * 20.0;
    let t = frames_ticks.floor();
    (t0_ticks + t as u64, frames_ticks - t)
}

/// Pour chaque durée de partie et chaque accélération : écart des animations
/// (défilement à 1 m/s sur une texture de 10 m, houle de 7 s et 0,5 m) entre
/// le calcul f32 du shader et le calcul exact, et frames figées.
pub fn temps_csv(entete: bool) {
    if entete {
        println!("representation,duree_j,acceleration,ulp_temps_s,defilement_max_mm,defilement_saut_max_mm,houle_max_mm,frames_figees_pct,saut_bouclage_mm,visible");
    }
    let px_m = 0.0124; // taille d'un pixel à la vue rapprochée (25 m, champ de 30°)
    let durees = [1.0 / 24.0, 1.0, 1.5, 7.0, 28.0, 42.0, 365.0, 3650.0];
    for rep in [Rep::Absolu, Rep::Bevy3600, Rep::Phase] {
        for acc in [1.0, 100.0] {
            for &dj in &durees {
                let t0 = (dj * 86400.0 * 20.0) as u64;
                let (mut dmax, mut sautmax, mut hmax) = (0.0f64, 0.0f64, 0.0f64);
                let mut figees = 0u32;
                let mut prec: Option<(f32, f64)> = None;
                let mut prec32 = f32::NAN;
                let n = 600u64;
                for k in 0..n {
                    let (ti, fr) = temps_exact(t0, k, acc);
                    let t = (ti as f64 + fr) / 20.0;
                    let (defil32, houle32) = match rep {
                        Rep::Absolu | Rep::Bevy3600 => {
                            let ts = match rep {
                                Rep::Absolu => t as f32,
                                _ => {
                                    let m = ti % (3600 * 20);
                                    ((m as f64 + fr) / 20.0) as f32
                                }
                            };
                            if ts == prec32 {
                                figees += 1;
                            }
                            prec32 = ts;
                            // Ce que ferait le shader, en f32.
                            let d = (ts * 0.1).fract() * 10.0;
                            let h = 0.5 * (ts * (std::f32::consts::TAU / 7.0)).sin();
                            (d, h)
                        }
                        Rep::Phase => {
                            // Phases exactes depuis les ticks (modulo entier de la période).
                            let pd = ((ti % 200) as f64 + fr) / 200.0; // 10 s
                            let ph = ((ti % 140) as f64 + fr) / 140.0; // 7 s
                            let (pd, ph) = (pd as f32, ph as f32);
                            if pd == prec32 {
                                figees += 1;
                            }
                            prec32 = pd;
                            (pd * 10.0, 0.5 * (ph * std::f32::consts::TAU).sin())
                        }
                    };
                    let d_ex = (t * 0.1).rem_euclid(1.0) * 10.0;
                    let h_ex = 0.5 * (t * std::f64::consts::TAU / 7.0).sin();
                    let mut ed = (defil32 as f64 - d_ex).abs();
                    if ed > 5.0 {
                        ed = 10.0 - ed; // bouclage de la texture
                    }
                    dmax = dmax.max(ed);
                    hmax = hmax.max((houle32 as f64 - h_ex).abs());
                    if let Some((p32, pex)) = prec {
                        let mut s = (defil32 as f64 - p32 as f64) - (d_ex - pex);
                        if s.abs() > 5.0 {
                            s = s.abs() - 10.0;
                        }
                        sautmax = sautmax.max(s.abs());
                    }
                    prec = Some((defil32, d_ex));
                }
                // Saut au bouclage de 3 600 s : la houle de 7 s n'y est pas périodique.
                let saut_bouclage = match rep {
                    Rep::Bevy3600 => {
                        let a = 0.5 * (3600.0f64 * std::f64::consts::TAU / 7.0).sin();
                        a.abs() * 1000.0
                    }
                    _ => 0.0,
                };
                let ulp = match rep {
                    Rep::Absolu => geo::ulp32(dj * 86400.0),
                    Rep::Bevy3600 => geo::ulp32(3599.0),
                    Rep::Phase => geo::ulp32(0.99) * 10.0,
                };
                let visible = (dmax.max(hmax) / px_m > 0.5) || saut_bouclage / 1000.0 / px_m > 0.5;
                println!(
                    "{},{},{},{},{},{},{},{:.1},{},{}",
                    nom(rep),
                    f(dj),
                    acc,
                    f(ulp),
                    f(dmax * 1000.0),
                    f(sautmax * 1000.0),
                    f(hmax * 1000.0),
                    100.0 * figees as f64 / n as f64,
                    f(saut_bouclage),
                    if visible { "oui" } else { "non" }
                );
            }
        }
    }
}

/// Seuil d'apparition : première durée (puissance de 2 de secondes) où le
/// défilement à 1 m/s d'une horloge f32 absolue s'écarte de plus d'un
/// demi-pixel (vue rapprochée).
pub fn seuil_horloge() -> (f64, f64) {
    let px_m = 0.0124;
    for e in 0..40 {
        let t = 2f64.powi(e);
        // Écart max = demi-ulp du temps × 1 m/s.
        if geo::ulp32(t) / 2.0 > 0.5 * px_m {
            return (t, t / 3600.0);
        }
    }
    (f64::NAN, f64::NAN)
}

pub fn _inutilise(_: DVec2) {}
