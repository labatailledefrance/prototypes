//! P-01 — Coût d'une tile parisienne : 20 000 zombies agents dans 1 km² de
//! bâti dense, avec perception, déplacement et collisions ; temps par tick
//! pour 1 à N tiles actives simulées en même temps.

mod alea;
mod grille;
mod mesure;
mod sim;
mod ville;

use std::hint::black_box;
use std::time::{Duration, Instant};

use clap::Parser;

use clap::ValueEnum;

use sim::{ConfigSim, NOMS_PHASES, Params, Scenario, Simulation, SourceVille};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Ville {
    /// Îlots haussmanniens générés.
    Synthetique,
    /// Emprises réelles de la BD TOPO (voir outils/extraire_tiles.py).
    Reelle,
}

#[derive(Parser)]
#[command(about = "P-01 — coût d'une tile parisienne")]
struct Args {
    #[arg(long, value_enum, default_value = "assaut")]
    scenario: Scenario,
    #[arg(long, value_enum, default_value = "synthetique")]
    ville: Ville,
    /// Dossier des tiles réelles.
    #[arg(long, default_value = "../../data/generated/p01-tiles")]
    dossier_tiles: std::path::PathBuf,
    /// Variante : mise en veille des zombies sans stimulus (A-03).
    #[arg(long)]
    veille: bool,
    #[arg(long, default_value_t = 1)]
    tiles: usize,
    /// Zombies par tile.
    #[arg(long, default_value_t = 20_000)]
    zombies: usize,
    /// Humains par tile.
    #[arg(long, default_value_t = 1_000)]
    humains: usize,
    #[arg(long, default_value_t = 16)]
    threads: usize,
    /// Taille minimale d'un lot des requêtes parallèles (F-02).
    #[arg(long, default_value_t = 1024)]
    lot_min: usize,
    /// Perception une fois tous les N ticks.
    #[arg(long, default_value_t = 5)]
    periode: u32,
    /// Lignes de vue tracées au plus par perception.
    #[arg(long, default_value_t = 4)]
    rayons: usize,
    /// Distance de vue maximale (m).
    #[arg(long, default_value_t = 200.0)]
    vue: f32,
    /// Variante : relais de l'alerte entre voisins.
    #[arg(long)]
    relais: bool,
    /// Âge maximal d'une alerte relayée, en secondes de jeu.
    #[arg(long, default_value_t = 3.0)]
    fraicheur: f32,
    /// Ticks simulés sans mesure avant la mesure (60 s de jeu = 1 200).
    #[arg(long, default_value_t = 40)]
    prechauffage: usize,
    #[arg(long, default_value_t = 200)]
    ticks: usize,
    /// Chronologie : mesure par fenêtre de `fenetre` ticks sur toute la durée.
    #[arg(long)]
    chronologie: bool,
    #[arg(long, default_value_t = 100)]
    fenetre: usize,
    /// Affiche l'en-tête CSV et quitte.
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
            relais: a.relais,
            veille: a.veille,
            fraicheur_alerte: (a.fraicheur / sim::DT).round() as u32,
            ..Params::default()
        },
    }
}

/// Médianes du total et de chaque phase, p99 du total.
fn resumer(d: &[[Duration; 3]]) -> (mesure::Stats, [f64; 3]) {
    let total = mesure::stats(d.iter().map(|p| p.iter().sum()).collect());
    let phases = std::array::from_fn(|i| mesure::stats(d.iter().map(|p| p[i]).collect()).mediane_ms);
    (total, phases)
}

fn chronologie(a: &Args, sim: &mut Simulation) {
    println!("scenario,ville,relais,veille,tiles,t_s,mediane_ms,p99_ms,grilles_ms,perception_ms,deplacement_ms,errance,poursuite,relayes,attire,perdu,densite_max");
    let mut tick = 0usize;
    while tick < a.ticks {
        let d: Vec<_> = (0..a.fenetre).map(|_| sim.tick()).collect();
        tick += a.fenetre;
        let (s, ph) = resumer(&d);
        let b = sim.bilan();
        println!(
            "{:?},{:?},{},{},{},{:.0},{:.4},{:.4},{:.4},{:.4},{:.4},{},{},{},{},{},{}",
            a.scenario,
            a.ville,
            a.relais,
            a.veille,
            a.tiles,
            tick as f64 * sim::DT as f64,
            s.mediane_ms,
            s.p99_ms,
            ph[0],
            ph[1],
            ph[2],
            b.errance,
            b.poursuite,
            b.relais,
            b.attire,
            b.perdu,
            b.densite_max
        );
    }
}

fn main() {
    let a = Args::parse();
    if a.entete {
        println!(
            "scenario,ville,relais,veille,fraicheur_s,tiles,zombies,humains,threads,lot_min,periode,rayons,vue,prechauffage,creation_ms,mediane_ms,p99_ms,max_ms,grilles_ms,perception_ms,deplacement_ms,rss_mo,errance,poursuite,relayes,attire,perdu,densite_max,couverture,somme"
        );
        return;
    }
    let t0 = Instant::now();
    let mut sim = Simulation::new(&config(&a));
    let creation_ms = t0.elapsed().as_secs_f64() * 1e3;

    if a.chronologie {
        return chronologie(&a, &mut sim);
    }
    for _ in 0..a.prechauffage {
        sim.tick();
    }
    let d: Vec<_> = (0..a.ticks).map(|_| sim.tick()).collect();
    let (s, ph) = resumer(&d);
    let somme = black_box(sim.somme_controle());
    let b = sim.bilan();
    let rss = mesure::rss_mo().unwrap_or(f64::NAN);

    eprintln!(
        "{:?} {:?}{}{} ×{} tiles, {} threads, lot ≥ {}, perception 1/{} : médiane {:.3} ms (p99 {:.3}) — {} {:.3} / {} {:.3} / {} {:.3} — poursuite {} relayés {} attirés {} perdus {} errance {} — densité max {} / 4 m²",
        a.scenario,
        a.ville,
        if a.relais { " + relais" } else { "" },
        if a.veille { " + veille" } else { "" },
        a.tiles,
        a.threads,
        a.lot_min,
        a.periode,
        s.mediane_ms,
        s.p99_ms,
        NOMS_PHASES[0],
        ph[0],
        NOMS_PHASES[1],
        ph[1],
        NOMS_PHASES[2],
        ph[2],
        b.poursuite,
        b.relais,
        b.attire,
        b.perdu,
        b.errance,
        b.densite_max
    );
    println!(
        "{:?},{:?},{},{},{},{},{},{},{},{},{},{},{},{},{:.1},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.0},{},{},{},{},{},{},{:.3},{:016x}",
        a.scenario,
        a.ville,
        a.relais,
        a.veille,
        a.fraicheur,
        a.tiles,
        a.zombies,
        a.humains,
        a.threads,
        a.lot_min,
        a.periode,
        a.rayons,
        a.vue,
        a.prechauffage,
        creation_ms,
        s.mediane_ms,
        s.p99_ms,
        s.max_ms,
        ph[0],
        ph[1],
        ph[2],
        rss,
        b.errance,
        b.poursuite,
        b.relais,
        b.attire,
        b.perdu,
        b.densite_max,
        sim.couverture,
        somme
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn somme(scenario: Scenario, lot_min: usize, ticks: usize) -> u64 {
        somme_v(scenario, lot_min, ticks, false)
    }

    fn somme_v(scenario: Scenario, lot_min: usize, ticks: usize, relais: bool) -> u64 {
        somme_vv(scenario, lot_min, ticks, relais, false)
    }

    fn somme_vv(scenario: Scenario, lot_min: usize, ticks: usize, relais: bool, veille: bool) -> u64 {
        let mut s = Simulation::new(&ConfigSim {
            source: SourceVille::Synthetique,
            scenario,
            tiles: 2,
            zombies: 3_000,
            humains: 200,
            threads: 4,
            lot_min,
            params: Params {
                relais,
                veille,
                ..Params::default()
            },
        });
        for _ in 0..ticks {
            s.tick();
        }
        s.somme_controle()
    }

    /// Le résultat ne dépend pas du découpage en lots parallèles.
    #[test]
    fn independant_du_decoupage() {
        for sc in [Scenario::Calme, Scenario::Assaut, Scenario::Disperse] {
            let a = somme(sc, 1, 100);
            assert_eq!(a, somme(sc, 64, 100), "{sc:?}");
            assert_eq!(a, somme(sc, 100_000, 100), "{sc:?}");
            let b = somme_v(sc, 1, 100, true);
            assert_eq!(b, somme_v(sc, 64, 100, true), "{sc:?} relais");
            assert_eq!(b, somme_v(sc, 100_000, 100, true), "{sc:?} relais");
            let c = somme_vv(sc, 1, 100, true, true);
            assert_eq!(c, somme_vv(sc, 100_000, 100, true, true), "{sc:?} veille");
        }
    }

    /// Le relais crée bien des zombies relayés, et ils perdent la cible dès
    /// que plus aucune source ne la perçoit (au plus fraîcheur + période).
    #[test]
    fn perte_de_trace_avec_relais() {
        let p = Params {
            relais: true,
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
        assert!(b.relais > 0 && b.poursuite > 0, "relayés {} poursuite {}", b.relais, b.poursuite);
        s.retirer_humains();
        for _ in 0..delai {
            s.tick();
        }
        let b = s.bilan();
        assert_eq!((b.poursuite, b.relais), (0, 0));
        assert!(b.perdu > 0, "dernière position connue");
    }

    /// Aucun zombie ne finit dans un bâtiment.
    #[test]
    fn aucun_zombie_dans_les_murs() {
        let mut s = Simulation::new(&ConfigSim {
            source: SourceVille::Synthetique,
            scenario: Scenario::Assaut,
            tiles: 1,
            zombies: 5_000,
            humains: 200,
            threads: 4,
            lot_min: 256,
            params: Params::default(),
        });
        for _ in 0..400 {
            s.tick();
        }
        assert_eq!(s.zombies_dans_les_murs(), 0);
    }
}
