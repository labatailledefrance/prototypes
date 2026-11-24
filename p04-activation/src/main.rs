//! P-04 — Activation et désactivation des tiles, sur un bloc de tiles réelles
//! de Paris : chantier d'activation étalé, sauvegarde et restauration,
//! passage des frontières, délai de désactivation, migration entre tiles
//! inertes. Reprend la simulation de P-03.

mod alea;
mod bancs;
mod carte;
#[allow(dead_code)] // repris de P-03
mod champ;
mod humains;
#[allow(dead_code)] // repris de P-03
mod index;
#[allow(dead_code)]
mod mesure;
mod observation;
mod sauvegarde;
mod sim;
mod ville;
#[allow(dead_code)] // repris de P-03
mod voisinage;

#[cfg(test)]
mod tests;

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};

use carte::{Carte, Geo};
use humains::{Etape, Humains};
use sauvegarde::Format;
use sim::{ConfigSim, NB_PHASES, NOMS_PHASES, Params, Simulation};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Scenario {
    /// Une colonne d'humains traverse le bloc d'ouest en est.
    Colonne,
    /// Base fixe près d'une frontière ; une patrouille fait des
    /// allers-retours de l'autre côté.
    Base,
    /// Base barricadée qui tire, abandonnée, puis reprise.
    Reprise,
    /// Une colonne rapide traverse, un incinérateur attire les zombies des
    /// tiles inertes, la colonne revient.
    Migration,
}

#[derive(Parser, Clone)]
#[command(about = "P-04 — activation et désactivation des tiles", args_override_self = true)]
pub struct Args {
    #[arg(long, value_enum, default_value = "colonne")]
    pub scenario: Scenario,
    #[arg(long, default_value = "../../data/generated/p01-tiles")]
    pub dossier_tiles: PathBuf,
    /// Zones accessibles précalculées (chaîne de données) ; à produire avec
    /// `--precalculer`.
    #[arg(long, default_value = "../../data/generated/p04-zones")]
    pub dossier_zones: PathBuf,
    /// Sauvegardes des tiles.
    #[arg(long, default_value = "../../data/generated/p04-sauvegardes")]
    pub dossier_sauvegardes: PathBuf,
    /// Bloc : x0, y0 (km Lambert-93), largeur, hauteur (tiles).
    #[arg(long, default_value = "646,6859,11,5", value_delimiter = ',')]
    pub bloc: Vec<i64>,
    /// Multiplicateur de densité (20 000 zombies à 61 % de bâti).
    #[arg(long, default_value_t = 1.0)]
    pub densite: f32,
    #[arg(long, default_value_t = 16)]
    pub threads: usize,
    #[arg(long, default_value_t = 1024)]
    pub lot_min: usize,
    #[arg(long, default_value_t = 16)]
    pub slots: usize,
    /// Délai de désactivation (s de jeu).
    #[arg(long, default_value_t = 60.0)]
    pub delai: f64,
    #[arg(long)]
    pub sans_etalement_activation: bool,
    #[arg(long, default_value_t = 100_000)]
    pub budget_terrain: u64,
    #[arg(long, default_value_t = 5_000)]
    pub budget_placement: usize,
    /// Naissances d'entités par tick (0 : toutes d'un coup).
    #[arg(long, default_value_t = 2000)]
    pub lot_naissance: usize,
    /// Destructions d'entités par tick (0 : toutes d'un coup).
    #[arg(long, default_value_t = 2000)]
    pub lot_destruction: usize,
    /// Désactivations au plus par tick (0 : sans limite).
    #[arg(long, default_value_t = 1)]
    pub desactivations_par_tick: usize,
    /// Entités créées et détruites au démarrage (taille des tables).
    #[arg(long, default_value_t = 250_000)]
    pub prechauffe_entites: usize,
    /// Zone accessible calculée à l'activation (tile seule), sans précalcul.
    #[arg(long)]
    pub zone_locale: bool,
    /// Bord de la zone active traversable par les errants (flux à sens
    /// unique vers les tiles inertes).
    #[arg(long)]
    pub bord_traversable: bool,
    #[arg(long, value_enum, default_value = "zstd")]
    pub format: Format,
    #[arg(long)]
    pub fsync: bool,
    /// Humains de la colonne (ou de la base).
    #[arg(long, default_value_t = 500)]
    pub humains: usize,
    /// Vitesse de la colonne (m/s).
    #[arg(long, default_value_t = 3.0)]
    pub vitesse: f64,
    /// Les humains tirent.
    #[arg(long)]
    pub tir: bool,
    /// Durée de jeu (s) ; par défaut, jusqu'à la fin du scénario.
    #[arg(long)]
    pub duree: Option<f64>,
    /// Pause de la patrouille à chaque bout (s).
    #[arg(long, default_value_t = 60.0)]
    pub pause: f64,
    /// Budget d'un tick (ms) pour compter les dépassements.
    #[arg(long, default_value_t = 25.0)]
    pub budget: f64,
    /// Chronologie par seconde de jeu (CSV).
    #[arg(long)]
    pub chrono: Option<PathBuf>,
    /// Vérifie la conservation toutes les N ticks (0 : jamais).
    #[arg(long, default_value_t = 20)]
    pub verif: u64,
    /// Affiche l'en-tête CSV du mode choisi et quitte.
    #[arg(long)]
    pub entete: bool,
    /// Chaîne de données : précalcule les zones accessibles du bloc.
    #[arg(long)]
    pub precalculer: bool,
    /// Banc de l'activation et de la sauvegarde, tile par tile.
    #[arg(long)]
    pub banc: bool,
    /// Banc : nombre de tiles (les plus bâties du bloc d'abord).
    #[arg(long, default_value_t = 55)]
    pub banc_tiles: usize,
    #[arg(long, default_value_t = 3)]
    pub repetitions: usize,
    /// Observation : images du bloc dans ce dossier, une toutes les N s.
    #[arg(long)]
    pub observer: Option<PathBuf>,
    #[arg(long, default_value_t = 60)]
    pub fenetre: u32,
    #[arg(long, default_value = "obs")]
    pub etiquette: String,
}

pub fn geo(a: &Args) -> Geo {
    Geo { x0: a.bloc[0], y0: a.bloc[1], w: a.bloc[2] as usize, h: a.bloc[3] as usize }
}

pub fn params(a: &Args) -> Params {
    Params {
        delai: (a.delai * 20.0).round() as u64,
        etalement_activation: !a.sans_etalement_activation,
        budget_terrain: a.budget_terrain,
        budget_placement: a.budget_placement,
        lot_naissance: a.lot_naissance,
        lot_destruction: a.lot_destruction,
        desactivations_par_tick: a.desactivations_par_tick,
        prechauffe_entites: a.prechauffe_entites,
        zone_precalculee: !a.zone_locale,
        bord_reflechi: !a.bord_traversable,
        format: a.format,
        fsync: a.fsync,
        dossier: a.dossier_sauvegardes.join(format!("{:?}-{}", a.scenario, std::process::id()).to_lowercase()),
        ..Params::default()
    }
}

/// Point (km Lambert-93) → mètres.
fn km(x: f64, y: f64) -> [f64; 2] {
    [x * 1000.0, y * 1000.0]
}

/// Construit les humains et les réglages du scénario.
pub fn scenario(a: &Args, carte: &mut Carte, p: &mut Params) -> Humains {
    let g = carte.geo;
    let mut h = Humains::new([g.x0 as f64 * 1000.0, g.y0 as f64 * 1000.0]);
    let ym = g.y0 as f64 + g.h as f64 / 2.0;
    let (xa, xb) = (g.x0 as f64 + 0.3, (g.x0 + g.w as i64) as f64 - 0.3);
    match a.scenario {
        Scenario::Colonne => {
            let ch = carte::chemin(carte, km(xa, ym), km(xb, ym));
            h.ajouter(carte, a.humains, vec![Etape::marcher(ch, a.vitesse, 2.0)], false, a.tir);
        }
        Scenario::Base => {
            // Base à 150 m d'une frontière de tile ; patrouille à 150 m de
            // l'autre côté.
            let xf = g.x0 as f64 + (g.w / 2) as f64;
            let base = km(xf - 0.15, ym);
            let loin = km(xf + 0.15, ym);
            let aller = carte::chemin(carte, base, loin);
            let retour: Vec<[f64; 2]> = aller.iter().rev().copied().collect();
            let (bp, lp) = (aller[0], *aller.last().unwrap());
            let pause = (a.pause * 20.0) as u32;
            h.ajouter(carte, a.humains, vec![Etape::Attendre { ticks: u32::MAX, centre: bp, rayon: 60.0 }], false, a.tir);
            h.ajouter(
                carte,
                20,
                vec![
                    Etape::Attendre { ticks: pause, centre: bp, rayon: 20.0 },
                    Etape::marcher(aller, 1.5, 2.0),
                    Etape::Attendre { ticks: pause, centre: lp, rayon: 20.0 },
                    Etape::marcher(retour, 1.5, 2.0),
                ],
                true,
                a.tir,
            );
        }
        Scenario::Reprise => {
            // Base barricadée au centre d'une tile ; elle tire ; abandon à
            // 5 min, retour à 20 min.
            let tb = g.tile(g.w as i32 / 2 - 1, g.h as i32 / 2).unwrap();
            let o = g.origine(tb);
            let centre = sim::centre_accessible(&carte.tuiles[tb].ville, [500.0, 500.0]);
            let anneau = sim::anneau_barricades(&carte.tuiles[tb].ville, &carte.tuiles[tb].zone, centre, 80.0);
            carte.poser_barricades(tb, &anneau, p.pv_barricade);
            let base = [o[0] + centre[0] as f64, o[1] + centre[1] as f64];
            let loin = km(xb - 0.5, ym);
            let aller = carte::chemin(carte, base, loin);
            let retour: Vec<[f64; 2]> = aller.iter().rev().copied().collect();
            let lp = *aller.last().unwrap();
            h.ajouter(
                carte,
                a.humains,
                vec![
                    Etape::Attendre { ticks: 300 * 20, centre: base, rayon: 50.0 },
                    Etape::marcher(aller, 10.0, 1.0),
                    Etape::Attendre { ticks: 600 * 20, centre: lp, rayon: 50.0 },
                    Etape::marcher(retour, 10.0, 1.0),
                    Etape::Attendre { ticks: 300 * 20, centre: base, rayon: 50.0 },
                ],
                false,
                true,
            );
        }
        Scenario::Migration => {
            let aller = carte::chemin(carte, km(xa, ym), km(xb, ym));
            let retour: Vec<[f64; 2]> = aller.iter().rev().copied().collect();
            let (bp, lp) = (aller[0], *aller.last().unwrap());
            h.ajouter(
                carte,
                50,
                vec![
                    Etape::marcher(aller, 10.0, 2.0),
                    Etape::Attendre { ticks: 1800 * 20, centre: lp, rayon: 30.0 },
                    Etape::marcher(retour, 10.0, 2.0),
                    Etape::Attendre { ticks: 120 * 20, centre: bp, rayon: 30.0 },
                ],
                false,
                false,
            );
            // Incinérateur au milieu du bloc : attire les zombies des
            // tiles inertes à 3 km.
            p.stimuli = vec![(km(g.x0 as f64 + g.w as f64 / 2.0, ym), 3000.0)];
        }
    }
    h
}

const ENTETE: &str = "scenario,bord,densite,humains,vitesse,tir,delai_s,etalement,budget_terrain,budget_placement,lot_naissance,lot_destruction,desact_tick,zone,format,fsync,threads,lot_min,chargement_ms,duree_s,ticks,mediane_ms,p99_ms,p999_ms,max_ms,max_chantier_tick_ms,max_entites_tick_ms,ticks_sur_budget,ticks_sur_2_5ms,gestion_ms,humains_ms,grilles_ms,champs_ms,perception_ms,deplacement_ms,admission_ms,frontieres_ms,p99_avec_chantier_ms,max_avec_chantier_ms,p99_sans_chantier_ms,actifs_moy,actifs_max,slots_max,activations,premieres,restaurations,desactivations,activations_min,desactivations_min,chantier_ticks_moy,chantier_ticks_max,chantier_ms_moy,chantier_ms_max,zone_ms,terrain_ms,zombies_ms,naissance_ms_moy,naissance_ms_max,instantane_ms_moy,destruction_ms_moy,destruction_ms_max,encode_ms_moy,ecriture_ms_moy,lecture_ms_moy,decode_ms_moy,octets_moy,zombies_sauves_moy,controles_ok,controles_ko,constructions_ok,constructions_ko,passages,passages_max_tick,vers_chantier,vers_inerte,migres,migration_ms,abattus,conservation_ko,vivants_fin,migration_saturee,passages_refuses,prechauffe,rss_mo,somme";

fn moy(v: &[f64]) -> f64 {
    if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
}
fn maxf(v: &[f64]) -> f64 {
    v.iter().copied().fold(0.0, f64::max)
}

fn main() {
    let a = Args::parse();
    if a.entete {
        println!(
            "{}",
            if a.precalculer {
                bancs::ENTETE_PRECALCUL
            } else if a.banc {
                bancs::ENTETE_BANC
            } else {
                ENTETE
            }
        );
        return;
    }
    if a.precalculer {
        return bancs::precalculer(&a);
    }
    if a.banc {
        return bancs::banc(&a);
    }
    let geo = geo(&a);
    let zones = (!a.zone_locale).then_some(a.dossier_zones.as_path());
    let (mut carte, _) = Carte::reelle(&a.dossier_tiles, geo, zones, a.densite).expect("bloc de tiles réelles");
    let mut p = params(&a);
    let h = scenario(&a, &mut carte, &mut p);
    let dossier = p.dossier.clone();
    let mut sim = Simulation::new(&ConfigSim { threads: a.threads, lot_min: a.lot_min, slots_initiaux: a.slots, params: p }, carte, h);
    // Chargement : jusqu'à ce que toutes les tiles requises soient actives.
    let t0 = Instant::now();
    loop {
        sim.tick();
        if sim.nb_chantiers() == 0 && sim.nb_actifs() > 0 {
            break;
        }
    }
    let chargement_ms = t0.elapsed().as_secs_f64() * 1e3;
    let s0 = sim.stats.clone();
    let tick0 = sim.horloge();
    let max_ticks = a.duree.map(|d| (d * 20.0) as u64).unwrap_or(20 * 3600 * 4);
    let mut d: Vec<[Duration; NB_PHASES]> = vec![];
    let mut chantier_ms: Vec<f64> = vec![];
    let mut entites_ms: Vec<f64> = vec![];
    let mut actifs = 0u64;
    let mut chrono = a.chrono.as_ref().map(|f| {
        let mut w = std::io::BufWriter::new(std::fs::File::create(f).expect("chronologie"));
        writeln!(w, "t_s,max_ms,moy_ms,actifs,chantiers,vivants,activations,desactivations,passages,abattus,pv_barricades,barricades_detruites,etat_tile_base").unwrap();
        w
    });
    let mut obs = a.observer.as_ref().map(|dir| observation::Observateur::new(dir, &a.etiquette, a.fenetre, &sim));
    let (mut smax, mut ssum) = (0f64, 0f64);
    while (sim.horloge() - tick0) < max_ticks {
        if a.duree.is_none() && sim.humains().groupes.iter().all(|g| g.fini()) {
            break;
        }
        sim.entites_tick_ms = 0.0;
        let avant = chantiers_ns(&sim);
        let dt = sim.tick();
        chantier_ms.push((chantiers_ns(&sim).saturating_sub(avant)) as f64 / 1e6);
        entites_ms.push(sim.entites_tick_ms);
        actifs += sim.nb_actifs() as u64;
        let tot: f64 = dt.iter().map(|x| x.as_secs_f64() * 1e3).sum();
        smax = smax.max(tot);
        ssum += tot;
        d.push(dt);
        let k = sim.horloge() - tick0;
        if a.verif > 0 && k % a.verif == 0 {
            sim.verifier_conservation();
        }
        if let Some(o) = obs.as_mut() {
            o.tick(&mut sim);
        }
        if k % 20 == 0
            && let Some(w) = chrono.as_mut()
        {
            let viv: usize = sim.slots().iter().map(|s| s.vivants).sum();
            let (mut pv, mut detruites, mut etat_base) = (0i64, 0usize, "-");
            for (t, tu) in sim.carte().tuiles.iter().enumerate() {
                if tu.barricades.is_empty() {
                    continue;
                }
                for b in &tu.barricades {
                    let v = b.pv.load(std::sync::atomic::Ordering::Relaxed);
                    pv += v.max(0) as i64;
                    detruites += (v <= 0) as usize;
                }
                etat_base = match sim.topo().etat[t] {
                    sim::EtatTuile::Actif(_) => "active",
                    sim::EtatTuile::Chantier(_) => "chantier",
                    sim::EtatTuile::Inerte => "inerte",
                };
            }
            let st = &sim.stats;
            writeln!(
                w,
                "{},{:.3},{:.3},{},{},{},{},{},{},{},{},{},{}",
                k / 20,
                smax,
                ssum / 20.0,
                sim.nb_actifs(),
                sim.nb_chantiers(),
                viv,
                st.activations,
                st.desactivations,
                st.passages,
                st.abattus,
                pv,
                detruites,
                etat_base
            )
            .unwrap();
            (smax, ssum) = (0.0, 0.0);
        }
    }
    sim.finir_ecritures();
    sim.verifier_conservation();
    let n = d.len().max(1);
    let duree_s = n as f64 / 20.0;
    let tot: Vec<Duration> = d.iter().map(|p| p.iter().sum()).collect();
    let st = mesure::stats(tot.clone());
    let p999 = {
        let mut v = tot.clone();
        v.sort_unstable();
        v[((0.999 * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1].as_secs_f64() * 1e3
    };
    let ph: [f64; NB_PHASES] = std::array::from_fn(|i| mesure::stats(d.iter().map(|p| p[i]).collect()).mediane_ms);
    let avec: Vec<Duration> =
        tot.iter().zip(chantier_ms.iter().zip(&entites_ms)).filter(|(_, (c, e))| **c > 0.0 || **e > 0.0).map(|(t, _)| *t).collect();
    let sans: Vec<Duration> =
        tot.iter().zip(chantier_ms.iter().zip(&entites_ms)).filter(|(_, (c, e))| **c == 0.0 && **e == 0.0).map(|(t, _)| *t).collect();
    let (p99a, maxa) = if avec.is_empty() { (0.0, 0.0) } else { let s = mesure::stats(avec); (s.p99_ms, s.max_ms) };
    let p99s = if sans.is_empty() { 0.0 } else { mesure::stats(sans).p99_ms };
    let sur = tot.iter().filter(|x| x.as_secs_f64() * 1e3 > a.budget).count();
    let sur10 = tot.iter().filter(|x| x.as_secs_f64() * 1e3 > 2.5).count();
    let s1 = sim.stats.clone();
    let tranche = |v1: &Vec<f64>, v0: &Vec<f64>| v1[v0.len().min(v1.len())..].to_vec();
    let ct: Vec<f64> = s1.chantier_ticks[s0.chantier_ticks.len()..].iter().map(|&x| x as f64).collect();
    let cm = tranche(&s1.chantier_ms, &s0.chantier_ms);
    let nm = tranche(&s1.naissance_ms, &s0.naissance_ms);
    let dm = tranche(&s1.destruction_ms, &s0.destruction_ms);
    let octets: Vec<f64> = s1.octets.iter().map(|&x| x as f64).collect();
    let zs: Vec<f64> = s1.zombies_sauves.iter().map(|&x| x as f64).collect();
    let somme = sim.somme_controle();
    let vivants: usize = sim.slots().iter().map(|s| s.vivants).sum();
    let minutes = duree_s / 60.0;
    let act = (s1.activations - s0.activations) as f64;
    let des = (s1.desactivations - s0.desactivations) as f64;
    let rss = mesure::rss_mo().unwrap_or(f64::NAN);
    eprintln!(
        "{:?} densité {} : {} ticks ({:.0} s), médiane {:.2} ms, p99 {:.2}, p99.9 {:.2}, max {:.2} ; chantier max/tick {:.2} ms, entités max/tick {:.2} ms ; actifs moy {:.1} (max {}) ; activations {} (dont {} restaurations), désactivations {} ; chantier {:.1} ticks, {:.2} ms ; passages {} (max {}/tick) ; contrôles {}/{} ; constructions {}/{} ; conservation KO {} ; {} — {}",
        a.scenario,
        a.densite,
        n,
        duree_s,
        st.mediane_ms,
        st.p99_ms,
        p999,
        st.max_ms,
        maxf(&chantier_ms),
        maxf(&entites_ms),
        actifs as f64 / n as f64,
        s1.actifs_max,
        act,
        s1.restaurations - s0.restaurations,
        des,
        moy(&ct),
        moy(&cm),
        s1.passages - s0.passages,
        s1.passages_max_tick,
        s1.controles_ok,
        s1.controles_ok + s1.controles_ko,
        s1.constructions_ok,
        s1.constructions_ok + s1.constructions_ko,
        s1.conservation_ko,
        NOMS_PHASES.iter().zip(ph).map(|(n, x)| format!("{n} {x:.2}")).collect::<Vec<_>>().join(" / "),
        dossier.display()
    );
    println!(
        "{:?},{},{},{},{},{},{},{},{},{},{},{},{},{},{:?},{},{},{},{:.1},{:.0},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.2},{},{},{},{},{},{},{:.3},{:.3},{:.2},{:.0},{:.3},{:.3},{:.2},{:.2},{:.2},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.0},{:.0},{},{},{},{},{},{},{},{},{},{:.2},{},{},{},{},{},{},{:.0},{:016x}",
        a.scenario,
        if a.bord_traversable { "traversable" } else { "reflechi" },
        a.densite,
        a.humains,
        a.vitesse,
        a.tir,
        a.delai,
        !a.sans_etalement_activation,
        a.budget_terrain,
        a.budget_placement,
        a.lot_naissance,
        a.lot_destruction,
        a.desactivations_par_tick,
        if a.zone_locale { "locale" } else { "precalculee" },
        a.format,
        a.fsync,
        a.threads,
        a.lot_min,
        chargement_ms,
        duree_s,
        n,
        st.mediane_ms,
        st.p99_ms,
        p999,
        st.max_ms,
        maxf(&chantier_ms),
        maxf(&entites_ms),
        sur,
        sur10,
        ph[0],
        ph[1],
        ph[2],
        ph[3],
        ph[4],
        ph[5],
        ph[6],
        ph[7],
        p99a,
        maxa,
        p99s,
        actifs as f64 / n as f64,
        s1.actifs_max,
        s1.slots_max,
        act,
        s1.premieres - s0.premieres,
        s1.restaurations - s0.restaurations,
        des,
        act / minutes,
        des / minutes,
        moy(&ct),
        ct.iter().copied().fold(0.0, f64::max),
        moy(&cm),
        maxf(&cm),
        s1.etapes_ms[0] - s0.etapes_ms[0],
        s1.etapes_ms[1] - s0.etapes_ms[1],
        s1.etapes_ms[2] - s0.etapes_ms[2],
        moy(&nm),
        maxf(&nm),
        moy(&s1.instantane_ms),
        moy(&dm),
        maxf(&dm),
        moy(&s1.encode_ms),
        moy(&s1.ecriture_ms),
        moy(&s1.lecture_ms),
        moy(&s1.decode_ms),
        moy(&octets),
        moy(&zs),
        s1.controles_ok,
        s1.controles_ko,
        s1.constructions_ok,
        s1.constructions_ko,
        s1.passages - s0.passages,
        s1.passages_max_tick,
        s1.vers_chantier - s0.vers_chantier,
        s1.vers_inerte - s0.vers_inerte,
        s1.migres,
        s1.migration_ms,
        s1.abattus,
        s1.conservation_ko,
        vivants,
        s1.migration_saturee,
        s1.passages_refuses,
        a.prechauffe_entites,
        rss,
        somme
    );
    if let Some(o) = obs {
        o.finir();
    }
    let _ = std::fs::remove_dir_all(&dossier);
}

/// Travail cumulé des chantiers en cours (ns).
fn chantiers_ns(sim: &Simulation) -> u64 {
    sim.slots().iter().filter_map(|s| s.chantier.as_ref()).map(|c| c.ns).sum::<u64>()
        + sim.stats.chantier_ns
}
