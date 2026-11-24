//! P-06 — Origine flottante et précision.
//!
//! Modes sans fenêtre : `geo`, `precision`, `cout`, `temps`, `simu`, `derive`,
//! `sauvegarde`. Modes fenêtrés (RTX 3060) : `voir`, `capture`, `profondeur`,
//! `ombres`, `cadence` (voir `ecran.rs`).

mod analyse;
mod ecran;
mod geo;
mod monde;
mod simu;
mod stats;
#[cfg(test)]
mod tests;

use bevy::prelude::*;
use big_space::prelude::*;
use clap::Parser;

use monde::{Chronos, Options, Releve, Scenario, Strategie};
use stats::f;

#[derive(Parser, Debug, Clone)]
pub struct Args {
    #[arg(long, default_value = "precision")]
    pub mode: String,
    #[arg(long, default_value = "chaque-frame")]
    pub strategie: String,
    #[arg(long, default_value = "soldat")]
    pub scenario: String,
    #[arg(long, default_value = "Brest")]
    pub lieu: String,
    #[arg(long, default_value_t = 3600)]
    pub frames: u32,
    /// Temps de jeu au départ (jours).
    #[arg(long, default_value_t = 20.0)]
    pub jour: f64,
    #[arg(long, default_value_t = 1.0)]
    pub acceleration: f64,
    #[arg(long, default_value_t = 25.0)]
    pub distance: f64,
    #[arg(long, default_value_t = 16)]
    pub threads: usize,
    #[arg(long, default_value_t = 1024)]
    pub lot: usize,
    #[arg(long, default_value_t = 100_000)]
    pub n: usize,
    #[arg(long, default_value_t = 0.01)]
    pub mobiles: f64,
    #[arg(long, default_value_t = 1000.0)]
    pub vitesse: f64,
    #[arg(long, default_value = "c")]
    pub repr: String,
    #[arg(long, default_value_t = 200)]
    pub ticks: u64,
    #[arg(long)]
    pub rendu: Option<String>,
    #[arg(long)]
    pub chrono: Option<String>,
    #[arg(long, default_value = "")]
    pub etiquette: String,
    #[arg(long)]
    pub entete: bool,
    #[arg(long)]
    pub proj: bool,
    #[arg(long)]
    pub ombres: bool,
    #[arg(long, default_value = "")]
    pub sortie: String,
}

fn main() {
    let a = Args::parse();
    match a.mode.as_str() {
        "geo" => {
            if a.proj {
                analyse::geo_proj();
            } else {
                analyse::geo_csv(a.entete);
                let (n, d, c) = analyse::plus_loin();
                eprintln!("plus loin de Paris : {n}, {:.0} km (corde {:.0} km)", d / 1000.0, c / 1000.0);
            }
        }
        "precision" => precision(&a),
        "cout" => cout(&a),
        "temps" => {
            analyse::temps_csv(a.entete);
            let (t, h) = analyse::seuil_horloge();
            eprintln!("horloge f32 absolue visible (défilement 1 m/s, 0,5 px) à partir de {t} s = {h:.1} h de jeu");
        }
        "simu" => simu(&a),
        "derive" => {
            if a.entete {
                println!("vitesse_m_s,distance_km,repr,ecart_m,biais_vitesse_pct");
            }
            for v in [1.4f32, 0.3, 0.05] {
                for (r, e, b) in simu::derive(v, 100_000.0) {
                    println!("{v},100,{r},{},{}", f(e), f(b));
                }
            }
        }
        "sauvegarde" => {
            if a.entete {
                println!("repr,octets_memoire_ou_bruts,octets_zstd");
            }
            for (r, b, z) in simu::sauvegarde(12_300) {
                println!("{r},{},{}", f(b), f(z));
            }
        }
        _ => ecran::lancer(&a),
    }
}

pub fn scenario_de(a: &Args) -> Scenario {
    match a.scenario.as_str() {
        "soldat" => Scenario::Soldat { lieu: a.lieu.clone(), distance: a.distance },
        "navire" => Scenario::Navire,
        "zoom" => Scenario::Zoom { lieu: a.lieu.clone(), frames: a.frames / 2 },
        "cout" => Scenario::Cout { n: a.n, mobiles: a.mobiles, vitesse: a.vitesse },
        s => panic!("scénario inconnu : {s}"),
    }
}

pub fn app_sans_fenetre(strat: Strategie, threads: usize) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(TaskPoolPlugin { task_pool_options: TaskPoolOptions::with_num_threads(threads) }));
    if strat.big_space() {
        app.add_plugins(BigSpaceDefaultPlugins);
    } else {
        app.add_plugins(TransformPlugin);
    }
    app
}

fn precision(a: &Args) {
    let strat = Strategie::lire(&a.strategie);
    let mut app = app_sans_fenetre(strat, a.threads);
    monde::installer(
        &mut app,
        &Options { strat, scenario: scenario_de(a), t0: a.jour * 86400.0, acceleration: a.acceleration, lot: a.lot, fenetre: false },
    );
    app.finish();
    app.cleanup();
    for _ in 0..3 {
        app.update();
    }
    app.world_mut().resource_mut::<Releve>().actif = true;
    let frames = if a.scenario == "navire" {
        let l = app.world().resource::<monde::Navire>().longueur();
        let v = app.world().resource::<monde::Navire>().vitesse;
        ((l / (v * a.acceleration / 60.0)) as u32).max(10)
    } else {
        a.frames
    };
    for _ in 0..frames {
        app.update();
    }
    let w = app.world();
    let rel = w.resource::<Releve>();
    if a.entete {
        println!("etiquette,scenario,lieu,strategie,sonde,n,px_mediane,px_p99,px_max,mm_mediane,mm_p99,mm_max,trem_px_p99,trem_px_max,recalages");
    }
    let recalages = w.resource::<monde::Origine>().recalages;
    for (nom, px, mm, tr) in monde::resumes(rel) {
        println!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            a.etiquette,
            a.scenario,
            if a.scenario == "navire" { "Brest-Papeete-Noumea" } else { &a.lieu },
            strat.nom(),
            nom,
            px.n,
            f(px.mediane),
            f(px.p99),
            f(px.max),
            f(mm.mediane),
            f(mm.p99),
            f(mm.max),
            f(tr.p99),
            f(tr.max),
            recalages
        );
    }
    if let Some(c) = &a.chrono {
        let mut s = String::new();
        if a.scenario == "navire" {
            s.push_str("frame,s_km,lat,lon,en_tile,ecart_tile_exact_m\n");
            for l in &rel.chrono {
                s.push_str(l);
                s.push('\n');
            }
            let nav = w.resource::<monde::Navire>();
            s.push_str("\n# sauts de représentation\ntemps_s,vers_tile,saut_m,saut_px\n");
            for j in &nav.sauts {
                s.push_str(&format!("{:.1},{},{:.6},{:.3}\n", j.temps, j.vers_tile as u8, j.saut_m, j.saut_px));
            }
        } else {
            for l in &rel.chrono {
                s.push_str(l);
                s.push('\n');
            }
        }
        std::fs::write(c, s).unwrap();
    }
    if a.scenario == "navire" {
        let nav = w.resource::<monde::Navire>();
        for j in &nav.sauts {
            eprintln!("saut {} : {:.1} mm, {:.2} px", if j.vers_tile { "mer → tile" } else { "tile → mer" }, j.saut_m * 1000.0, j.saut_px);
        }
    }
}

fn cout(a: &Args) {
    let strat = Strategie::lire(&a.strategie);
    let mut app = app_sans_fenetre(strat, a.threads);
    monde::installer(
        &mut app,
        &Options {
            strat,
            scenario: Scenario::Cout { n: a.n, mobiles: a.mobiles, vitesse: a.vitesse },
            t0: 0.0,
            acceleration: 1.0,
            lot: a.lot,
            fenetre: false,
        },
    );
    app.finish();
    app.cleanup();
    for _ in 0..30 {
        app.update();
    }
    app.world_mut().resource_mut::<Chronos>().actif = true;
    for _ in 0..a.frames {
        app.update();
    }
    let c = app.world().resource::<Chronos>();
    let tot: Vec<f64> = c.recalage.iter().zip(&c.propagation).map(|(x, y)| x + y).collect();
    let rec: Vec<f64> = tot.iter().zip(&c.frames_recalees).filter(|(_, r)| **r).map(|(t, _)| *t).collect();
    let norec: Vec<f64> = tot.iter().zip(&c.frames_recalees).filter(|(_, r)| !**r).map(|(t, _)| *t).collect();
    let (r, p, t) = (stats::resumer(&c.recalage), stats::resumer(&c.propagation), stats::resumer(&tot));
    let (fr, fnr) = (stats::resumer(&rec), stats::resumer(&norec));
    if a.entete {
        println!("etiquette,strategie,n,mobiles,threads,recal_med,recal_p99,recal_max,prop_med,prop_p99,prop_max,total_med,total_p99,total_max,frames_recalees,total_recalees_med,total_recalees_max,total_autres_med,total_autres_max");
    }
    println!(
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        a.etiquette,
        strat.nom(),
        a.n,
        a.mobiles,
        a.threads,
        f(r.mediane),
        f(r.p99),
        f(r.max),
        f(p.mediane),
        f(p.p99),
        f(p.max),
        f(t.mediane),
        f(t.p99),
        f(t.max),
        rec.len(),
        f(fr.mediane),
        f(fr.max),
        f(fnr.mediane),
        f(fnr.max)
    );
}

fn simu(a: &Args) {
    let repr = a.repr.chars().next().unwrap();
    let r = simu::lancer(repr, a.n, a.ticks, a.threads, a.lot, &a.lieu, a.rendu.as_deref());
    let s = stats::resumer(&r.ticks_ms[r.ticks_ms.len().min(10)..]);
    if a.entete {
        println!("etiquette,repr,n,lieu,threads,lot,rendu,ticks,tick_med_ms,tick_p99_ms,tick_max_ms,octets_position,somme");
    }
    println!(
        "{},{},{},{},{},{},{},{},{},{},{},{},{:016x}",
        a.etiquette,
        repr,
        a.n,
        a.lieu,
        a.threads,
        a.lot,
        a.rendu.clone().unwrap_or_else(|| "aucun".into()),
        a.ticks,
        f(s.mediane),
        f(s.p99),
        f(s.max),
        r.octets_position,
        r.somme
    );
}
