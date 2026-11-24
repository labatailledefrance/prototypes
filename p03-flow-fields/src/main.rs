//! P-03 — Flow fields : déplacement des hordes par champs de distance sur
//! les tiles réelles de P-01, remplacement de la grille des zombies et
//! réservation des places au contact. Reprend la simulation de P-02.

mod alea;
mod bancs;
mod champ;
mod compteurs;
mod index;
mod mesure;
mod observation;
mod sim;
mod ville;
mod voisinage;

use std::hint::black_box;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};

use champ::Regle;
use sim::{ConfigSim, ModeChamp, NB_PHASES, NOMS_PHASES, Params, Recalcul, Reservation, Scenario, Simulation, SourceVille};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Ville {
    Synthetique,
    Reelle,
}

#[derive(Parser, Clone)]
#[command(about = "P-03 — flow fields", args_override_self = true)]
pub struct Args {
    #[arg(long, value_enum, default_value = "assaut")]
    pub scenario: Scenario,
    #[arg(long, value_enum, default_value = "reelle")]
    pub ville: Ville,
    #[arg(long, default_value = "../../data/generated/p01-tiles")]
    pub dossier_tiles: std::path::PathBuf,
    /// Flow fields, ou ligne droite (P-01, test 1).
    #[arg(long, value_enum, default_value = "flow")]
    pub champ: ModeChamp,
    /// Côté d'une case de champ (m).
    #[arg(long, default_value_t = 2)]
    pub resolution: usize,
    #[arg(long, value_enum, default_value = "majorite")]
    pub regle: Regle,
    /// Recalcul du champ « proies » tous les N ticks.
    #[arg(long, default_value_t = 20)]
    pub periode_champ: u32,
    #[arg(long, value_enum, default_value = "partiel")]
    pub recalcul: Recalcul,
    /// Index des zombies.
    #[arg(long, value_enum, default_value = "dense16")]
    pub voisinage: voisinage::Structure,
    /// Index utilisé pendant le préchauffage (sans effet sur le résultat).
    #[arg(long, value_enum)]
    pub chauffe_voisinage: Option<voisinage::Structure>,
    /// Côté d'une case de l'index des zombies (m).
    #[arg(long, default_value_t = 2.0)]
    pub case_zombies: f32,
    /// Réservation exacte des places : aucune (admission progressive
    /// partout, comme P-01 et P-02), au contact, ou partout.
    #[arg(long, value_enum, default_value = "partout")]
    pub reservation: Reservation,
    /// Sans marge le long des murs dans les champs.
    #[arg(long)]
    pub sans_marge: bool,
    /// Calcul du champ « proies » d'un seul tenant (pas d'étalement).
    #[arg(long)]
    pub sans_etalement: bool,
    /// Pose l'anneau de barricades au premier tick mesuré (obstacle qui
    /// apparaît pendant l'assaut).
    #[arg(long)]
    pub barricade: bool,
    /// Pose l'anneau de barricades avant le préchauffage (obstacle fixe
    /// devant lequel la foule s'accumule).
    #[arg(long)]
    pub barricade_initiale: bool,
    #[arg(long)]
    pub sans_relais: bool,
    #[arg(long)]
    pub sans_profonde: bool,
    #[arg(long, default_value_t = 1)]
    pub tiles: usize,
    #[arg(long, default_value_t = 20_000)]
    pub zombies: usize,
    #[arg(long, default_value_t = 1_000)]
    pub humains: usize,
    #[arg(long, default_value_t = 16)]
    pub threads: usize,
    #[arg(long, default_value_t = 1024)]
    pub lot_min: usize,
    /// Distance de vue (m).
    #[arg(long, default_value_t = 400.0)]
    pub vue: f32,
    #[arg(long, default_value_t = 200)]
    pub prechauffage: usize,
    #[arg(long, default_value_t = 200)]
    pub ticks: usize,
    /// Affiche l'en-tête CSV du mode choisi et quitte.
    #[arg(long)]
    pub entete: bool,
    /// Banc des champs (calcul, recalcul, obstacles, connexité).
    #[arg(long)]
    pub banc_champs: bool,
    /// Banc des frontières (échange de bordures sur 3 × 3 tiles).
    #[arg(long)]
    pub banc_frontieres: bool,
    /// Banc mono-thread des index de zombies, sur l'instantané obtenu après
    /// le préchauffage.
    #[arg(long)]
    pub banc_voisinage: bool,
    #[arg(long, default_value_t = 5)]
    pub repetitions: usize,
    /// Mode observation : crédibilité (images, blocages, progression),
    /// écrit dans ce dossier, sur la première tile.
    #[arg(long)]
    pub observer: Option<std::path::PathBuf>,
    /// Observation : durée (s de jeu) après le préchauffage.
    #[arg(long, default_value_t = 60)]
    pub duree: u32,
    /// Observation : une image toutes les N secondes.
    #[arg(long, default_value_t = 30)]
    pub fenetre: u32,
    /// Observation : pose des barricades à cet instant (s).
    #[arg(long)]
    pub barricade_a: Option<u32>,
    /// Observation : étiquette des fichiers produits.
    #[arg(long, default_value = "obs")]
    pub etiquette: String,
}

pub fn params(a: &Args) -> Params {
    Params {
        distance_vue: a.vue,
        seuil_profond: a.vue + 100.0,
        relais: !a.sans_relais,
        filtre_relais: !a.sans_relais,
        profonde: !a.sans_profonde,
        mode_champ: a.champ,
        resolution: a.resolution,
        regle: a.regle,
        periode_champ: a.periode_champ,
        recalcul: a.recalcul,
        voisinage: a.voisinage,
        case_zombies: a.case_zombies,
        reservation: a.reservation,
        marge: !a.sans_marge,
        etalement: !a.sans_etalement,
        ..Params::default()
    }
}

pub fn config(a: &Args) -> ConfigSim {
    ConfigSim {
        source: match a.ville {
            Ville::Synthetique => SourceVille::Synthetique,
            Ville::Reelle => SourceVille::Reelle(a.dossier_tiles.clone()),
        },
        scenario: a.scenario,
        tiles: a.tiles,
        zombies: a.zombies,
        humains: a.humains,
        threads: a.threads,
        lot_min: a.lot_min,
        params: params(a),
    }
}

const ENTETE: &str = "scenario,ville,champ,marge,resolution,regle,periode_champ,etalement,recalcul,voisinage,case_zombies,reservation,barricade,relais,profonde,tiles,zombies,humains,threads,lot_min,vue,prechauffage,creation_ms,mediane_ms,p99_ms,max_ms,humains_ms,grilles_ms,champs_ms,perception_ms,deplacement_ms,admission_ms,recalculs_tick,champ_us,champ_max_us,obstacle_us,invalidees,demandes_tick,admis_tick,requetes_tick,rss_mo,errance,poursuite,relayes,attire,perdu,profond,densite_max,densite_contact,murs,couverture,somme";

fn main() {
    let a = Args::parse();
    if a.entete {
        println!(
            "{}",
            if a.banc_champs {
                bancs::ENTETE_CHAMPS
            } else if a.banc_frontieres {
                bancs::ENTETE_FRONTIERES
            } else if a.banc_voisinage {
                bancs::ENTETE_VOISINAGE
            } else if a.observer.is_some() {
                observation::ENTETE
            } else {
                ENTETE
            }
        );
        return;
    }
    if a.banc_champs {
        return bancs::banc_champs(&a);
    }
    if a.banc_frontieres {
        return bancs::banc_frontieres(&a);
    }
    let mut c = config(&a);
    if let Some(v) = a.chauffe_voisinage {
        c.params.voisinage = v;
    }
    let t0 = Instant::now();
    let mut sim = Simulation::new(&c);
    let creation_ms = t0.elapsed().as_secs_f64() * 1e3;
    if a.barricade_initiale {
        sim.poser_barricades();
    }
    for _ in 0..a.prechauffage {
        sim.tick();
    }
    if a.chauffe_voisinage.is_some() {
        sim.changer_voisinage(a.voisinage);
    }
    if a.banc_voisinage {
        return bancs::banc_voisinage(&a, &sim);
    }
    if let Some(dossier) = &a.observer {
        return observation::observer(&a, &mut sim, dossier);
    }
    if a.barricade {
        sim.poser_barricades();
    }
    sim.remettre_max();
    let s0 = sim.stats_champs();
    let c0 = compteurs::total();
    let mut demandes = 0u64;
    let mut admis = 0u64;
    let d: Vec<[Duration; NB_PHASES]> = (0..a.ticks)
        .map(|_| {
            let d = sim.tick();
            let (x, y) = sim.reservations();
            demandes += x;
            admis += y;
            d
        })
        .collect();
    let c1 = compteurs::total();
    let s1 = sim.stats_champs();
    let total = mesure::stats(d.iter().map(|p| p.iter().sum()).collect());
    let ph: [f64; NB_PHASES] =
        std::array::from_fn(|i| mesure::stats(d.iter().map(|p| p[i]).collect()).mediane_ms);
    let req = (c1[compteurs::REQUETES] - c0[compteurs::REQUETES]) as f64;
    let nrec = s1.recalculs - s0.recalculs;
    let champ_us = if nrec > 0 {
        (s1.ns - s0.ns) as f64 / nrec as f64 / 1e3
    } else {
        0.0
    };
    let nobs = s1.obstacles - s0.obstacles;
    let obstacle_us = if nobs > 0 {
        (s1.obstacle_ns - s0.obstacle_ns) as f64 / nobs as f64 / 1e3
    } else {
        0.0
    };
    let somme = black_box(sim.somme_controle());
    let b = sim.bilan();
    let rss = mesure::rss_mo().unwrap_or(f64::NAN);
    let tk = a.ticks as f64;
    eprintln!(
        "{:?} {:?} {:?} {} m {:?} /{} {:?} {:?} {} m réservation {:?}{} ×{} : médiane {:.3} ms (p99 {:.3}, max {:.3}) — {} — champs {:.2} recalculs/tick à {:.0} µs (max {:.0}), obstacle {:.0} µs — réservations {:.0}/{:.0} — poursuite {} relayés {} attirés {} perdus {} errance {} profonds {} — densité {} (contact {}) — murs {}",
        a.scenario,
        a.ville,
        a.champ,
        a.resolution,
        a.regle,
        a.periode_champ,
        a.recalcul,
        a.voisinage,
        a.case_zombies,
        a.reservation,
        if a.barricade { " +barricade" } else { "" },
        a.tiles,
        total.mediane_ms,
        total.p99_ms,
        total.max_ms,
        NOMS_PHASES
            .iter()
            .zip(ph)
            .map(|(n, x)| format!("{n} {x:.3}"))
            .collect::<Vec<_>>()
            .join(" / "),
        nrec as f64 / tk,
        champ_us,
        (s1.max_ns as f64) / 1e3,
        obstacle_us,
        admis as f64 / tk,
        demandes as f64 / tk,
        b.poursuite,
        b.relais,
        b.attire,
        b.perdu,
        b.errance,
        b.profond,
        b.densite_max,
        b.densite_contact,
        b.murs
    );
    println!(
        "{:?},{:?},{:?},{},{},{:?},{},{},{:?},{:?},{},{:?},{},{},{},{},{},{},{},{},{},{},{:.1},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.3},{:.1},{:.1},{:.1},{},{:.1},{:.1},{:.0},{:.0},{},{},{},{},{},{},{},{},{},{:.3},{:016x}",
        a.scenario,
        a.ville,
        a.champ,
        !a.sans_marge,
        a.resolution,
        a.regle,
        a.periode_champ,
        !a.sans_etalement,
        a.recalcul,
        a.voisinage,
        a.case_zombies,
        a.reservation,
        if a.barricade { "apparition" } else if a.barricade_initiale { "initiale" } else { "non" },
        !a.sans_relais,
        !a.sans_profonde,
        a.tiles,
        a.zombies,
        a.humains,
        a.threads,
        a.lot_min,
        a.vue,
        a.prechauffage,
        creation_ms,
        total.mediane_ms,
        total.p99_ms,
        total.max_ms,
        ph[0],
        ph[1],
        ph[2],
        ph[3],
        ph[4],
        ph[5],
        nrec as f64 / tk,
        champ_us,
        s1.max_ns as f64 / 1e3,
        obstacle_us,
        s1.invalidees - s0.invalidees,
        demandes as f64 / tk,
        admis as f64 / tk,
        req / tk,
        rss,
        b.errance,
        b.poursuite,
        b.relais,
        b.attire,
        b.perdu,
        b.profond,
        b.densite_max,
        b.densite_contact,
        b.murs,
        sim.couverture,
        somme
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Params {
        Params {
            seuil_profond: 300.0,
            distance_vue: 200.0,
            ..Params::default()
        }
    }

    fn simulation(sc: Scenario, p: Params, lot_min: usize, threads: usize) -> Simulation {
        Simulation::new(&ConfigSim {
            source: SourceVille::Synthetique,
            scenario: sc,
            tiles: 2,
            zombies: 3_000,
            humains: 200,
            threads,
            lot_min,
            params: p,
        })
    }

    fn somme(sc: Scenario, p: Params, lot_min: usize, ticks: usize, barricade: Option<usize>) -> u64 {
        let mut s = simulation(sc, p, lot_min, 4);
        for t in 0..ticks {
            if barricade == Some(t) {
                s.poser_barricades();
            }
            s.tick();
        }
        s.somme_controle()
    }

    /// Le résultat ne dépend pas du découpage en lots : champs, réservation,
    /// barricade et repli compris.
    #[test]
    fn independant_du_decoupage() {
        for (sc, barricade) in [
            (Scenario::Calme, None),
            (Scenario::Assaut, Some(100)),
            (Scenario::Disperse, None),
            (Scenario::Repli, None),
        ] {
            let p = base();
            let a = somme(sc, p.clone(), 64, 200, barricade);
            assert_eq!(a, somme(sc, p.clone(), 1, 200, barricade), "{sc:?}");
            assert_eq!(a, somme(sc, p, 100_000, 200, barricade), "{sc:?}");
        }
    }

    /// Toutes les structures d'index des zombies (cases de 2 m) donnent la
    /// même simulation, et la bascule en cours de partie aussi.
    #[test]
    fn independant_du_voisinage() {
        for sc in [Scenario::Calme, Scenario::Assaut, Scenario::Disperse] {
            let p = base();
            let a = somme(sc, p.clone(), 64, 200, Some(120));
            for v in [
                voisinage::Structure::Dense32,
                voisinage::Structure::Triee,
                voisinage::Structure::Creuse,
            ] {
                let q = Params {
                    voisinage: v,
                    ..p.clone()
                };
                assert_eq!(a, somme(sc, q, 64, 200, Some(120)), "{sc:?} {v:?}");
            }
            let mut s = simulation(sc, p.clone(), 64, 4);
            for t in 0..200 {
                if t == 120 {
                    s.poser_barricades();
                }
                if t == 100 {
                    s.changer_voisinage(voisinage::Structure::Creuse);
                }
                s.tick();
            }
            assert_eq!(a, s.somme_controle(), "{sc:?} bascule");
        }
    }

    /// Recalcul partiel et complet après la barricade : même simulation.
    #[test]
    fn partiel_comme_complet() {
        let p = base();
        let a = somme(Scenario::Assaut, p.clone(), 64, 300, Some(150));
        let q = Params {
            recalcul: Recalcul::Complet,
            ..p
        };
        assert_eq!(a, somme(Scenario::Assaut, q, 64, 300, Some(150)));
    }

    /// Réservation partout : aucune case au-dessus de la capacité.
    #[test]
    fn reservation_partout() {
        let p = Params {
            reservation: Reservation::Partout,
            ..base()
        };
        let mut s = simulation(Scenario::Assaut, p, 256, 4);
        for _ in 0..1500 {
            s.tick();
        }
        let b = s.bilan();
        assert!(b.densite_max <= 24, "densité {}", b.densite_max);
    }

    /// Réservation : jamais plus que la capacité sur une case au contact où
    /// l'on est entré ; aucun zombie dans les murs, barricade comprise.
    #[test]
    fn reservation_exacte() {
        let mut s = simulation(Scenario::Assaut, base(), 256, 4);
        for t in 0..1500 {
            if t == 600 {
                s.poser_barricades();
            }
            s.tick();
        }
        let b = s.bilan();
        assert!(b.densite_contact <= 24, "contact {}", b.densite_contact);
        assert_eq!(b.murs, 0);
        let (d, admis) = s.reservations();
        assert!(d > 0 && admis <= d, "demandes {d} admis {admis}");
    }

    /// Relais : perte en chaîne au plus fraîcheur + période (P-01).
    #[test]
    fn perte_de_trace_avec_relais() {
        let p = Params {
            profonde: false,
            ..base()
        };
        let delai = p.fraicheur_alerte + p.periode_perception + 2;
        let mut s = Simulation::new(&ConfigSim {
            source: SourceVille::Synthetique,
            scenario: Scenario::Disperse,
            tiles: 1,
            zombies: 20_000,
            humains: 1_000,
            threads: 4,
            lot_min: 1024,
            params: p,
        });
        for _ in 0..400 {
            s.tick();
        }
        let b = s.bilan();
        assert!(b.relais > 0 && b.poursuite > 0);
        s.retirer_humains();
        for _ in 0..delai {
            s.tick();
        }
        let b = s.bilan();
        assert_eq!((b.poursuite, b.relais), (0, 0));
        assert!(b.perdu > 0);
    }
}
