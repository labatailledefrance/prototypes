//! Tests et micro-bancs (`cargo test --release -- --ignored --nocapture`).

use std::time::Instant;

use crate::alea::hacher;
use crate::colon::*;

fn colon_type(id: u32) -> Colon {
    Colon {
        id,
        maj: 0,
        logement: 0,
        travail: 1,
        j: [20_000, 10_000, 30_000, 40_000, 5_000],
        experience: 100,
        competences: 3,
        malade: (id % 50 == 0) as u8,
        drapeaux: 0,
    }
}

/// Coût d'un appel à `vivre`, un thread, données chaudes et froides.
#[test]
#[ignore]
fn banc_vivre() {
    let p = ParamsCivils::default();
    let taux = vec![[1.0f32; NB_RES]; 1];
    let pos = vec![[0.0, 0.0], [800.0, 300.0]];
    for &n in &[4_500usize, 45_000, 1_000_000] {
        let mut v: Vec<Colon> = (0..n as u32).map(colon_type).collect();
        let mut s = Sorties::default();
        s.preparer(1);
        let ctx = CtxLieu { lieu: 0, colonie: 0, risque: 0.5, chauffe: true, exterieur: false, deces: 0 };
        let mut meilleur = f64::MAX;
        for r in 1..=20u32 {
            let t = r * 20;
            let sec = CtxGlobal::heure(&p, t);
            let g = CtxGlobal {
                p: &p,
                k: p.coefs(),
                maintenant: t,
                seconde_jour: sec,
                temp_air: 10.0,
                taux: &taux,
                pos_lieux: &pos,
                graine: hacher(1, t as u64),
            };
            let t0 = Instant::now();
            for c in v.iter_mut() {
                let _ = std::hint::black_box(vivre(c, &ctx, &g, &mut s));
            }
            meilleur = meilleur.min(t0.elapsed().as_secs_f64() * 1e9 / n as f64);
            s.vider();
        }
        println!("vivre : {n} colons, {meilleur:.1} ns par colon (meilleur de 20)");
    }
}

// ------------------------------------------------------------ Tests de bout en bout

use crate::civils::{Civils, ConfigCivils, Dossiers, Repr};
use crate::lieux::{Batiment, Plan, Usage, implanter};

/// Ville synthétique de 2 × 2 km : logements de 25 places et lieux de
/// travail sur une grille.
fn implantation(colons: usize, periode: u32) -> crate::lieux::Implantation {
    let mut bats = vec![];
    for k in 0..900u32 {
        let (x, y) = ((k % 30) as f64 * 66.0 + 30.0, (k / 30) as f64 * 66.0 + 30.0);
        let travail = k % 7 == 0;
        bats.push(Batiment {
            fid: k,
            pos: [1000.0 + x, 1000.0 + y],
            emprise: 300.0,
            niveaux: 3,
            usage: if travail { Usage::Travail } else { Usage::Logement },
            capacite: 25,
        });
    }
    implanter(
        &bats,
        &Plan {
            colons,
            taille_colonie: colons / 2,
            centre: [2000.0, 2000.0],
            ecart: 800.0,
            cadre: [1000.0, 1000.0, 3000.0, 3000.0],
            part_actifs: 0.6,
            part_chantiers: 0.2,
            periode,
            graine: 42,
        },
    )
}

/// Paramètres durs : maladies et morts fréquentes, pour éprouver les décès.
fn params_durs(periode: u32) -> ParamsCivils {
    ParamsCivils {
        periode,
        heure_debut: 6.5,
        temperature: -10.0,
        chauffage: false,
        maladie_jour: 30.0,
        mort_jour: 200.0,
        ..ParamsCivils::default()
    }
}

fn civils(repr: Repr, lot_min: usize, individus: bool) -> Civils {
    let imp = implantation(5_000, 20);
    let cfg = ConfigCivils {
        repr,
        multi: true,
        individus,
        threads: 4,
        lot_min,
        stocks: [200_000, 400_000, 0, 0, 0, 0],
        livraison: [45_000, 100_000, 0, 0, 0, 0],
    };
    Civils::new(&imp, params_durs(20), &cfg)
}

#[test]
fn conservation_a_chaque_tick() {
    for repr in [Repr::B, Repr::A2, Repr::C] {
        let mut c = civils(repr, 64, false);
        for k in 0..3_000 {
            c.tick();
            assert!(c.conservation(), "{repr:?} : conservation fausse au tick {k}");
            if repr != Repr::C && k % 100 == 0 {
                let ids = c.identifiants();
                assert!(ids.windows(2).all(|w| w[0] < w[1]), "{repr:?} : doublon au tick {k}");
            }
        }
        let st = &c.commun().stats;
        assert!(st.morts > 50, "{repr:?} : trop peu de morts ({})", st.morts);
        if repr != Repr::C {
            assert!(st.departs > 200, "{repr:?} : trop peu de départs ({})", st.departs);
        }
        let corps: u64 = c.commun().corps.iter().flatten().map(|&x| x as u64).sum();
        assert_eq!(corps, st.morts, "{repr:?} : un corps par mort");
        assert_eq!(c.commun().evenements.len() as u64, st.morts, "{repr:?} : un événement par mort");
    }
}

#[test]
fn a1_a2_b_identiques() {
    let mut sommes = vec![];
    for repr in [Repr::A1, Repr::A2, Repr::B, Repr::B] {
        let mut c = civils(repr, 1024, false);
        for _ in 0..2_000 {
            c.tick();
        }
        sommes.push((repr, c.somme(), c.commun().stats.morts));
    }
    for s in &sommes {
        assert_eq!(s.1, sommes[0].1, "{sommes:?}");
    }
}

#[test]
fn independant_du_decoupage() {
    let mut s = vec![];
    for lot in [1, 64, 1024, 100_000] {
        let mut c = civils(Repr::B, lot, false);
        for _ in 0..2_000 {
            c.tick();
        }
        s.push(c.somme());
    }
    assert!(s.iter().all(|&x| x == s[0]), "{s:x?}");
}

fn tous(c: &mut Civils) -> Vec<Colon> {
    let mut v: Vec<Colon> = c.world.resource::<Dossiers>().0.iter().flatten().copied().collect();
    v.extend(c.commun().transit.iter().filter_map(|x| x.0.colon));
    v.extend(c.world.query::<&crate::civils::Fiche>().iter(&c.world).map(|f| f.0));
    v.sort_unstable_by_key(|c| c.id);
    v
}

/// Agrégat → individu → agrégat : sans marche réelle, l'état final est
/// identique à celui d'une simulation sans caméra, colon par colon.
#[test]
fn individualisation_aller_retour() {
    let mut r = civils(Repr::B, 64, true);
    let mut c = civils(Repr::B, 64, true);
    let mut ind = crate::individus::Indiv::new(c.commun(), None, false);
    ind.rayon = 5_000.0;
    ind.rayon_retour = 5_000.0;
    ind.delai = 100;
    ind.lot_ind = 300;
    ind.lot_ret = 300;
    c.world.insert_resource(ind);
    let mut max_ent = 0;
    for k in 0..3_000 {
        // Caméra sur la ville pendant 1 000 ticks, puis ailleurs.
        c.world.resource_mut::<crate::individus::Indiv>().camera =
            Some(if k < 1_000 { [2000.0, 2000.0] } else { [90_000.0, 0.0] });
        c.tick();
        r.tick();
        assert!(c.conservation(), "conservation fausse au tick {k}");
        let e = c.world.query::<&crate::civils::Fiche>().iter(&c.world).count();
        max_ent = max_ent.max(e);
        if k % 50 == 0 {
            let ids = c.identifiants();
            assert!(ids.windows(2).all(|w| w[0] < w[1]), "doublon au tick {k}");
        }
    }
    let vivants = c.vivants() as usize;
    assert!(max_ent * 10 >= vivants * 9, "individualisation incomplète : {max_ent} sur {vivants}");
    let e = c.world.query::<&crate::civils::Fiche>().iter(&c.world).count();
    if e > 0 {
        let mut q = c.world.query::<&crate::civils::Place>();
        let ps: Vec<crate::civils::Place> = q.iter(&c.world).copied().collect();
        let ind = c.world.resource::<crate::individus::Indiv>();
        let mut etats = std::collections::BTreeMap::new();
        for p in ps {
            *etats.entry((ind.etat[p.lieu as usize], p.arrivee > c.tick, ind.membres[p.lieu as usize].len() > 0)).or_insert(0) += 1;
        }
        eprintln!("restants (état, en trajet, dans membres) : {etats:?}");
    }
    assert_eq!(e, 0, "retour à l'agrégat incomplet");
    assert_eq!(tous(&mut c), tous(&mut r));
    assert_eq!(c.somme(), r.somme());
}

#[test]
fn sauvegarde_aller_retour() {
    let mut c = civils(Repr::B, 64, false);
    for _ in 0..1_500 {
        c.tick();
    }
    let t = c.tick;
    let d: Vec<(u32, Vec<Colon>)> = c
        .world
        .resource::<Dossiers>()
        .0
        .iter()
        .enumerate()
        .map(|(l, d)| (l as u32, d.clone()))
        .filter(|(_, d)| !d.is_empty())
        .collect();
    let b = crate::sauve_civils::encoder(&d, t);
    let r = crate::sauve_civils::decoder(&b).unwrap();
    for ((l1, d1), (l2, d2)) in d.iter().zip(&r) {
        assert_eq!(l1, l2);
        let mut a = d1.clone();
        a.sort_unstable_by_key(|c| c.id);
        assert_eq!(&a, d2);
    }
    // Une sauvegarde relue puis réécrite est identique.
    assert_eq!(crate::sauve_civils::encoder(&r, t), b);
}

/// Assaut sur un bloc synthétique de 3 × 3 tiles : conservation des humains
/// et des zombies à chaque tick, un zombie par zombifié.
#[test]
fn assaut_conservation() {
    use crate::carte::{Carte, Geo};
    let geo = Geo { x0: 0, y0: 0, w: 3, h: 3 };
    let mut carte = Carte::synthetique(geo, 3_000);
    carte.tuiles[4].population = 0;
    // Colonie : 40 bâtiments au centre de la tile du milieu.
    let mut bats = vec![];
    for k in 0..40u32 {
        let (x, y) = (1300.0 + (k % 8) as f64 * 50.0, 1350.0 + (k / 8) as f64 * 60.0);
        bats.push(Batiment { fid: k, pos: [x, y], emprise: 200.0, niveaux: 2, usage: Usage::Logement, capacite: 15 });
    }
    let imp = implanter(
        &bats,
        &Plan {
            colons: 500,
            taille_colonie: 500,
            centre: [1500.0, 1500.0],
            ecart: 1.0,
            cadre: [1000.0, 1000.0, 2000.0, 2000.0],
            part_actifs: 0.0,
            part_chantiers: 0.0,
            periode: 20,
            graine: 7,
        },
    );
    let cfg = ConfigCivils {
        repr: Repr::B,
        multi: true,
        individus: false,
        threads: 4,
        lot_min: 64,
        stocks: [200_000, 400_000, 0, 0, 0, 0],
        livraison: [0; 6],
    };
    let p = ParamsCivils { heure_debut: 23.0, ..ParamsCivils::default() };
    let civ = Civils::new(&imp, p, &cfg);
    let sp = crate::sim::Params {
        dossier: std::env::temp_dir().join(format!("p05-test-assaut-{}", std::process::id())),
        rayon_ouie: 200.0,
        ..crate::sim::Params::default()
    };
    let cs = crate::sim::ConfigSim { threads: 4, lot_min: 64, slots_initiaux: 9, params: sp };
    let mut x = crate::assaut::Assaut::new(civ, carte, crate::assaut::ParamsAssaut::default(), &cs, 0.05);
    for k in 0..4_000 {
        x.tick();
        assert!(x.conservation_humains(), "humains : conservation fausse au tick {k}");
        assert!(x.sim.verifier_conservation(), "zombies : conservation fausse au tick {k}");
    }
    let zh = x.civils.commun().stats.zombifies;
    assert_eq!(x.stats.zombifies, zh, "un zombie par zombifié");
    assert!(x.stats.entrees > 0, "aucun zombie n'est entré");
    assert!(zh > 0, "aucun zombifié");
    let _ = std::fs::remove_dir_all(&x.sim.p.dossier);
}
