//! P-05 — Civils agrégés : faire vivre 1 million de colons (et plus) dans
//! des colonies, sur les bâtiments réels de la BD TOPO ; comparer une entité
//! par colon (A), des dossiers compacts par bâtiment (B) et un agrégat
//! statistique (C) ; individualiser près de la caméra ; assaut d'une
//! colonie isolée avec la simulation des zombies de P-04.

mod alea;
mod assaut;
#[allow(dead_code)] // repris de P-04
mod carte;
#[allow(dead_code)]
mod champ;
mod civils;
mod colon;
#[allow(dead_code)]
mod humains;
#[allow(dead_code)]
mod index;
mod individus;
mod lieux;
mod marche;
#[allow(dead_code)]
mod mesure;
#[allow(dead_code)]
mod sauvegarde;
mod sauve_civils;
#[allow(dead_code)]
mod sim;
#[allow(dead_code)]
mod ville;
#[allow(dead_code)]
mod voisinage;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};

use civils::{Civils, ConfigCivils, Repr};
use colon::{NB_RES, ParamsCivils, TPS};
use lieux::Plan;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Colonie calme en tiles sûres : coût par tick.
    Calme,
    /// Caméra qui survole une colonie dense du bloc de P-04.
    Survol,
    /// Assaut d'une colonie isolée (zombies de P-04).
    Assaut,
    /// Hiver sans couvertures ni médicaments : plusieurs jours de jeu.
    Hiver,
    /// Taille de l'état d'un colon dans la sauvegarde d'une tile.
    Sauvegarde,
}

#[derive(Parser, Clone)]
#[command(about = "P-05 — civils agrégés", args_override_self = true)]
pub struct Args {
    #[arg(long, value_enum, default_value = "calme")]
    pub mode: Mode,
    #[arg(long, value_enum, default_value = "b")]
    pub repr: Repr,
    #[arg(long, default_value = "../../data/generated/p05-batiments/batiments.csv")]
    pub batiments: PathBuf,
    #[arg(long, default_value_t = 1_000_000)]
    pub colons: usize,
    #[arg(long, default_value_t = 100_000)]
    pub taille_colonie: usize,
    /// Ticks entre deux mises à jour des jauges (20 : 1 Hz).
    #[arg(long, default_value_t = 20)]
    pub periode: u32,
    #[arg(long, default_value_t = 16)]
    pub threads: usize,
    #[arg(long, default_value_t = 1024)]
    pub lot_min: usize,
    /// Exécuteur mono-thread de Bevy pour le schedule des civils.
    #[arg(long)]
    pub mono: bool,
    /// Heure de jeu au début (h).
    #[arg(long, default_value_t = 3.0)]
    pub heure: f64,
    /// Ticks de mise en route avant la mesure.
    #[arg(long, default_value_t = 400)]
    pub chauffe: u32,
    /// Ticks mesurés.
    #[arg(long, default_value_t = 2400)]
    pub ticks: u32,
    #[arg(long, default_value_t = 0x05C1_7E25)]
    pub graine: u64,
    #[arg(long, default_value_t = 12.0)]
    pub temperature: f32,
    #[arg(long)]
    pub sans_chauffage: bool,
    /// Stocks initiaux pour 100 000 colons : vivres, eau, médicaments
    /// (doses), vêtements, couvertures.
    #[arg(long, default_value = "2000000,4000000,20000,200000,200000", value_delimiter = ',')]
    pub stocks: Vec<u64>,
    /// Livraisons par jour pour 100 000 colons.
    #[arg(long, default_value = "450000,1000000,3000,0,0", value_delimiter = ',')]
    pub livraisons: Vec<u64>,
    /// Vérifie la conservation tous les N ticks (0 : jamais).
    #[arg(long, default_value_t = 0)]
    pub verif: u32,
    #[arg(long)]
    pub entete: bool,
    // ---- Survol
    #[arg(long, default_value = "../../data/generated/p01-tiles")]
    pub dossier_tiles: PathBuf,
    #[arg(long, default_value = "../../data/generated/p04-zones")]
    pub dossier_zones: PathBuf,
    /// Vitesse de la caméra (m/s de jeu).
    #[arg(long, default_value_t = 50.0)]
    pub vitesse_camera: f64,
    /// Longueur de l'aller-retour de la caméra (m).
    #[arg(long, default_value_t = 3000.0)]
    pub parcours: f64,
    /// Rayon d'individualisation (m) ; retour au-delà de rayon + 100 m.
    #[arg(long, default_value_t = 250.0)]
    pub rayon: f64,
    /// Délai avant retour à l'agrégat (s de jeu).
    #[arg(long, default_value_t = 30.0)]
    pub delai: f64,
    /// Colons individualisés (et rendus) au plus par tick.
    #[arg(long, default_value_t = 2000)]
    pub lot_ind: usize,
    /// Expansions d'A* au plus par tick (marche réelle).
    #[arg(long, default_value_t = 20_000)]
    pub budget_expansions: u64,
    /// Expansions au plus pour un chemin (au-delà : trajet abstrait).
    #[arg(long, default_value_t = 60_000)]
    pub max_expansions: u64,
    #[arg(long)]
    pub sans_marche: bool,
    #[arg(long)]
    pub sans_camera: bool,
    /// Variante : les chantiers extérieurs sont toujours individualisés.
    #[arg(long)]
    pub chantiers_individuels: bool,
    /// Durée de jeu (s).
    #[arg(long, default_value_t = 600.0)]
    pub duree: f64,
    // ---- Assaut
    /// Tile de la colonie (km Lambert-93).
    #[arg(long, default_value = "654,6861", value_delimiter = ',')]
    pub tile_colonie: Vec<i64>,
    /// Densité de zombies des tiles voisines (1 : 20 000 à 61 % de bâti).
    #[arg(long, default_value_t = 1.0)]
    pub densite: f32,
    /// Part des colons armés (miliciens).
    #[arg(long, default_value_t = 0.05)]
    pub armes: f32,
    /// Un point de perception par occupant (sinon : un par bâtiment).
    #[arg(long)]
    pub points_occupants: bool,
    #[arg(long, default_value_t = 0.3)]
    pub p_tue: f32,
    #[arg(long, default_value_t = 0.25)]
    pub p_expose: f32,
    /// Chronologie (CSV) : toutes les 10 s (assaut), chaque jour (hiver).
    #[arg(long)]
    pub chrono: Option<PathBuf>,
    // ---- Hiver
    #[arg(long, default_value_t = 7)]
    pub jours: u32,
    /// Ticks par « jour » rapporté (validation rapide du script seulement).
    #[arg(long, default_value_t = colon::TICKS_JOUR)]
    pub ticks_par_jour: u32,
    #[arg(long, default_value = "")]
    pub etiquette: String,
}

pub fn params(a: &Args) -> ParamsCivils {
    ParamsCivils {
        periode: a.periode,
        graine: a.graine,
        heure_debut: a.heure,
        temperature: a.temperature,
        chauffage: !a.sans_chauffage,
        ..ParamsCivils::default()
    }
}

pub fn ressources(v: &[u64]) -> [u64; NB_RES] {
    let mut r = [0u64; NB_RES];
    for (i, x) in v.iter().take(5).enumerate() {
        r[i] = *x;
    }
    r
}

/// Plan d'implantation sur tout le fichier (Paris et petite couronne).
pub fn plan_fichier(a: &Args) -> Plan {
    Plan {
        colons: a.colons,
        taille_colonie: a.taille_colonie,
        centre: [652_000.0, 6_862_000.0],
        ecart: 2_500.0,
        cadre: [638_000.0, 6_852_000.0, 667_000.0, 6_873_000.0],
        part_actifs: 0.6,
        part_chantiers: 0.2,
        periode: a.periode,
        graine: a.graine,
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

const ENTETE_CALME: &str = "etiquette,repr,colons,colonies,lieux,periode,threads,lot_min,heure,ticks,mediane_ms,p99_ms,max_ms,moyenne_ms,maj_par_tick,departs_par_tick,creation_s,rss_mo,octets_par_colon,conservation,arrivees_ms,maj_ms,apres_ms,somme";

fn calme(a: &Args) {
    let rss0 = mesure::rss_mo().unwrap_or(0.0);
    let bats = lieux::charger(&a.batiments).expect("bâtiments");
    let imp = lieux::implanter(&bats, &plan_fichier(a));
    drop(bats);
    let n = imp.colons.len();
    let nl = imp.lieux.len();
    let ncol = imp.colonies;
    let rss1 = mesure::rss_mo().unwrap_or(0.0);
    let t0 = Instant::now();
    let cfg = ConfigCivils {
        repr: a.repr,
        multi: !a.mono,
        individus: false,
        threads: a.threads,
        lot_min: a.lot_min,
        stocks: ressources(&a.stocks),
        livraison: ressources(&a.livraisons),
    };
    let mut c = Civils::new(&imp, params(a), &cfg);
    let creation = t0.elapsed().as_secs_f64();
    // Mémoire de la représentation : mesurée avant de libérer l'implantation
    // (sinon l'allocateur réutilise sa place).
    let rss2 = mesure::rss_mo().unwrap_or(0.0);
    drop(imp);
    let _ = rss0;
    for _ in 0..a.chauffe {
        c.tick();
    }
    c.chronos = Default::default();
    let m0 = c.commun().stats.mises_a_jour;
    let d0 = c.commun().stats.departs;
    let mut d = Vec::with_capacity(a.ticks as usize);
    let mut ok = true;
    for k in 0..a.ticks {
        let t = Instant::now();
        c.tick();
        d.push(t.elapsed());
        if a.verif > 0 && k % a.verif == 0 {
            ok &= c.conservation();
        }
    }
    ok &= c.conservation();
    let st = mesure::stats(d);
    let maj = (c.commun().stats.mises_a_jour - m0) as f64 / a.ticks as f64;
    let dep = (c.commun().stats.departs - d0) as f64 / a.ticks as f64;
    let somme = c.somme();
    let rss = rss2 - rss1;
    println!(
        "{},{:?},{n},{ncol},{nl},{},{},{},{},{},{:.4},{:.4},{:.4},{:.4},{maj:.0},{dep:.1},{creation:.2},{rss2:.0},{:.1},{},{:.4},{:.4},{:.4},{somme:016x}",
        a.etiquette,
        a.repr,
        a.periode,
        a.threads,
        a.lot_min,
        a.heure,
        a.ticks,
        st.mediane_ms,
        st.p99_ms,
        st.max_ms,
        st.moyenne_ms,
        rss * 1048576.0 / n as f64,
        ok as u8,
        ms(c.chronos[0]) / a.ticks as f64,
        ms(c.chronos[1]) / a.ticks as f64,
        ms(c.chronos[2]) / a.ticks as f64,
    );
    let _ = (TPS, ms(Duration::ZERO));
}

/// Bloc de P-04 : 11 × 5 tiles de Paris (x 646 à 656, y 6859 à 6863 km).
pub const BLOC: [i64; 4] = [646, 6859, 11, 5];

pub fn geo_bloc() -> carte::Geo {
    carte::Geo { x0: BLOC[0], y0: BLOC[1], w: BLOC[2] as usize, h: BLOC[3] as usize }
}

pub fn plan_bloc(a: &Args) -> Plan {
    let g = geo_bloc();
    let (x0, y0) = (g.x0 as f64 * 1000.0, g.y0 as f64 * 1000.0);
    let (x1, y1) = (x0 + g.w as f64 * 1000.0, y0 + g.h as f64 * 1000.0);
    Plan {
        colons: a.colons,
        taille_colonie: a.taille_colonie,
        centre: [(x0 + x1) / 2.0, (y0 + y1) / 2.0],
        ecart: 2_500.0,
        cadre: [x0, y0, x1, y1],
        part_actifs: 0.6,
        part_chantiers: 0.2,
        periode: a.periode,
        graine: a.graine,
    }
}

const ENTETE_SURVOL: &str = "etiquette,colons,periode,vitesse_camera,rayon,delai_s,lot_ind,marche,chantiers_individuels,heure,duree_s,ticks,mediane_ms,p99_ms,max_ms,camera_ms,ind_ms,ret_ms,marche_ms,maj_ms,apres_ms,lieux_ind_min,colons_ind_min,lieux_ret_min,colons_ret_min,rebascules,ticks_ind_moy,ticks_ind_max,ns_par_colon_ind,ns_par_colon_ret,entites_max,chemins,chemins_echoues,expansions_moy,chemin_ms_moy,chemin_ms_max,marcheurs_max,arrivees_marche,attente_chemin_max_s,portes_pct,conservation,doublons,somme";

fn survol(a: &Args) {
    let bats = lieux::charger(&a.batiments).expect("bâtiments");
    let imp = lieux::implanter(&bats, &plan_bloc(a));
    drop(bats);
    let (carte, _) =
        carte::Carte::reelle(&a.dossier_tiles, geo_bloc(), Some(&a.dossier_zones), 1.0).expect("carte du bloc");
    let cfg = ConfigCivils {
        repr: Repr::B,
        multi: !a.mono,
        individus: true,
        threads: a.threads,
        lot_min: a.lot_min,
        stocks: ressources(&a.stocks),
        livraison: ressources(&a.livraisons),
    };
    let n = imp.colons.len();
    let mut c = Civils::new(&imp, params(a), &cfg);
    drop(imp);
    let mut ind = individus::Indiv::new(c.commun(), Some(carte), !a.sans_marche);
    ind.rayon = a.rayon;
    ind.rayon_retour = a.rayon + 100.0;
    ind.delai = (a.delai * TPS as f64) as u32;
    ind.lot_ind = a.lot_ind;
    ind.lot_ret = a.lot_ind;
    ind.budget_expansions = a.budget_expansions;
    ind.max_expansions = a.max_expansions;
    if a.chantiers_individuels {
        ind.chantiers_permanents(c.commun());
    }
    let portes = ind.portes.iter().filter(|p| p.is_some()).count() as f64 / ind.portes.len() as f64;
    c.world.insert_resource(ind);
    let g = geo_bloc();
    let centre = [(g.x0 as f64 + g.w as f64 / 2.0) * 1000.0, (g.y0 as f64 + g.h as f64 / 2.0) * 1000.0];
    let ticks = (a.duree * TPS as f64) as u32;
    let mut d = Vec::with_capacity(ticks as usize);
    c.chronos = Default::default();
    let mut ok = true;
    for k in 0..ticks {
        if !a.sans_camera {
            // Aller-retour d'ouest en est, centré sur le bloc.
            let s = (k as f64 / TPS as f64 * a.vitesse_camera) % (2.0 * a.parcours);
            let x = if s < a.parcours { s } else { 2.0 * a.parcours - s };
            c.world.resource_mut::<individus::Indiv>().camera = Some([centre[0] - a.parcours / 2.0 + x, centre[1]]);
        }
        let t = Instant::now();
        c.tick();
        d.push(t.elapsed());
        if a.verif > 0 && k % a.verif == 0 {
            ok &= c.conservation();
        }
    }
    ok &= c.conservation();
    let ids = c.identifiants();
    let doublons = ids.windows(2).filter(|w| w[0] == w[1]).count();
    let somme = c.somme();
    let st = mesure::stats(d);
    let ind = c.world.resource::<individus::Indiv>();
    let s = &ind.stats;
    let minutes = a.duree / 60.0;
    let tk = ticks as f64;
    let moy = |v: &[u32]| if v.is_empty() { 0.0 } else { v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64 };
    println!(
        "{},{n},{},{},{},{},{},{},{},{},{},{ticks},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.1},{:.0},{:.1},{:.0},{},{:.1},{},{:.0},{:.0},{},{},{},{:.0},{:.3},{:.3},{},{},{:.1},{:.3},{},{doublons},{somme:016x}",
        a.etiquette,
        a.periode,
        a.vitesse_camera,
        a.rayon,
        a.delai,
        a.lot_ind,
        !a.sans_marche as u8,
        a.chantiers_individuels as u8,
        a.heure,
        a.duree,
        st.mediane_ms,
        st.p99_ms,
        st.max_ms,
        s.ns_camera as f64 / 1e6 / tk,
        s.ns_ind as f64 / 1e6 / tk,
        s.ns_ret as f64 / 1e6 / tk,
        s.ns_marche as f64 / 1e6 / tk,
        ms(c.chronos[1]) / tk,
        ms(c.chronos[2]) / tk,
        s.lieux_ind as f64 / minutes,
        s.colons_ind as f64 / minutes,
        s.lieux_ret as f64 / minutes,
        s.colons_ret as f64 / minutes,
        s.rebascules,
        moy(&s.ticks_ind),
        s.ticks_ind.iter().max().copied().unwrap_or(0),
        s.ns_ind as f64 / s.colons_ind.max(1) as f64,
        s.ns_ret as f64 / s.colons_ret.max(1) as f64,
        s.max_entites,
        s.chemins,
        s.chemins_echoues,
        s.expansions as f64 / s.chemins.max(1) as f64,
        s.ns_chemins as f64 / 1e6 / s.chemins.max(1) as f64,
        s.ns_chemin_max as f64 / 1e6,
        s.marcheurs_max,
        s.arrivees_marche,
        s.attente_max as f64 / TPS as f64,
        portes * 100.0,
        ok as u8,
    );
}

const ENTETE_ASSAUT: &str = "etiquette,colons,batiments,armes,points_occupants,densite,p_tue,p_expose,threads,duree_s,ticks,mediane_ms,p99_ms,max_ms,gestion_ms,humains_ms,grilles_ms,champs_ms,perception_ms,deplacement_ms,admission_ms,frontieres_ms,entrees_ms,resolution_ms,zombification_ms,civils_ms,points_perception,entrees,abattus_dedans,abattus_total,expositions,zombifies,sorties,batiments_tombes,chute_s,humains_fin,morts,zombies_fin,actives_max,conservation_humains,conservation_zombies,zombifies_egal_entites,somme";

fn assaut(a: &Args) {
    use std::io::Write;
    let g = geo_bloc();
    let (tx, ty) = (a.tile_colonie[0], a.tile_colonie[1]);
    let tile = g.tile((tx - g.x0) as i32, (ty - g.y0) as i32).expect("tile de la colonie hors du bloc");
    let (x0, y0) = (tx as f64 * 1000.0, ty as f64 * 1000.0);
    let plan = Plan {
        colons: a.colons,
        taille_colonie: a.colons,
        centre: [x0 + 500.0, y0 + 500.0],
        ecart: 2_500.0,
        cadre: [x0, y0, x0 + 1000.0, y0 + 1000.0],
        part_actifs: 0.6,
        part_chantiers: 0.2,
        periode: a.periode,
        graine: a.graine,
    };
    let bats = lieux::charger(&a.batiments).expect("bâtiments");
    let imp = lieux::implanter(&bats, &plan);
    drop(bats);
    let cfg = ConfigCivils {
        repr: Repr::B,
        multi: !a.mono,
        individus: false,
        threads: a.threads,
        lot_min: a.lot_min,
        stocks: ressources(&a.stocks),
        livraison: ressources(&a.livraisons),
    };
    let n = imp.colons.len();
    let civ = Civils::new(&imp, params(a), &cfg);
    drop(imp);
    let carte = assaut::carte_colonie(&a.dossier_tiles, &a.dossier_zones, g, a.densite, tile);
    let mut p = sim::Params {
        prechauffe_entites: 250_000,
        dossier: PathBuf::from(format!("../../data/generated/p05-sauvegardes/assaut-{}", std::process::id())),
        ..sim::Params::default()
    };
    p.delai = 1200;
    let cs = sim::ConfigSim { threads: a.threads, lot_min: a.lot_min, slots_initiaux: 16, params: p };
    let pa = assaut::ParamsAssaut {
        p_tue: a.p_tue,
        p_expose: a.p_expose,
        points_occupants: a.points_occupants,
        ..Default::default()
    };
    let mut x = assaut::Assaut::new(civ, carte, pa, &cs, a.armes);
    let nb = x.bats.len();
    let npts: usize = x.bats.iter().map(|b| b.nh).sum();
    let mut chrono = a.chrono.as_ref().map(|c| {
        let mut f = std::fs::File::create(c).expect("chronologie");
        writeln!(f, "t_s,humains,exposes,zombifies,morts,batiments_tenus,zombies_dedans,zombies_vivants,abattus,tick_max_ms,tick_moy_ms").unwrap();
        f
    });
    let ticks = (a.duree * TPS as f64) as u32;
    let mut d = Vec::with_capacity(ticks as usize);
    let (mut ok_h, mut ok_z) = (true, true);
    let mut actives_max = 0;
    let (mut fen_max, mut fen_tot) = (0.0f64, 0.0f64);
    for k in 0..ticks {
        let t = Instant::now();
        x.tick();
        let e = t.elapsed();
        d.push(e);
        fen_max = fen_max.max(ms(e));
        fen_tot += ms(e);
        actives_max = actives_max.max(x.sim.nb_actifs());
        if a.verif > 0 && k % a.verif == 0 {
            ok_h &= x.conservation_humains();
            ok_z &= x.sim.verifier_conservation();
        }
        if (k + 1) % (10 * TPS) == 0
            && let Some(f) = chrono.as_mut()
        {
            let h = x.civils.vivants();
            let zv = x.zombies_vivants();
            let cm = x.civils.commun();
            writeln!(
                f,
                "{},{h},{},{},{},{},{},{zv},{},{fen_max:.3},{:.3}",
                (k + 1) / TPS,
                x.exposes(),
                cm.stats.zombifies,
                cm.stats.morts,
                x.tenus(),
                x.zombies_dedans(),
                x.sim.stats.abattus,
                fen_tot / (10 * TPS) as f64
            )
            .unwrap();
            fen_max = 0.0;
            fen_tot = 0.0;
        }
    }
    ok_h &= x.conservation_humains();
    ok_z &= x.sim.verifier_conservation();
    let st = mesure::stats(d);
    let tk = ticks as f64;
    let s = x.stats.clone();
    let ph: Vec<String> = s.phases.iter().map(|p| format!("{:.4}", ms(*p) / tk)).collect();
    let somme = x.sim.somme_controle() ^ x.civils.somme();
    let h = x.civils.vivants();
    let zf = x.zombies_vivants();
    let cm = x.civils.commun();
    let egal = s.zombifies == cm.stats.zombifies;
    println!(
        "{},{n},{nb},{},{},{},{},{},{},{},{ticks},{:.4},{:.4},{:.4},{},{:.4},{:.4},{:.4},{:.4},{npts},{},{},{},{},{},{},{},{},{h},{},{zf},{actives_max},{},{},{},{somme:016x}",
        a.etiquette,
        a.armes,
        a.points_occupants as u8,
        a.densite,
        a.p_tue,
        a.p_expose,
        a.threads,
        a.duree,
        st.mediane_ms,
        st.p99_ms,
        st.max_ms,
        ph.join(","),
        s.ns_entrees as f64 / 1e6 / tk,
        s.ns_resolution as f64 / 1e6 / tk,
        s.ns_zombification as f64 / 1e6 / tk,
        s.ns_civils as f64 / 1e6 / tk,
        s.entrees,
        s.abattus_dedans,
        x.sim.stats.abattus,
        s.expositions,
        s.zombifies,
        s.sorties,
        s.batiments_tombes,
        s.chute_tick.map(|t| format!("{:.0}", t as f64 / TPS as f64)).unwrap_or_default(),
        cm.stats.morts,
        ok_h as u8,
        ok_z as u8,
        egal as u8,
    );
    let _ = std::fs::remove_dir_all(&x.sim.p.dossier);
}

/// Actifs disponibles : colons qui ont un travail et ne sont pas malades.
fn disponibles(c: &mut Civils) -> u64 {
    match c.repr {
        Repr::B => {
            let d = c.world.resource::<civils::Dossiers>();
            let mut n = d.0.iter().flatten().filter(|x| x.travail != colon::NUL && x.malade == 0).count() as u64;
            n += c
                .commun()
                .transit
                .iter()
                .filter(|x| x.0.colon.is_some_and(|x| x.travail != colon::NUL && x.malade == 0))
                .count() as u64;
            n
        }
        Repr::A1 | Repr::A2 => c
            .world
            .query::<&civils::Fiche>()
            .iter(&c.world)
            .filter(|f| f.0.travail != colon::NUL && f.0.malade == 0)
            .count() as u64,
        Repr::C => c
            .world
            .resource::<civils::Agregats>()
            .0
            .iter()
            .map(|a| if a.n == 0 { 0 } else { (a.actifs as f64 * (1.0 - a.malades as f64 / a.n as f64)).round() as u64 })
            .sum(),
    }
}

const ENTETE_HIVER: &str = "etiquette,repr,colons,periode,temperature,chauffage,jours,secondes_reelles,us_par_tick,malades_fin,morts,corps,disponibles_debut,disponibles_fin,heures_travail_j1,heures_travail_fin,vivres_deficit,somme";

fn hiver(a: &Args) {
    use std::io::Write;
    let bats = lieux::charger(&a.batiments).expect("bâtiments");
    let imp = lieux::implanter(&bats, &plan_fichier(a));
    drop(bats);
    let cfg = ConfigCivils {
        repr: a.repr,
        multi: !a.mono,
        individus: false,
        threads: a.threads,
        lot_min: a.lot_min,
        stocks: ressources(&a.stocks),
        livraison: ressources(&a.livraisons),
    };
    let n = imp.colons.len();
    let mut c = Civils::new(&imp, params(a), &cfg);
    drop(imp);
    let mut chrono = a.chrono.as_ref().map(|p| {
        let mut f = std::fs::File::create(p).expect("chronologie");
        writeln!(f, "jour,vivants,malades,morts,corps,disponibles,heures_travail,vivres,eau_potable,medicaments,vetements_chauds,couvertures,eau_non_potable_bue,risque_moyen,risque_max").unwrap();
        f
    });
    let dispo0 = disponibles(&mut c);
    let t0 = Instant::now();
    let jour = a.ticks_par_jour;
    let (mut h_j1, mut h_fin) = (0.0, 0.0);
    let mut travail_prec = 0u64;
    for j in 1..=a.jours {
        for _ in 0..jour {
            c.tick();
        }
        let v = c.vivants();
        let m = c.malades();
        let dispo = disponibles(&mut c);
        let cm = c.commun();
        let corps: u64 = cm.corps.iter().flatten().map(|&x| x as u64).sum();
        let h = (cm.stats.travail_ticks - travail_prec) as f64 / TPS as f64 / 3600.0;
        travail_prec = cm.stats.travail_ticks;
        if j == 1 {
            h_j1 = h;
        }
        h_fin = h;
        let st: Vec<u64> = (0..5).map(|r| cm.stocks.iter().map(|s| s[r]).sum()).collect();
        let rs: Vec<f32> = cm.risque.iter().flatten().copied().collect();
        let rmoy = rs.iter().sum::<f32>() / rs.len() as f32;
        let rmax = rs.iter().copied().fold(0.0, f32::max);
        if let Some(f) = chrono.as_mut() {
            writeln!(
                f,
                "{j},{v},{m},{},{corps},{dispo},{h:.0},{},{},{},{},{},{},{rmoy:.3},{rmax:.3}",
                cm.stats.morts,
                st[0],
                st[1],
                st[2] / colon::DOSE,
                st[3],
                st[4],
                cm.stats.consomme[colon::EAU_NP]
            )
            .unwrap();
        }
        eprintln!("jour {j} : {v} vivants, {m} malades, {} morts", cm.stats.morts);
    }
    let secs = t0.elapsed().as_secs_f64();
    let ticks = a.jours as f64 * jour as f64;
    let m = c.malades();
    let dispo = disponibles(&mut c);
    let somme = c.somme();
    let cm = c.commun();
    let corps: u64 = cm.corps.iter().flatten().map(|&x| x as u64).sum();
    println!(
        "{},{:?},{n},{},{},{},{},{secs:.1},{:.2},{m},{},{corps},{dispo0},{dispo},{h_j1:.0},{h_fin:.0},{},{somme:016x}",
        a.etiquette,
        a.repr,
        a.periode,
        a.temperature,
        !a.sans_chauffage as u8,
        a.jours,
        secs * 1e6 / ticks,
        cm.stats.morts,
        cm.stats.deficit[colon::VIVRES],
    );
}

const ENTETE_SAUVEGARDE: &str = "etiquette,colons,tiles,ticks_simules,octets_memoire_par_colon,octets_bruts_par_colon,octets_zstd_par_colon,octets_zstd_p90,ko_tile_mediane,ko_tile_max,encodage_ms_mediane,decodage_ms_mediane,colons_tile_mediane,aller_retour_identique";

fn sauvegarde(a: &Args) {
    let bats = lieux::charger(&a.batiments).expect("bâtiments");
    let imp = lieux::implanter(&bats, &plan_fichier(a));
    drop(bats);
    let cfg = ConfigCivils {
        repr: Repr::B,
        multi: !a.mono,
        individus: false,
        threads: a.threads,
        lot_min: a.lot_min,
        stocks: ressources(&a.stocks),
        livraison: ressources(&a.livraisons),
    };
    let n = imp.colons.len();
    let mut c = Civils::new(&imp, params(a), &cfg);
    drop(imp);
    for _ in 0..a.ticks {
        c.tick();
    }
    let t = c.tick;
    let cm = c.commun();
    let ds = c.world.resource::<civils::Dossiers>();
    let (mut tailles, mut parcolon, mut enc, mut dec, mut nb) = (vec![], vec![], vec![], vec![], vec![]);
    let (mut brut, mut total_z, mut total_n) = (0usize, 0usize, 0usize);
    let mut ok = true;
    for lt in &cm.lieux_tile {
        let dossiers: Vec<(u32, Vec<colon::Colon>)> =
            lt.iter().map(|&l| (l, ds.0[l as usize].clone())).filter(|(_, d)| !d.is_empty()).collect();
        let k: usize = dossiers.iter().map(|(_, d)| d.len()).sum();
        if k == 0 {
            continue;
        }
        brut += sauve_civils::charge(&dossiers, t).len();
        let t0 = Instant::now();
        let b = sauve_civils::encoder(&dossiers, t);
        enc.push(t0.elapsed());
        let t0 = Instant::now();
        let r = sauve_civils::decoder(&b).expect("décodage");
        dec.push(t0.elapsed());
        let mut a1: Vec<colon::Colon> = dossiers.iter().flat_map(|(_, d)| d.iter().copied()).collect();
        let mut a2: Vec<colon::Colon> = r.iter().flat_map(|(_, d)| d.iter().copied()).collect();
        a1.sort_unstable_by_key(|c| c.id);
        a2.sort_unstable_by_key(|c| c.id);
        ok &= a1 == a2;
        total_z += b.len();
        total_n += k;
        tailles.push(b.len() as f64 / 1024.0);
        parcolon.push(b.len() as f64 / k as f64);
        nb.push(k as f64);
    }
    let med = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };
    let p90 = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() * 9 / 10]
    };
    let ntiles = tailles.len();
    let kmax = tailles.iter().copied().fold(0.0, f64::max);
    println!(
        "{},{n},{ntiles},{},{},{:.2},{:.2},{:.2},{:.1},{:.1},{:.3},{:.3},{:.0},{}",
        a.etiquette,
        a.ticks,
        std::mem::size_of::<colon::Colon>(),
        brut as f64 / total_n as f64,
        total_z as f64 / total_n as f64,
        p90(&mut parcolon),
        med(&mut tailles),
        kmax,
        mesure::stats(enc).mediane_ms,
        mesure::stats(dec).mediane_ms,
        med(&mut nb),
        ok as u8
    );
}

fn main() {
    let a = Args::parse();
    if a.entete {
        match a.mode {
            Mode::Calme => println!("{ENTETE_CALME}"),
            Mode::Survol => println!("{ENTETE_SURVOL}"),
            Mode::Assaut => println!("{ENTETE_ASSAUT}"),
            Mode::Hiver => println!("{ENTETE_HIVER}"),
            Mode::Sauvegarde => println!("{ENTETE_SAUVEGARDE}"),
        }
        return;
    }
    match a.mode {
        Mode::Calme => calme(&a),
        Mode::Survol => survol(&a),
        Mode::Assaut => assaut(&a),
        Mode::Hiver => hiver(&a),
        Mode::Sauvegarde => sauvegarde(&a),
    }
}
