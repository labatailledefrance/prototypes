//! P-02 — Perception des zombies : structure de données des requêtes de
//! voisinage et de ligne de vue, mesurée dans la simulation de P-01 (tiles
//! réelles, relais, veille), avec des humains mobiles.

mod alea;
mod compteurs;
mod grille;
mod index;
mod mesure;
mod sim;
mod ville;

use std::hint::black_box;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};

use index::Structure;
use sim::{ConfigSim, NB_PHASES, NOMS_PHASES, Params, Scenario, Simulation, SourceVille};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Ville {
    /// Îlots haussmanniens générés.
    Synthetique,
    /// Emprises réelles de la BD TOPO (voir p01-tile-paris/outils).
    Reelle,
}

#[derive(Parser)]
#[command(about = "P-02 — perception des zombies")]
struct Args {
    #[arg(long, value_enum, default_value = "assaut")]
    scenario: Scenario,
    #[arg(long, value_enum, default_value = "reelle")]
    ville: Ville,
    #[arg(long, default_value = "../../data/generated/p01-tiles")]
    dossier_tiles: std::path::PathBuf,
    /// Structure d'index des humains.
    #[arg(long, value_enum, default_value = "grille")]
    structure: Structure,
    /// Côté d'une case de l'index (case fine pour la grille hiérarchique).
    #[arg(long, default_value_t = 25.0)]
    case: f32,
    /// Rejet rapide par carte de proximité.
    #[arg(long)]
    rejet: bool,
    /// Humains immobiles (comme P-01) : index construit une seule fois.
    #[arg(long)]
    immobiles: bool,
    #[arg(long)]
    relais: bool,
    /// Filtre du relais : saute les voisinages sans alerte.
    #[arg(long)]
    filtre_relais: bool,
    #[arg(long)]
    veille: bool,
    /// Veille profonde (A-01).
    #[arg(long)]
    profonde: bool,
    #[arg(long, default_value_t = 1)]
    tiles: usize,
    #[arg(long, default_value_t = 20_000)]
    zombies: usize,
    #[arg(long, default_value_t = 1_000)]
    humains: usize,
    #[arg(long, default_value_t = 16)]
    threads: usize,
    #[arg(long, default_value_t = 1024)]
    lot_min: usize,
    /// Perception une fois tous les N ticks.
    #[arg(long, default_value_t = 5)]
    periode: u32,
    #[arg(long, default_value_t = 4)]
    rayons: usize,
    /// Distance de vue maximale (m).
    #[arg(long, default_value_t = 200.0)]
    vue: f32,
    /// Seuil de veille profonde (m) ; par défaut la distance de vue, plus
    /// 100 m de marge (décision P-02 : au-delà de la vue). 300 m dans la
    /// campagne principale, pour une vue de 200 m.
    #[arg(long)]
    seuil_profond: Option<f32>,
    #[arg(long, default_value_t = 200)]
    prechauffage: usize,
    /// Structure utilisée pendant le préchauffage (la plus rapide), avant de
    /// basculer sur la structure mesurée. Sans effet sur le résultat.
    #[arg(long, value_enum, default_value = "kdtree")]
    chauffe: Structure,
    #[arg(long, default_value_t = 200)]
    ticks: usize,
    /// Banc mono-thread de toutes les structures sur l'instantané obtenu
    /// après le préchauffage (au lieu de la mesure de la simulation).
    #[arg(long)]
    banc: bool,
    /// Répétitions du banc.
    #[arg(long, default_value_t = 5)]
    repetitions: usize,
    /// Affiche l'en-tête CSV (simulation, ou banc avec --banc) et quitte.
    #[arg(long)]
    entete: bool,
}

fn config(a: &Args) -> ConfigSim {
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
        params: Params {
            periode_perception: a.periode,
            rayons: a.rayons,
            distance_vue: a.vue,
            seuil_profond: a.seuil_profond.unwrap_or(a.vue + 100.0),
            structure: a.structure,
            case: a.case,
            rejet: a.rejet,
            humains_mobiles: !a.immobiles,
            relais: a.relais,
            filtre_relais: a.filtre_relais,
            veille: a.veille,
            profonde: a.profonde,
            ..Params::default()
        },
    }
}

const ENTETE: &str = "scenario,ville,structure,case,rejet,mobiles,relais,filtre_relais,veille,profonde,tiles,zombies,humains,threads,lot_min,periode,vue,prechauffage,creation_ms,mediane_ms,p99_ms,max_ms,humains_ms,grilles_ms,perception_ms,deplacement_ms,requetes_tick,rejets_tick,cases_req,candidats_req,lignes_req,mreq_s,rss_mo,errance,poursuite,relayes,attire,perdu,profond,densite_max,couverture,somme";
const ENTETE_BANC: &str = "scenario,ville,relais,veille,profonde,prechauffage,structure,case,rejet,requetes,ns_requete,ns_recherche,mreq_s_1thread,reconstruction_us,cases_req,candidats_req,lignes_req,empreinte";

fn banc(a: &Args, sim: &mut Simulation) {
    // Toutes les structures, à plusieurs tailles de case, avec et sans rejet.
    let mut v = vec![];
    for case in [10.0, 25.0, 50.0, 100.0] {
        v.push((Structure::Grille, case, false));
    }
    for case in [5.0, 10.0, 12.5, 25.0, 50.0] {
        v.push((Structure::Anneaux, case, false));
    }
    for case in [6.25, 12.5, 25.0] {
        v.push((Structure::Hierarchique, case, false));
    }
    v.push((Structure::Kdtree, 0.0, false));
    v.push((Structure::Grille, 25.0, true));
    v.push((Structure::Anneaux, 10.0, true));
    v.push((Structure::Anneaux, 12.5, true));
    v.push((Structure::Kdtree, 0.0, true));
    let lignes = sim.banc(&v, a.repetitions);
    let refe = lignes[0].empreinte;
    for l in &lignes {
        assert_eq!(
            l.empreinte, refe,
            "{:?} {} : résultat différent de la référence",
            l.structure, l.case
        );
        eprintln!(
            "{:?} {:?} {:>12?} case {:>5} rejet {:>5} : {:>7.0} ns/requête ({:>7.0} recherche), reconstruction {:>6.1} µs, {:>6.1} cases, {:>7.1} candidats, {:.2} lignes",
            a.scenario,
            a.ville,
            l.structure,
            l.case,
            l.rejet,
            l.ns_requete,
            l.ns_recherche,
            l.reconstruction_us,
            l.cases,
            l.candidats,
            l.lignes
        );
        println!(
            "{:?},{:?},{},{},{},{},{:?},{},{},{},{:.1},{:.1},{:.3},{:.2},{:.2},{:.2},{:.3},{:016x}",
            a.scenario,
            a.ville,
            a.relais,
            a.veille,
            a.profonde,
            a.prechauffage,
            l.structure,
            l.case,
            l.rejet,
            l.requetes,
            l.ns_requete,
            l.ns_recherche,
            1e3 / l.ns_requete,
            l.reconstruction_us,
            l.cases,
            l.candidats,
            l.lignes,
            l.empreinte
        );
    }
}

fn main() {
    let a = Args::parse();
    if a.entete {
        println!("{}", if a.banc { ENTETE_BANC } else { ENTETE });
        return;
    }
    let mut c = config(&a);
    c.params.structure = a.chauffe;
    let t0 = Instant::now();
    let mut sim = Simulation::new(&c);
    let creation_ms = t0.elapsed().as_secs_f64() * 1e3;
    for _ in 0..a.prechauffage {
        sim.tick();
    }
    sim.changer_structure(a.structure, a.case);
    if a.banc {
        return banc(&a, &mut sim);
    }
    let c0 = compteurs::total();
    let d: Vec<[Duration; NB_PHASES]> = (0..a.ticks).map(|_| sim.tick()).collect();
    let c1 = compteurs::total();
    let dc: Vec<f64> = (0..5).map(|i| (c1[i] - c0[i]) as f64).collect();
    let total = mesure::stats(d.iter().map(|p| p.iter().sum()).collect());
    let ph: [f64; NB_PHASES] =
        std::array::from_fn(|i| mesure::stats(d.iter().map(|p| p[i]).collect()).mediane_ms);
    let perception_s: f64 = d.iter().map(|p| p[2].as_secs_f64()).sum();
    let req = dc[compteurs::REQUETES];
    let par_req = |x: f64| if req > 0.0 { x / req } else { 0.0 };
    let mreq_s = req / perception_s / 1e6;
    let somme = black_box(sim.somme_controle());
    let b = sim.bilan();
    let rss = mesure::rss_mo().unwrap_or(f64::NAN);

    eprintln!(
        "{:?} {:?} {:?}/{}{}{}{}{}{} ×{} : médiane {:.3} ms (p99 {:.3}) — {} {:.3} / {} {:.3} / {} {:.3} / {} {:.3} — {:.0} req/tick ({:.0} rejetées), {:.1} cases, {:.1} candidats, {:.2} lignes par requête, {:.1} M req/s — poursuite {} relayés {} attirés {} perdus {} errance {} profonds {} — densité {}",
        a.scenario,
        a.ville,
        a.structure,
        a.case,
        if a.rejet { " +rejet" } else { "" },
        if a.immobiles { " immobiles" } else { "" },
        if a.relais { if a.filtre_relais { " +relais filtré" } else { " +relais" } } else { "" },
        if a.veille { " +veille" } else { "" },
        if a.profonde { " +profonde" } else { "" },
        a.tiles,
        total.mediane_ms,
        total.p99_ms,
        NOMS_PHASES[0],
        ph[0],
        NOMS_PHASES[1],
        ph[1],
        NOMS_PHASES[2],
        ph[2],
        NOMS_PHASES[3],
        ph[3],
        req / a.ticks as f64,
        dc[compteurs::REJETS] / a.ticks as f64,
        par_req(dc[compteurs::CASES]),
        par_req(dc[compteurs::CANDIDATS]),
        par_req(dc[compteurs::LIGNES]),
        mreq_s,
        b.poursuite,
        b.relais,
        b.attire,
        b.perdu,
        b.errance,
        b.profond,
        b.densite_max
    );
    println!(
        "{:?},{:?},{:?},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.1},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.0},{:.0},{:.2},{:.2},{:.3},{:.2},{:.0},{},{},{},{},{},{},{},{:.3},{:016x}",
        a.scenario,
        a.ville,
        a.structure,
        a.case,
        a.rejet,
        !a.immobiles,
        a.relais,
        a.filtre_relais,
        a.veille,
        a.profonde,
        a.tiles,
        a.zombies,
        a.humains,
        a.threads,
        a.lot_min,
        a.periode,
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
        req / a.ticks as f64,
        dc[compteurs::REJETS] / a.ticks as f64,
        par_req(dc[compteurs::CASES]),
        par_req(dc[compteurs::CANDIDATS]),
        par_req(dc[compteurs::LIGNES]),
        mreq_s,
        rss,
        b.errance,
        b.poursuite,
        b.relais,
        b.attire,
        b.perdu,
        b.profond,
        b.densite_max,
        sim.couverture,
        somme
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct V {
        filtre: bool,
        structure: Structure,
        case: f32,
        rejet: bool,
        relais: bool,
        veille: bool,
        profonde: bool,
        lot_min: usize,
    }

    const BASE: V = V {
        filtre: false,
        structure: Structure::Grille,
        case: 25.0,
        rejet: false,
        relais: true,
        veille: true,
        profonde: false,
        lot_min: 64,
    };

    fn somme(sc: Scenario, v: V, ticks: usize) -> u64 {
        let mut s = Simulation::new(&ConfigSim {
            source: SourceVille::Synthetique,
            scenario: sc,
            tiles: 2,
            zombies: 3_000,
            humains: 200,
            threads: 4,
            lot_min: v.lot_min,
            params: Params {
                structure: v.structure,
                case: v.case,
                rejet: v.rejet,
                relais: v.relais,
                filtre_relais: v.filtre,
                veille: v.veille,
                profonde: v.profonde,
                ..Params::default()
            },
        });
        for _ in 0..ticks {
            s.tick();
        }
        s.somme_controle()
    }

    /// Le résultat ne dépend pas du découpage en lots parallèles, y compris
    /// avec humains mobiles et veille profonde.
    #[test]
    fn independant_du_decoupage() {
        for sc in [Scenario::Calme, Scenario::Assaut, Scenario::Disperse] {
            for profonde in [false, true] {
                let v = V { profonde, ..BASE };
                let a = somme(sc, v, 150);
                assert_eq!(a, somme(sc, V { lot_min: 1, ..v }, 150), "{sc:?}");
                assert_eq!(
                    a,
                    somme(
                        sc,
                        V {
                            lot_min: 100_000,
                            ..v
                        },
                        150
                    ),
                    "{sc:?}"
                );
            }
        }
    }

    /// Le résultat ne dépend pas de la structure d'index ni du rejet rapide :
    /// toutes rendent exactement la même cible.
    #[test]
    fn independant_de_la_structure() {
        for sc in [Scenario::Calme, Scenario::Assaut, Scenario::Disperse] {
            for (relais, profonde) in [(false, false), (true, true)] {
                let v = V {
                    relais,
                    profonde,
                    ..BASE
                };
                let a = somme(sc, v, 150);
                for (structure, case) in [
                    (Structure::Grille, 10.0),
                    (Structure::Anneaux, 10.0),
                    (Structure::Anneaux, 25.0),
                    (Structure::Hierarchique, 12.5),
                    (Structure::Kdtree, 0.0),
                ] {
                    for rejet in [false, true] {
                        let w = V {
                            structure,
                            case,
                            rejet,
                            ..v
                        };
                        assert_eq!(
                            a,
                            somme(sc, w, 150),
                            "{sc:?} {structure:?} {case} rejet {rejet}"
                        );
                    }
                }
            }
        }
    }

    /// Changer de structure en cours de partie ne change pas le résultat.
    #[test]
    fn bascule_de_structure() {
        let v = V { profonde: true, ..BASE };
        let a = somme(Scenario::Assaut, v, 200);
        let mut s = Simulation::new(&ConfigSim {
            source: SourceVille::Synthetique,
            scenario: Scenario::Assaut,
            tiles: 2,
            zombies: 3_000,
            humains: 200,
            threads: 4,
            lot_min: v.lot_min,
            params: Params {
                structure: Structure::Kdtree,
                relais: v.relais,
                veille: v.veille,
                profonde: v.profonde,
                ..Params::default()
            },
        });
        for _ in 0..100 {
            s.tick();
        }
        s.changer_structure(Structure::Grille, 25.0);
        for _ in 0..100 {
            s.tick();
        }
        assert_eq!(a, s.somme_controle());
    }

    /// Relais : perte en chaîne au plus fraîcheur + période après la
    /// disparition des humains (repris de P-01).
    #[test]
    fn perte_de_trace_avec_relais() {
        let p = Params {
            relais: true,
            structure: Structure::Anneaux,
            case: 10.0,
            ..Params::default()
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
        assert!(
            b.relais > 0 && b.poursuite > 0,
            "relayés {} poursuite {}",
            b.relais,
            b.poursuite
        );
        s.retirer_humains();
        for _ in 0..delai {
            s.tick();
        }
        let b = s.bilan();
        assert_eq!((b.poursuite, b.relais), (0, 0));
        assert!(b.perdu > 0, "dernière position connue");
    }

    /// Veille profonde : des zombies s'endorment loin du groupe, aucun
    /// zombie ne finit dans un bâtiment, et un bruit les réveille.
    #[test]
    fn veille_profonde() {
        let mut s = Simulation::new(&ConfigSim {
            source: SourceVille::Synthetique,
            scenario: Scenario::Calme,
            tiles: 1,
            zombies: 5_000,
            humains: 200,
            threads: 4,
            lot_min: 256,
            params: Params {
                veille: true,
                relais: true,
                profonde: true,
                ..Params::default()
            },
        });
        for _ in 0..400 {
            s.tick();
        }
        let b = s.bilan();
        assert!(b.profond > 1000, "profonds {}", b.profond);
        assert_eq!(s.zombies_dans_les_murs(), 0);
    }
}
