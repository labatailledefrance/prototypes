//! P-00 — Socle ECS : temps par tick de bevy_ecs contre une référence en
//! Rust brut, pour N entités (position, vitesse, quatre jauges).

mod bevy_bench;
mod brut_bench;
mod etats_bench;
mod logique;
mod mesure;

use std::hint::black_box;
use std::time::Instant;

use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Impl {
    Bevy,
    Brut,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Disposition {
    /// Une jauge = un composant (ou un tableau).
    Separees,
    /// Les quatre jauges dans un seul composant.
    Groupees,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Mode {
    /// Code séquentiel, un seul cœur.
    Mono,
    /// Code parallèle, `--threads` threads.
    Par,
}

pub trait Banc {
    fn tick(&mut self);
    fn somme_controle(&mut self) -> u64;
}

#[derive(Parser)]
#[command(about = "P-00 — banc d'essai ECS")]
struct Args {
    #[arg(long, value_enum, default_value = "bevy")]
    r#impl: Impl,
    #[arg(long, value_enum, default_value = "separees")]
    jauges: Disposition,
    #[arg(long, value_enum, default_value = "par")]
    mode: Mode,
    #[arg(long, default_value_t = 16)]
    threads: usize,
    #[arg(long, default_value_t = 1_000_000)]
    entites: u64,
    #[arg(long, default_value_t = 20)]
    prechauffage: usize,
    #[arg(long, default_value_t = 200)]
    ticks: usize,
    /// Bevy : contourne la détection de changement (horodatage des écritures).
    #[arg(long)]
    sans_detection: bool,
    /// Bevy : nombre d'archétypes (types d'entités) entre lesquels répartir
    /// les entités, de 1 à 1 024.
    #[arg(long, default_value_t = 1)]
    archetypes: usize,
    /// Bevy : taille minimale d'un lot parallèle (1 = comportement par défaut).
    #[arg(long, default_value_t = 1)]
    lot_min: usize,
    /// Test 3 : représentation d'un état fréquent (« en feu »). Si présent,
    /// lance le test 3 au lieu du test 1 (Bevy, multi-thread).
    #[arg(long, value_enum)]
    etat: Option<etats_bench::Representation>,
    /// Test 3 : part des entités qui changent d'état à chaque tick.
    #[arg(long, default_value_t = 0.01)]
    changements: f64,
    /// Affiche l'en-tête CSV et quitte.
    #[arg(long)]
    entete: bool,
}

#[allow(clippy::too_many_arguments)] // prototype jetable
pub fn construire(
    i: Impl,
    n: u64,
    d: Disposition,
    m: Mode,
    threads: usize,
    detection: bool,
    archetypes: usize,
    lot_min: usize,
) -> Box<dyn Banc> {
    match i {
        Impl::Bevy => Box::new(bevy_bench::BancBevy::new(
            n, d, m, threads, detection, archetypes, lot_min,
        )),
        Impl::Brut => Box::new(brut_bench::BancBrut::new(n, d, m, threads)),
    }
}

/// Préchauffage, puis mesure de `ticks` ticks. Renvoie les statistiques, la
/// somme de contrôle et la mémoire résidente.
fn mesurer(banc: &mut dyn Banc, prechauffage: usize, ticks: usize) -> (mesure::Stats, u64, f64) {
    for _ in 0..prechauffage {
        banc.tick();
    }
    let mut durees = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        let t = Instant::now();
        banc.tick();
        durees.push(t.elapsed());
    }
    let somme = black_box(banc.somme_controle());
    let rss = mesure::rss_mo().unwrap_or(f64::NAN);
    (mesure::stats(durees), somme, rss)
}

/// Test 3 : état fréquent, marqueur contre champ.
fn test_etats(a: &Args, repr: etats_bench::Representation) {
    if a.entete {
        println!(
            "etat,changements,threads,entites,creation_ms,mediane_ms,p99_ms,max_ms,moyenne_ms,rss_mo,somme"
        );
        return;
    }
    let t0 = Instant::now();
    let mut banc = etats_bench::BancEtats::new(a.entites, repr, a.threads, a.changements);
    let creation_ms = t0.elapsed().as_secs_f64() * 1e3;
    let (s, somme, rss) = mesurer(&mut banc, a.prechauffage, a.ticks);
    eprintln!(
        "État {:?}, {} % de changements par tick ×{} — {} entités : médiane {:.3} ms, p99 {:.3} ms",
        repr,
        a.changements * 100.0,
        a.threads,
        a.entites,
        s.mediane_ms,
        s.p99_ms
    );
    println!(
        "{:?},{},{},{},{:.1},{:.4},{:.4},{:.4},{:.4},{:.0},{:016x}",
        repr,
        a.changements,
        a.threads,
        a.entites,
        creation_ms,
        s.mediane_ms,
        s.p99_ms,
        s.max_ms,
        s.moyenne_ms,
        rss,
        somme
    );
}

fn main() {
    let a = Args::parse();
    if let Some(repr) = a.etat {
        return test_etats(&a, repr);
    }
    if a.entete {
        println!(
            "impl,detection,archetypes,lot_min,jauges,mode,threads,entites,creation_ms,mediane_ms,p99_ms,max_ms,moyenne_ms,rss_mo,somme"
        );
        return;
    }
    let threads = match a.mode {
        Mode::Mono => 1,
        Mode::Par => a.threads,
    };

    let t0 = Instant::now();
    let detection = matches!(a.r#impl, Impl::Bevy) && !a.sans_detection;
    let archetypes = match a.r#impl {
        Impl::Bevy => a.archetypes,
        Impl::Brut => 1,
    };
    let mut banc = construire(
        a.r#impl, a.entites, a.jauges, a.mode, threads, detection, archetypes, a.lot_min,
    );
    let creation_ms = t0.elapsed().as_secs_f64() * 1e3;

    let (s, somme, rss) = mesurer(banc.as_mut(), a.prechauffage, a.ticks);

    eprintln!(
        "{:?}{} [{} arch., lot ≥ {}] {:?} {:?} ×{} — {} entités : médiane {:.3} ms, p99 {:.3} ms, RSS {:.0} Mo",
        a.r#impl,
        if detection { "" } else { " (sans détection)" },
        archetypes,
        a.lot_min,
        a.jauges,
        a.mode,
        threads,
        a.entites,
        s.mediane_ms,
        s.p99_ms,
        rss
    );
    println!(
        "{:?},{},{},{},{:?},{:?},{},{},{:.1},{:.4},{:.4},{:.4},{:.4},{:.0},{:016x}",
        a.r#impl,
        detection,
        archetypes,
        a.lot_min,
        a.jauges,
        a.mode,
        threads,
        a.entites,
        creation_ms,
        s.mediane_ms,
        s.p99_ms,
        s.max_ms,
        s.moyenne_ms,
        rss,
        somme
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les deux implémentations, dans toutes leurs variantes, doivent faire
    /// exactement le même calcul : même somme de contrôle après N ticks.
    #[test]
    fn memes_sommes_de_controle() {
        let n = 10_000;
        let mut attendu = None;
        for (i, det) in [(Impl::Brut, false), (Impl::Bevy, true), (Impl::Bevy, false)] {
            for d in [Disposition::Separees, Disposition::Groupees] {
                for m in [Mode::Mono, Mode::Par] {
                    let mut b = construire(i, n, d, m, 4, det, 1, 1);
                    for _ in 0..100 {
                        b.tick();
                    }
                    let s = b.somme_controle();
                    match attendu {
                        None => attendu = Some(s),
                        Some(a) => assert_eq!(a, s, "{i:?} {det} {d:?} {m:?}"),
                    }
                }
            }
        }
        // La fragmentation en archétypes ne doit rien changer au calcul.
        for (k, lot) in [(7, 1), (1024, 1), (1024, 4096)] {
            for m in [Mode::Mono, Mode::Par] {
                let mut b = construire(Impl::Bevy, n, Disposition::Groupees, m, 4, true, k, lot);
                for _ in 0..100 {
                    b.tick();
                }
                assert_eq!(attendu, Some(b.somme_controle()), "{k} archétypes, {m:?}");
            }
        }
    }

    /// Test 3 : marqueur et champ doivent donner exactement le même état.
    #[test]
    fn etats_marqueur_egal_champ() {
        use etats_bench::{BancEtats, Representation};
        for changements in [0.0, 0.01, 0.1] {
            let sommes: Vec<u64> = [Representation::Marqueur, Representation::Champ]
                .into_iter()
                .map(|r| {
                    let mut b = BancEtats::new(10_000, r, 4, changements);
                    for _ in 0..100 {
                        b.tick();
                    }
                    b.somme_controle()
                })
                .collect();
            assert_eq!(sommes[0], sommes[1], "changements {changements}");
        }
    }
}
