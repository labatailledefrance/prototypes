//! P-08 — Génération des bâtiments.
//!
//! Modes sans rendu :
//! - `stats` : génération et contrôles de chaque tile (CSV par tile, par
//!   catégorie d'emprise, portes, modèle) ;
//! - `debit` : bâtiments par seconde, sur 1 et N threads ;
//! - `determinisme` : mêmes octets selon les threads et l'ordre ;
//! - `etale` : génération étalée par ticks de 3 ms ;
//! - `destruction` : endommager puis effondrer un bâtiment (coûts CPU) ;
//! - `repli` : validation croisée de la règle de repli de la hauteur ;
//! - `format` : taille des attributs proposés pour le format de tile ;
//! - `monuments` : natures de la BD TOPO et tailles ;
//! - `images` : vue de dessus des portes.
//!
//! Modes de rendu (Bevy, hors écran, RTX 3060) : `cadence`, `survol`,
//! `rendu-destruction`, `capture`.

mod batiment;
mod donnees;
mod generation;
mod geo;
mod geom;
mod porte;
mod raster;
mod relief;
mod rendu;
mod stats;
mod stockage;
#[cfg(test)]
mod tests;
mod tuile;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
pub struct Args {
    #[arg(long)]
    pub mode: String,
    /// Dossier de sortie de P-07 (paquets, tiles.csv).
    #[arg(long)]
    pub sortie: PathBuf,
    /// GeoPackage de la BD TOPO (attributs absents des tiles).
    #[arg(long)]
    pub gpkg: Option<PathBuf>,
    /// Tiles (xNNN_yNNNN, séparées par des virgules) ; vide = toutes.
    #[arg(long, default_value = "")]
    pub tiles: String,
    /// Dossier des résultats.
    #[arg(long, default_value = "resultats")]
    pub res: PathBuf,
    #[arg(long, default_value = "essai")]
    pub etiquette: String,
    #[arg(long, default_value_t = 16)]
    pub threads: usize,
    #[arg(long, default_value = "1,16")]
    pub liste_threads: String,
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub raster: bool,
    #[arg(long, default_value_t = 10)]
    pub zstd19_un_sur: usize,
    #[arg(long, default_value_t = 3.0)]
    pub budget: f64,
    // --- Rendu
    /// Organisation : `tuile` (un maillage fusionné par tile), `morceaux`
    /// (fusion par carrés de 250 m), `batiment` (une entité par bâtiment).
    #[arg(long, default_value = "tuile")]
    pub organisation: String,
    /// Côté des tiles visibles (1, 3 ou 7).
    #[arg(long, default_value_t = 1)]
    pub cote: i32,
    #[arg(long, default_value = "iso")]
    pub lod: String,
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub ombres: bool,
    #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
    pub compresse: bool,
    #[arg(long, default_value_t = 600)]
    pub frames: u32,
    /// Survol : vitesse (m/s), altitude (m), rayon de chargement (m).
    #[arg(long, default_value_t = 100.0)]
    pub vitesse: f64,
    #[arg(long, default_value_t = 300.0)]
    pub altitude: f64,
    #[arg(long, default_value_t = 1500.0)]
    pub rayon: f64,
    /// Survol : trajet (x0,y0,x1,y1 en km Lambert-93).
    #[arg(long, default_value = "")]
    pub trajet: String,
    /// Survol : maillages générés avant le vol (témoin sans génération).
    #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
    pub pregenere: bool,
    /// Envoi au GPU limité à tant d'octets par frame (0 : sans limite).
    #[arg(long, default_value_t = 0)]
    pub octets_par_frame: usize,
    /// Allocation des maillages sur le GPU : `defaut` (dalles de Bevy) ou
    /// `separees` (un tampon par maillage de plus de 1 Mio).
    #[arg(long, default_value = "defaut")]
    pub dalles: String,
    #[arg(long, default_value_t = false)]
    pub entete: bool,
    /// Capture : vue `iso` ou `loin`.
    #[arg(long, default_value = "iso")]
    pub vue: String,
}

pub fn tiles_de(s: &str) -> Vec<geo::TileId> {
    s.split(',').filter(|x| !x.is_empty()).map(geo::TileId::lire).collect()
}

fn main() {
    let a = Args::parse();
    let tiles = tiles_de(&a.tiles);
    let o = stats::Opt {
        sortie: &a.sortie,
        gpkg: a.gpkg.as_deref(),
        tiles: tiles.clone(),
        res: &a.res,
        etiquette: &a.etiquette,
        raster: a.raster,
        zstd19_un_sur: a.zstd19_un_sur,
    };
    std::fs::create_dir_all(&a.res).unwrap();
    rayon::ThreadPoolBuilder::new().num_threads(a.threads).build_global().ok();
    match a.mode.as_str() {
        "stats" => stats::stats(&o),
        "debit" => {
            let th: Vec<usize> = a.liste_threads.split(',').map(|x| x.parse().unwrap()).collect();
            stats::debit(&o, &th)
        }
        "determinisme" => stats::determinisme(&o),
        "etale" => stats::etalement(&o, a.budget),
        "destruction" => stats::destruction(&o),
        "repli" => stats::repli(&o),
        "format" => stats::format(&o),
        "monuments" => stats::monuments(&o),
        "images" => stats::images(&o),
        "cadence" | "survol" | "capture" | "rendu-destruction" => rendu::lancer(&a),
        m => panic!("mode inconnu : {m}"),
    }
}
