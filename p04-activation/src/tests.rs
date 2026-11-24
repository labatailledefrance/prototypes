//! Tests de bout en bout sur un bloc synthétique (sans données réelles).

use crate::carte::{self, Carte, Geo};
use crate::humains::{Etape, Humains};
use crate::sauvegarde::{self, Cote, Mouvement, ZSauve};
use crate::sim::{ConfigSim, Params, Simulation};

fn bloc() -> Carte {
    Carte::synthetique(Geo { x0: 0, y0: 0, w: 3, h: 2 }, 1500)
}

/// Une colonne rapide traverse le bloc, attend, revient ; délai court :
/// les tiles s'activent, se sauvegardent et se restaurent plusieurs fois.
fn simulation(lot_min: usize, etalement: bool, migration: bool, tir: bool) -> Simulation {
    let mut carte = bloc();
    let a = [100.0, 900.0];
    let b = [2900.0, 1100.0];
    let aller = carte::chemin(&carte, a, b);
    let retour: Vec<[f64; 2]> = aller.iter().rev().copied().collect();
    let (pa, pb) = (aller[0], *aller.last().unwrap());
    let mut h = Humains::new([0.0, 0.0]);
    h.ajouter(
        &carte,
        30,
        vec![
            Etape::marcher(aller, 15.0, 2.0),
            Etape::Attendre { ticks: 600, centre: pb, rayon: 20.0 },
            Etape::marcher(retour, 15.0, 2.0),
            Etape::Attendre { ticks: 400, centre: pa, rayon: 20.0 },
        ],
        false,
        tir,
    );
    let p = Params {
        delai: 100,
        etalement_activation: etalement,
        budget_terrain: 50_000,
        budget_placement: 300,
        lot_naissance: if etalement { 500 } else { 0 },
        lot_destruction: if etalement { 700 } else { 0 },
        dossier: std::env::temp_dir().join(format!("p04-test-{}-{lot_min}-{etalement}-{migration}-{tir}", std::process::id())),
        stimuli: if migration { vec![([1500.0, 1000.0], 3000.0)] } else { vec![] },
        periode_migration: 200,
        taux_migration: 0.05,
        ..Params::default()
    };
    let _ = carte.tuiles.iter_mut().count();
    Simulation::new(&ConfigSim { threads: 4, lot_min, slots_initiaux: 4, params: p }, carte, h)
}

fn jouer(s: &mut Simulation, verifier: bool) -> u64 {
    while !s.humains().groupes.iter().all(|g| g.fini()) {
        s.tick();
        if verifier {
            assert!(s.verifier_conservation(), "conservation au tick {}", s.horloge());
        }
    }
    s.finir_ecritures();
    s.somme_controle()
}

/// Conservation exacte à chaque tick (activation, désactivation,
/// restauration, passages, migration, tirs) ; toutes les restaurations
/// sont identiques bit à bit à la sauvegarde.
#[test]
fn conservation_et_restauration() {
    for (migration, tir) in [(false, false), (true, true)] {
        let mut s = simulation(256, true, migration, tir);
        jouer(&mut s, true);
        let st = &s.stats;
        eprintln!("{st:?}");
        assert!(st.desactivations >= 2 && st.restaurations >= 2, "{} / {}", st.desactivations, st.restaurations);
        assert!(st.passages > 0, "aucun passage de frontière");
        assert_eq!(st.controles_ko, 0);
        assert!(st.controles_ok >= 2);
        assert_eq!(st.constructions_ko, 0);
        if migration {
            assert!(st.migres > 0);
        }
        if tir {
            assert!(st.abattus > 0);
        }
    }
}

/// Résultat indépendant du découpage en lots.
#[test]
fn independant_du_decoupage() {
    let a = jouer(&mut simulation(64, true, true, true), false);
    assert_eq!(a, jouer(&mut simulation(1, true, true, true), false));
    assert_eq!(a, jouer(&mut simulation(100_000, true, true, true), false));
}

/// Même graine, mêmes positions : deux simulations identiques donnent le
/// même état, y compris sans étalement de l'activation.
#[test]
fn meme_graine_memes_positions() {
    for e in [true, false] {
        let mut a = simulation(256, e, false, false);
        let mut b = simulation(256, e, false, false);
        for _ in 0..400 {
            a.tick();
            b.tick();
        }
        assert_eq!(a.somme_controle(), b.somme_controle());
    }
}

/// Journal : les départs sont les plus proches du bord de sortie, les
/// arrivées sont près du bord d'entrée.
#[test]
fn journal_de_migration() {
    let carte = bloc();
    let mut z: Vec<ZSauve> = (0..1000)
        .map(|i| ZSauve::de([(i as f32 * 0.997) % 1000.0, (i as f32 * 7.31) % 1000.0], 0.0))
        .collect();
    z.sort_unstable();
    let est = Cote { dx: 1, dy: 0 };
    let ouest = Cote { dx: -1, dy: 0 };
    let l = sauvegarde::rejouer(
        z.clone(),
        &[Mouvement::Depart { cote: est, k: 100 }, Mouvement::Arrivee { cote: ouest, k: 50, graine: 3 }],
        &carte,
        0,
    );
    assert_eq!(l.len(), 950);
    let xmax_restant = l[..900].iter().map(|z| z.0[0]).fold(0f32, f32::max);
    let mut xs: Vec<f32> = z.iter().map(|z| z.pos()[0]).collect();
    xs.sort_by(|a, b| b.total_cmp(a));
    assert!(xmax_restant <= xs[99], "un départ n'était pas parmi les plus proches du bord");
    assert!(l[900..].iter().all(|z| z.0[0] < sauvegarde::BANDE), "arrivée loin du bord d'entrée");
}

/// Le terrain construit par tranches est identique au terrain d'un coup.
#[test]
fn terrain_etale_identique() {
    use crate::champ::{ChantierTerrain, Regle, Terrain};
    let carte = bloc();
    let f = |x, y| carte.fin(1, x, y);
    let reference = Terrain::depuis(2, Regle::Majorite, true, true, f);
    for budget in [1u64, 5_000, 123_457, u64::MAX] {
        let mut t = Terrain::vide(2, Regle::Majorite, true);
        let mut ch = ChantierTerrain::default();
        while !t.avancer_chantier(&mut ch, budget, &f) {}
        assert!(t.cout == reference.cout && t.masque == reference.masque, "budget {budget}");
    }
}

/// Micro-banc (ignoré) : coût du rejeu d'un journal chargé, par type
/// d'événement, sur une tile réelle.
#[test]
#[ignore]
fn banc_rejeu() {
    use std::time::Instant;
    let geo = Geo { x0: 650, y0: 6861, w: 1, h: 1 };
    let (carte, _) = Carte::reelle(
        std::path::Path::new("../../data/generated/p01-tiles"),
        geo,
        Some(std::path::Path::new("../../data/generated/p04-zones")),
        1.0,
    )
    .unwrap();
    let z: Vec<ZSauve> = (0..15_000).map(|i| ZSauve::de(crate::sim::placer(&carte.tuiles[0].ville, &carte.tuiles[0].zone, 0, i).0, 0.0)).collect();
    for (nom, m) in [
        ("depart_est", Mouvement::Depart { cote: Cote { dx: 1, dy: 0 }, k: 300 }),
        ("arrivee_ouest", Mouvement::Arrivee { cote: Cote { dx: -1, dy: 0 }, k: 300, graine: 1 }),
        ("arrivee_coin", Mouvement::Arrivee { cote: Cote { dx: -1, dy: -1 }, k: 300, graine: 1 }),
    ] {
        let t0 = Instant::now();
        let l = sauvegarde::rejouer(z.clone(), &vec![m; 30], &carte, 0);
        eprintln!("{nom} : 30 × 300 en {:.2} ms ({} zombies)", t0.elapsed().as_secs_f64() * 1e3, l.len());
    }
}
