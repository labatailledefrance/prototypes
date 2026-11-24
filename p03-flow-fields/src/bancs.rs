//! Bancs mono-thread : champs (calcul, recalcul, obstacles, connexité),
//! frontières entre tiles (échange de bordures), index des zombies.

use std::time::Instant;

use crate::Args;
use crate::alea::{Alea, hacher};
use crate::champ::{self, Champ, Dial, Echange, Fin, Graine, Regle, Terrain, INF};
use crate::sim::{self, Simulation};
use crate::ville::{COTE, COTE_F, Ville};
use crate::voisinage::{Structure, Voisinage};

pub const ENTETE_CHAMPS: &str =
    "tile,couverture,resolution,regle,cas,sources,ms,fixees,invalidees,egal,perte_pct,ns_direction,octets";
pub const ENTETE_FRONTIERES: &str =
    "resolution,cas,rondes,ms_ronde,ms_combine,ecarts,ecart_max_m,cases_finies,dmin_m";
pub const ENTETE_VOISINAGE: &str =
    "scenario,tiles,structure,case,indexes,construction_us,requete_ns,octets,empreinte";

fn liste_tiles(a: &Args) -> Vec<(i64, i64)> {
    let csv = std::fs::read_to_string(a.dossier_tiles.join("tiles.csv")).expect("tiles.csv");
    csv.lines()
        .skip(1)
        .map(|l| {
            let mut f = l.split(',');
            (f.next().unwrap().parse().unwrap(), f.next().unwrap().parse().unwrap())
        })
        .collect()
}

fn charger(a: &Args, x: i64, y: i64) -> Option<Ville> {
    Ville::charger(&a.dossier_tiles.join(format!("x{x}_y{y}.bin"))).ok()
}

fn mediane(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Points tirés dans la zone accessible, dans un disque.
fn points(v: &Ville, zone: &[u64], a: &mut Alea, centre: [f32; 2], rayon: f32, n: usize) -> Vec<[f32; 2]> {
    (0..n)
        .map(|_| loop {
            let q = v.point_libre(a, centre, rayon);
            if sim::dans_zone(zone, q) {
                break q;
            }
        })
        .collect()
}

fn graines(t: &Terrain, pts: &[[f32; 2]]) -> Vec<Graine> {
    let mut g: Vec<Graine> = pts
        .iter()
        .map(|&p| Graine {
            i: t.indice(p) as u32,
            d: 0,
            ts: 0,
        })
        .collect();
    g.sort_unstable_by_key(|g| g.i);
    g.dedup_by_key(|g| g.i);
    g
}

/// Banc des champs, sur les `a.tiles` premières tiles réelles.
pub fn banc_champs(a: &Args) {
    let liste = liste_tiles(a);
    let reps = a.repetitions.max(1);
    let variantes = [
        (1, Regle::Majorite),
        (2, Regle::Majorite),
        (2, Regle::Toute),
        (4, Regle::Majorite),
        (5, Regle::Majorite),
        (5, Regle::Toute),
    ];
    for (k, &(x, y)) in liste.iter().take(a.tiles).enumerate() {
        let ville = charger(a, x, y).expect("tile");
        let centre = sim::centre_accessible(&ville, [COTE_F / 2.0, COTE_F / 2.0]);
        let (t1, c1) = sim::champ_1m(&ville, &[centre]);
        let zone = sim::zone_de(&t1, &c1);
        let nzone = zone.iter().map(|w| w.count_ones() as usize).sum::<usize>();
        let mut al = Alea::new(hacher(0xBA4C, k as u64));
        let groupe = points(&ville, &zone, &mut al, centre, sim::RAYON_GROUPE, 1000);
        let disperse = points(&ville, &zone, &mut al, centre, 710.0, 1000);
        let essais = points(&ville, &zone, &mut al, centre, 710.0, 20_000);
        let rue = sim::barricade_rue(&ville, &zone, centre, 200.0);
        let anneau = sim::anneau_barricades(&ville, &zone, centre);
        let nom = format!("x{x}_y{y}");
        for &(res, regle) in &variantes {
            let ligne = |cas: &str, src: usize, ms: f64, fixees: u64, inval: u64, egal: &str, perte: f64, ns: f64, octets: usize| {
                println!(
                    "{nom},{:.3},{res},{regle:?},{cas},{src},{ms:.4},{fixees},{inval},{egal},{perte:.3},{ns:.1},{octets}",
                    ville.couverture
                );
                eprintln!(
                    "{nom} {res} m {regle:?} {cas:>22} : {ms:>8.3} ms, {fixees:>7} fixées, {inval:>7} invalidées {egal} perte {perte:.2} % {ns:.0} ns"
                );
            };
            // Terrain.
            let mut tt = vec![];
            let mut terrain = None;
            for _ in 0..reps {
                let t0 = Instant::now();
                terrain = Some(Terrain::de_ville(&ville, res, regle, !a.sans_marge));
                tt.push(t0.elapsed().as_secs_f64() * 1e3);
            }
            let t = terrain.unwrap();
            // Connexité : part de la zone accessible (1 m) dont la case n'est
            // pas atteinte depuis le centre.
            let mut c = Champ::new(&t);
            let mut d = Dial::new(&t);
            d.calculer(&t, &mut c, &graines(&t, &[centre]));
            let mut perdues = 0usize;
            for yy in 0..COTE {
                for xx in 0..COTE {
                    let q = [xx as f32 + 0.5, yy as f32 + 0.5];
                    if sim::dans_zone(&zone, q) && c.dist[t.indice(q)] == INF {
                        perdues += 1;
                    }
                }
            }
            let octets_champ = t.w * t.w * 4;
            ligne(
                "terrain",
                0,
                mediane(tt),
                t.libres() as u64,
                0,
                "",
                100.0 * perdues as f64 / nzone as f64,
                0.0,
                t.w * t.w,
            );
            // Calcul complet selon les sources.
            let mut base = Champ::new(&t);
            for (cas, pts) in [("centre", vec![centre]), ("groupe", groupe.clone()), ("disperse", disperse.clone())] {
                let g = graines(&t, &pts);
                let mut tm = vec![];
                let mut w = champ::Travail::default();
                for _ in 0..reps {
                    let t0 = Instant::now();
                    w = d.calculer(&t, &mut c, &g);
                    tm.push(t0.elapsed().as_secs_f64() * 1e3);
                }
                ligne(&format!("complet_{cas}"), g.len(), mediane(tm), w.fixees, 0, "", 0.0, 0.0, octets_champ);
                if cas == "groupe" {
                    base = c.clone();
                }
            }
            // Direction : coût d'une lecture.
            let t0 = Instant::now();
            let mut somme = 0f32;
            for _ in 0..reps {
                for &q in &essais {
                    if let Some(v) = base.direction(&t, q) {
                        somme += v[0];
                    }
                }
            }
            std::hint::black_box(somme);
            let ns = t0.elapsed().as_secs_f64() * 1e9 / (reps * essais.len()) as f64;
            ligne("direction", 0, 0.0, 0, 0, "", 0.0, ns, 0);
            // Obstacles, sur le champ du groupe.
            let g = graines(&t, &groupe);
            for (cas, cases) in [("rue", &rue), ("anneau", &anneau)] {
                if cases.is_empty() {
                    continue;
                }
                let mut v2 = Ville::charger(&a.dossier_tiles.join(format!("{nom}.bin"))).unwrap();
                let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                for &(bx, by) in cases.iter() {
                    v2.poser_barricade(bx as usize, by as usize);
                    let (bx, by) = (bx as i32, by as i32);
                    (x0, y0, x1, y1) = (x0.min(bx), y0.min(by), x1.max(bx), y1.max(by));
                }
                let mut t2 = t.clone();
                let maj = t2.mettre_a_jour(x0, y0, x1, y1, |x, y| champ::fin_de(&v2, x, y));
                assert!(maj.baisses.is_empty());
                let modif = maj.hausses;
                // Partiel (hausse).
                let (mut tp, mut wp) = (vec![], champ::Travail::default());
                let mut cp = base.clone();
                for _ in 0..reps {
                    cp = base.clone();
                    let t0 = Instant::now();
                    wp = d.recalcul_hausse(&t2, &mut cp, &modif, &g);
                    tp.push(t0.elapsed().as_secs_f64() * 1e3);
                }
                let (mut tc, mut wc) = (vec![], champ::Travail::default());
                let mut cc = base.clone();
                for _ in 0..reps {
                    let t0 = Instant::now();
                    wc = d.calculer(&t2, &mut cc, &g);
                    tc.push(t0.elapsed().as_secs_f64() * 1e3);
                }
                let egal = if cp.dist == cc.dist { "oui" } else { "NON" };
                ligne(&format!("{cas}_partiel"), modif.len(), mediane(tp), wp.fixees, wp.invalidees, egal, 0.0, 0.0, 0);
                ligne(&format!("{cas}_complet"), modif.len(), mediane(tc), wc.fixees, 0, "", 0.0, 0.0, 0);
                // Barricade détruite : retour au terrain initial.
                let (mut tb, mut wb) = (vec![], champ::Travail::default());
                let mut cb = cc.clone();
                for _ in 0..reps {
                    cb = cc.clone();
                    let t0 = Instant::now();
                    wb = d.recalcul_baisse(&t, &mut cb, &modif);
                    tb.push(t0.elapsed().as_secs_f64() * 1e3);
                }
                let egal = if cb.dist == base.dist { "oui" } else { "NON" };
                ligne(&format!("{cas}_detruite_partiel"), modif.len(), mediane(tb), wb.fixees, 0, egal, 0.0, 0.0, 0);
            }
        }
    }
}

/// Banc des frontières : bloc de 3 × 3 tiles réelles adjacentes, groupe
/// d'humains au centre de la tile du milieu. Chaque tile calcule son champ
/// avec, en graines de halo, les bordures exportées par ses voisines à la
/// ronde précédente (vecteur de distances entre tiles). On compare au champ
/// calculé d'un seul tenant sur les 3 km × 3 km.
pub fn banc_frontieres(a: &Args) {
    let (bx, by) = (650i64, 6861i64);
    let villes: Vec<Vec<Ville>> = (0..3)
        .map(|j| (0..3).map(|i| charger(a, bx + i, by + j).expect("tile du bloc")).collect())
        .collect();
    let fin_globale = |x: i32, y: i32| -> Fin {
        if x < 0 || y < 0 || x >= 3 * COTE as i32 || y >= 3 * COTE as i32 {
            return Fin::Bati;
        }
        let (i, j) = (x as usize / COTE, y as usize / COTE);
        champ::fin_de(&villes[j][i], x % COTE as i32, y % COTE as i32)
    };
    let vc = &villes[1][1];
    let centre = sim::centre_accessible(vc, [COTE_F / 2.0, COTE_F / 2.0]);
    let (t1, c1) = sim::champ_1m(vc, &[centre]);
    let zone = sim::zone_de(&t1, &c1);
    let mut al = Alea::new(0xF00);
    let groupe = points(vc, &zone, &mut al, centre, sim::RAYON_GROUPE, 1000);
    const TS: u16 = 1000;
    const AGE: u16 = 1200;
    for res in [1usize, 2, 5] {
        let n = COTE / res;
        // Champ d'un seul tenant.
        let tg = Terrain::depuis_n(3 * n, res, Regle::Majorite, !a.sans_marge, false, fin_globale);
        let gg: Vec<Graine> = graines(
            &tg,
            &groupe.iter().map(|p| [p[0] + COTE_F, p[1] + COTE_F]).collect::<Vec<_>>(),
        )
        .into_iter()
        .map(|g| Graine { ts: TS, ..g })
        .collect();
        let mut cg = Champ::new(&tg);
        let mut dg = Dial::new(&tg);
        let t0 = Instant::now();
        dg.calculer(&tg, &mut cg, &gg);
        let ms_combine = t0.elapsed().as_secs_f64() * 1e3;
        // Tiles avec halo.
        let terrains: Vec<Terrain> = (0..9)
            .map(|k| {
                let (i, j) = ((k % 3) as i32, (k / 3) as i32);
                Terrain::depuis(res, Regle::Majorite, !a.sans_marge, true, |x, y| {
                    fin_globale(i * COTE as i32 + x, j * COTE as i32 + y)
                })
            })
            .collect();
        let locales: Vec<Vec<Graine>> = (0..9)
            .map(|k| {
                if k == 4 {
                    graines(&terrains[4], &groupe)
                        .into_iter()
                        .map(|g| Graine { ts: TS, ..g })
                        .collect()
                } else {
                    vec![]
                }
            })
            .collect();
        let mut champs: Vec<Champ> = terrains.iter().map(Champ::new).collect();
        let mut dials: Vec<Dial> = terrains.iter().map(Dial::new).collect();
        let comparer = |champs: &[Champ]| {
            let (mut ecarts, mut emax) = (0usize, 0u32);
            for (k, c) in champs.iter().enumerate() {
                let (i, j) = ((k % 3) as i32, (k / 3) as i32);
                let t = &terrains[k];
                for cy in 0..n as i32 {
                    for cx in 0..n as i32 {
                        let a = if c.vide { INF } else { c.dist[t.ixy(cx, cy)] };
                        let b = cg.dist[tg.ixy(i * n as i32 + cx, j * n as i32 + cy)];
                        if a != b {
                            ecarts += 1;
                            emax = emax.max((a as i32 - b as i32).unsigned_abs());
                        }
                    }
                }
            }
            (ecarts, emax as f64 * res as f64 / champ::UNITE as f64)
        };
        let finies = |champs: &[Champ]| {
            let mut nb = 0usize;
            let mut dmin = INF;
            for (k, c) in champs.iter().enumerate() {
                if c.vide {
                    continue;
                }
                let t = &terrains[k];
                for cy in 0..n as i32 {
                    for cx in 0..n as i32 {
                        let v = c.dist[t.ixy(cx, cy)];
                        if v != INF {
                            nb += 1;
                            dmin = dmin.min(v);
                        }
                    }
                }
            }
            (nb, if nb > 0 { dmin as f64 * res as f64 / champ::UNITE as f64 } else { 0.0 })
        };
        // Une ronde : chaque tile recalcule avec ses sources et les
        // bordures de la ronde précédente (Jacobi : toutes en parallèle).
        let ronde = |champs: &mut Vec<Champ>, dials: &mut Vec<Dial>, avec_locales: bool, maintenant: u16, age: Option<u16>| {
            let exports: Vec<Echange> = (0..9).map(|k| Echange::exporter(&terrains[k], &champs[k])).collect();
            let t0 = Instant::now();
            for k in 0..9 {
                let (i, j) = ((k % 3) as i32, (k / 3) as i32);
                let mut vois: [[Option<&Echange>; 3]; 3] = [[None; 3]; 3];
                for (dj, ligne) in vois.iter_mut().enumerate() {
                    for (di, v) in ligne.iter_mut().enumerate() {
                        let (ni, nj) = (i + di as i32 - 1, j + dj as i32 - 1);
                        if (0..3).contains(&ni) && (0..3).contains(&nj) && (ni, nj) != (i, j) {
                            *v = Some(&exports[(nj * 3 + ni) as usize]);
                        }
                    }
                }
                let mut g = if avec_locales { locales[k].clone() } else { vec![] };
                champ::graines_halo(&terrains[k], &vois, maintenant, age, &mut g);
                dials[k].calculer(&terrains[k], &mut champs[k], &g);
            }
            t0.elapsed().as_secs_f64() * 1e3
        };
        let ligne = |cas: &str, rondes: usize, ms: f64, ecarts: usize, emax: f64, nb: usize, dmin: f64| {
            println!("{res},{cas},{rondes},{ms:.3},{ms_combine:.3},{ecarts},{emax:.1},{nb},{dmin:.1}");
            eprintln!(
                "frontières {res} m {cas:>22} : {rondes} rondes, {ms:.2} ms par ronde (9 tiles) contre {ms_combine:.2} ms d'un tenant, {ecarts} écarts (max {emax:.1} m), {nb} cases finies (min {dmin:.1} m)"
            );
        };
        // Ronde 0 : sources locales seules.
        let mut ms = vec![ronde(&mut champs, &mut dials, true, TS, Some(AGE))];
        let (mut e, mut emax) = comparer(&champs);
        ligne("ronde_0", 0, ms[0], e, emax, 0, 0.0);
        let mut r = 0;
        while e > 0 && r < 30 {
            r += 1;
            ms.push(ronde(&mut champs, &mut dials, true, TS, Some(AGE)));
            (e, emax) = comparer(&champs);
            ligne(&format!("ronde_{r}"), r, *ms.last().unwrap(), e, emax, 0, 0.0);
        }
        ligne("convergence", r, mediane(ms.clone()), e, emax, 0, 0.0);
        // Expiration : la source n'est plus observée depuis plus d'une
        // minute ; avec horodatage, une ronde suffit à tout effacer.
        let sauvegarde = champs.clone();
        let tms = ronde(&mut champs, &mut dials, false, TS.wrapping_add(AGE + 1), Some(AGE));
        let (nb, dmin) = finies(&champs);
        ligne("expiration_horodatee", 1, tms, 0, 0.0, nb, dmin);
        // Sans horodatage : comptage à l'infini (les valeurs rebondissent
        // d'une tile à l'autre en grandissant lentement).
        champs = sauvegarde;
        let mut r = 0;
        let mut tms;
        loop {
            r += 1;
            tms = ronde(&mut champs, &mut dials, false, 0, None);
            let (nb, _) = finies(&champs);
            if nb == 0 || r >= 200 {
                break;
            }
        }
        let (nb, dmin) = finies(&champs);
        ligne("expiration_naive", r, tms, 0, 0.0, nb, dmin);
    }
}

/// Banc des index de zombies : reconstruction et requêtes, un thread, sur
/// l'instantané de la simulation (toutes les tiles).
pub fn banc_voisinage(a: &Args, sim: &Simulation) {
    let (n, indexe, pos, alertes) = sim.donnees_index();
    let tiles = pos.len() / n;
    let reps = a.repetitions.max(1);
    let variantes = [
        (Structure::Dense32, 2.0),
        (Structure::Dense16, 2.0),
        (Structure::Triee, 2.0),
        (Structure::Creuse, 2.0),
        (Structure::Dense16, 4.0),
        (Structure::Triee, 4.0),
    ];
    let indexes = indexe.iter().filter(|&&b| b).count();
    for (s, case) in variantes {
        let mut vs: Vec<Voisinage> = (0..tiles).map(|_| Voisinage::new(s, COTE_F, case, n)).collect();
        let construire = |vs: &mut Vec<Voisinage>| {
            for (k, v) in vs.iter_mut().enumerate() {
                let b = k * n;
                v.reconstruire_avec(n, |i| indexe[b + i], |i| pos[b + i], |i| alertes[b + i]);
            }
        };
        construire(&mut vs);
        let t0 = Instant::now();
        for _ in 0..reps * 4 {
            construire(&mut vs);
        }
        let construction_us = t0.elapsed().as_secs_f64() * 1e6 / (reps * 4 * tiles) as f64;
        // Requêtes : balayage 3 × 3 limité à 16 voisins (séparation, relais),
        // filtre d'alerte et occupants de 3 cases (capacité).
        let mut empreinte = 0u64;
        let t0 = Instant::now();
        for rep in 0..reps {
            for (k, v) in vs.iter().enumerate() {
                let m = v.cote as i32 - 1;
                for i in 0..n {
                    if !indexe[k * n + i] {
                        continue;
                    }
                    let p = pos[k * n + i];
                    let (cx, cy) = v.coord(p);
                    let (x0, x1) = ((cx - 1).max(0), (cx + 1).min(m));
                    let mut h = 0u64;
                    let mut vus = 0u32;
                    let mut alerte = false;
                    for y in (cy - 1).max(0)..=(cy + 1).min(m) {
                        alerte |= (x0..=x1).any(|x| v.alerte_dans(x, y));
                        let suite = v.pour_plage(y, x0, x1, |e| {
                            if e.id == i as u32 {
                                return true;
                            }
                            h = h.wrapping_mul(31).wrapping_add(e.id as u64);
                            vus += 1;
                            vus < 16
                        });
                        if !suite {
                            break;
                        }
                    }
                    let occ = v.occupants(cx, cy) + v.occupants(x1, cy) + v.occupants(cx, (cy + 1).min(m));
                    if rep == 0 {
                        empreinte = empreinte
                            .wrapping_mul(0x100000001B3)
                            .wrapping_add(h ^ (occ as u64) << 40 ^ (alerte as u64) << 63);
                    }
                    std::hint::black_box((h, occ, alerte));
                }
            }
        }
        let requete_ns = t0.elapsed().as_secs_f64() * 1e9 / (reps * indexes) as f64;
        let octets = vs[0].octets_repertoire();
        println!(
            "{:?},{tiles},{s:?},{case},{indexes},{construction_us:.1},{requete_ns:.1},{octets},{empreinte:016x}",
            a.scenario
        );
        eprintln!(
            "{:?} {s:?} {case} m : reconstruction {construction_us:.0} µs par tile, requête {requete_ns:.0} ns, {octets} octets, empreinte {empreinte:016x}",
            a.scenario
        );
    }
}
