//! P-07 — Chaîne de données IGN sur la Manche.
//!
//! Modes :
//! - `chaine` : données brutes d'un département → paquets de tiles ;
//! - `charger` : coût de chargement d'une tile en jeu, et contrôles sur les
//!   tiles écrites (raccords du relief, décodage, recalcul du bâti) ;
//! - `images` : images de vérification de quelques tiles ;
//! - `verif-geo` : projections contre PROJ (`cs2cs`).

mod chaine;
mod decodage;
mod charge;
mod geo;
mod gpkg;
mod images;
mod mesure;
mod objets;
mod population;
mod raster;
mod relief;
mod tuile;
mod zone;

#[cfg(test)]
mod tests;

use clap::Parser;
use geo::TileId;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "chaine")]
    mode: String,
    #[arg(long)]
    gpkg: Option<PathBuf>,
    #[arg(long, default_value = "50")]
    dept: String,
    #[arg(long)]
    sortie: Option<PathBuf>,
    #[arg(long)]
    rge1m: Option<PathBuf>,
    #[arg(long)]
    rge5m: Option<PathBuf>,
    #[arg(long)]
    insee200: Option<PathBuf>,
    /// Pas des nœuds du relief stocké (m) : 1, 2 ou 5.
    #[arg(long, default_value_t = 2)]
    relief_pas: usize,
    /// Pas de quantification du relief (cm).
    #[arg(long, default_value_t = 10)]
    relief_q: i32,
    /// Mesure toutes les variantes du relief (1, 2, 5 m ; 1 et 10 cm ; 5 m natif).
    #[arg(long)]
    stats_relief: bool,
    #[arg(long, default_value_t = 9)]
    niveau: i32,
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    murs: bool,
    /// Taille minimale d'une cour fermée à percer (cases de 2 m).
    #[arg(long, default_value_t = 4)]
    cour_min: u32,
    /// Longueur maximale d'un passage (cases de 2 m).
    #[arg(long, default_value_t = 30)]
    passage_max: u32,
    /// Tiles à traiter seules (« x366_y6961,… »), pour les essais.
    #[arg(long, default_value = "")]
    essai: String,
    #[arg(long, default_value_t = 30.0)]
    mer_km: f64,
    #[arg(long, default_value_t = 0)]
    threads: usize,
    #[arg(long, default_value = "")]
    etiquette: String,
    /// Tiles pour les images (« x366_y6961:cherbourg,… »).
    #[arg(long, default_value = "")]
    tiles: String,
}

fn tile_de(s: &str) -> TileId {
    let s = s.trim();
    let (a, b) = s.trim_start_matches('x').split_once("_y").expect("tile au format x366_y6961");
    TileId { ix: a.parse().unwrap(), iy: b.parse().unwrap() }
}

fn main() {
    let a = Args::parse();
    if a.threads > 0 {
        rayon::ThreadPoolBuilder::new().num_threads(a.threads).build_global().unwrap();
    }
    match a.mode.as_str() {
        "chaine" => {
            let p = chaine::Params {
                gpkg: a.gpkg.expect("--gpkg"),
                dept: a.dept,
                sortie: a.sortie.expect("--sortie"),
                rge1m: a.rge1m,
                rge5m: a.rge5m,
                insee200: a.insee200,
                relief_pas: a.relief_pas,
                relief_q: a.relief_q,
                stats_relief: a.stats_relief,
                niveau: a.niveau,
                murs: a.murs,
                cour_min: a.cour_min,
                passage_max: a.passage_max,
                essai: a.essai.split(',').filter(|s| !s.is_empty()).map(tile_de).collect(),
                mer_km: a.mer_km,
                etiquette: a.etiquette,
            };
            chaine::executer(&p);
        }
        "charger" => charge::executer(&a.sortie.expect("--sortie"), &a.etiquette),
        "images" => {
            let ts: Vec<(TileId, String)> = a
                .tiles
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| {
                    let (t, n) = s.split_once(':').unwrap_or((s, ""));
                    (tile_de(t), n.to_string())
                })
                .collect();
            images::executer(&a.sortie.expect("--sortie"), &ts);
        }
        "verif-geo" => verif_geo(),
        m => panic!("mode inconnu : {m}"),
    }
}

/// Compare les projections à PROJ (cs2cs) sur une grille de points de la
/// Manche et de la France : écart en mm.
fn verif_geo() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let conv = geo::Laea2L93::new();
    let mut pts = Vec::new();
    for k in 0..200 {
        let lat = 41.3 + (k % 20) as f64 * 0.4;
        let lon = -5.0 + (k / 20) as f64 * 1.4;
        pts.push((lat, lon));
    }
    let entree: String = pts.iter().map(|(la, lo)| format!("{lo} {la}\n")).collect();
    let lancer = |epsg: &str| -> Vec<(f64, f64)> {
        let mut c = Command::new("cs2cs")
            .args(["-f", "%.4f", "EPSG:4326", &format!("EPSG:{epsg}")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("cs2cs");
        // cs2cs suit l'ordre des axes d'EPSG:4326 (lat, lon) : on lui donne lat lon.
        let e2: String = pts.iter().map(|(la, lo)| format!("{la} {lo}\n")).collect();
        c.stdin.take().unwrap().write_all(e2.as_bytes()).unwrap();
        let o = String::from_utf8(c.wait_with_output().unwrap().stdout).unwrap();
        o.lines()
            .map(|l| {
                let v: Vec<f64> = l.split_whitespace().take(2).map(|x| x.parse().unwrap()).collect();
                (v[0], v[1])
            })
            .collect()
    };
    let _ = entree;
    let l93 = lancer("2154");
    let laea = lancer("3035");
    let (mut m1, mut m2, mut m3) = (0f64, 0f64, 0f64);
    println!("projection,lat,lon,ecart_mm");
    for (k, (la, lo)) in pts.iter().enumerate() {
        let (x, y) = conv.l93.avant(*la, *lo);
        let e1 = ((x - l93[k].0).powi(2) + (y - l93[k].1).powi(2)).sqrt() * 1e3;
        // EPSG:3035 : axes (nord, est) dans PROJ.
        let (e, n) = conv.laea.avant(*la, *lo);
        let e2 = ((n - laea[k].0).powi(2) + (e - laea[k].1).powi(2)).sqrt() * 1e3;
        // Aller-retour LAEA → L93 contre cs2cs direct.
        let (xx, yy) = conv.conv(e, n);
        let e3 = ((xx - l93[k].0).powi(2) + (yy - l93[k].1).powi(2)).sqrt() * 1e3;
        m1 = m1.max(e1);
        m2 = m2.max(e2);
        m3 = m3.max(e3);
        println!("l93,{la},{lo},{e1:.3}");
        println!("laea,{la},{lo},{e2:.3}");
        println!("laea->l93,{la},{lo},{e3:.3}");
    }
    eprintln!("écart max : Lambert-93 {m1:.3} mm, LAEA {m2:.3} mm, LAEA → Lambert-93 {m3:.3} mm (précision de sortie de cs2cs : 0,1 mm)");
}
