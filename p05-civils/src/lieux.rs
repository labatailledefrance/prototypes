//! Bâtiments réels de la BD TOPO, capacités et implantation des colonies.
//!
//! Règles de capacité (B-02 : selon l'emprise et la hauteur), **hypothèses
//! de prototype** :
//! - niveaux = `nombre_d_etages` s'il existe, sinon hauteur / 3 m (au moins
//!   1) ; plancher = emprise × niveaux ;
//! - **logement** (usage Résidentiel, ou logements > 0) : un colon pour
//!   35 m² de plancher ;
//! - **lieu de travail** (Commercial et services, Industriel, Indifférencié
//!   sans logement, emprise d'au moins 100 m²) : un poste pour 20 m² ;
//! - annexes et constructions légères : ni logement, ni travail ;
//! - **chantiers** : des lieux extérieurs, un par carré de 500 m habité.

use std::collections::HashMap;
use std::path::Path;

use crate::alea::{hacher, unite};
use crate::colon::{Colon, NUL, q};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Usage {
    Logement,
    Travail,
    Autre,
}

#[derive(Clone, Debug)]
pub struct Batiment {
    pub fid: u32,
    pub pos: [f64; 2],
    pub emprise: f32,
    pub niveaux: u16,
    pub usage: Usage,
    /// Colons logés, ou postes de travail.
    pub capacite: u32,
}

pub const M2_LOGEMENT: f32 = 35.0;
pub const M2_POSTE: f32 = 20.0;

pub fn charger(chemin: &Path) -> std::io::Result<Vec<Batiment>> {
    let s = std::fs::read_to_string(chemin)?;
    let mut v = Vec::new();
    for l in s.lines().skip(1) {
        let c: Vec<&str> = l.split(',').collect();
        if c.len() < 10 {
            continue;
        }
        let num = |x: &str| x.parse::<f64>().ok();
        let emprise = num(c[3]).unwrap_or(0.0) as f32;
        let hauteur = num(c[4]);
        let etages = num(c[5]);
        let logements = num(c[6]).unwrap_or(0.0);
        let (u1, leger) = (c[7], c[9] == "1");
        let niveaux = match (etages, hauteur) {
            (Some(e), _) if e >= 1.0 => e as u16,
            (_, Some(h)) => ((h / 3.0).round() as u16).max(1),
            _ => 1,
        };
        let plancher = emprise * niveaux as f32;
        let usage = if leger || u1 == "Annexe" {
            Usage::Autre
        } else if u1 == "Résidentiel" || logements > 0.0 {
            Usage::Logement
        } else if matches!(u1, "Commercial et services" | "Industriel" | "Indifférencié") && emprise >= 100.0 {
            Usage::Travail
        } else {
            Usage::Autre
        };
        let capacite = match usage {
            Usage::Logement => (plancher / M2_LOGEMENT).floor() as u32,
            Usage::Travail => (plancher / M2_POSTE).floor() as u32,
            Usage::Autre => 0,
        };
        v.push(Batiment {
            fid: c[0].parse().unwrap_or(0),
            pos: [num(c[1]).unwrap_or(0.0), num(c[2]).unwrap_or(0.0)],
            emprise,
            niveaux,
            usage,
            capacite,
        });
    }
    Ok(v)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Genre {
    Logement,
    Travail,
    Chantier,
}

/// Un lieu où se trouvent des colons : bâtiment ou chantier extérieur.
#[derive(Clone, Debug)]
pub struct Lieu {
    pub genre: Genre,
    pub pos: [f64; 2],
    /// Indice du bâtiment (`NUL` pour un chantier).
    pub batiment: u32,
    pub capacite: u32,
    pub colonie: u16,
    /// Tile de 1 km (indice dans `Implantation::tiles`).
    pub tile: u32,
    /// Case de 100 m de la carte de risque de la tile (0..100).
    pub case: u8,
    /// Phase de mise à jour (tick modulo la période).
    pub phase: u32,
}

pub struct Implantation {
    pub lieux: Vec<Lieu>,
    pub colons: Vec<(u32, Colon)>,
    /// Tiles de 1 km utilisées (x, y en km Lambert-93).
    pub tiles: Vec<(i64, i64)>,
    pub colonies: usize,
    pub centres: Vec<[f64; 2]>,
}

pub struct Plan {
    pub colons: usize,
    /// Colons par colonie (au plus).
    pub taille_colonie: usize,
    /// Centre de la première colonie (m) ; les suivantes sont posées en
    /// spirale autour, à `ecart` m.
    pub centre: [f64; 2],
    pub ecart: f64,
    /// Rectangle autorisé (m) : xmin, ymin, xmax, ymax.
    pub cadre: [f64; 4],
    pub part_actifs: f32,
    pub part_chantiers: f32,
    pub periode: u32,
    pub graine: u64,
}

fn dedans(c: &[f64; 4], p: [f64; 2]) -> bool {
    p[0] >= c[0] && p[1] >= c[1] && p[0] < c[2] && p[1] < c[3]
}

fn d2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// Centres des colonies : spirale carrée autour du premier.
fn centres(plan: &Plan, k: usize) -> Vec<[f64; 2]> {
    let mut v = vec![plan.centre];
    let (mut x, mut y, mut dx, mut dy) = (0i64, 0i64, 1i64, 0i64);
    let (mut pas, mut fait, mut tours) = (1, 0, 0);
    while v.len() < k {
        x += dx;
        y += dy;
        fait += 1;
        if fait == pas {
            fait = 0;
            (dx, dy) = (-dy, dx);
            tours += 1;
            if tours % 2 == 0 {
                pas += 1;
            }
        }
        let c = [plan.centre[0] + x as f64 * plan.ecart, plan.centre[1] + y as f64 * plan.ecart];
        if dedans(&plan.cadre, c) {
            v.push(c);
        }
    }
    v
}

/// Installe les colons : chaque colonie prend les logements les plus proches
/// de son centre, pleins, jusqu'à son effectif ; puis les lieux de travail
/// voisins et les chantiers ; chaque actif reçoit un travail à moins de
/// 1,5 km si possible.
pub fn implanter(bats: &[Batiment], plan: &Plan) -> Implantation {
    let k = plan.colons.div_ceil(plan.taille_colonie.max(1)).max(1);
    let cs = centres(plan, k);
    let mut pris = vec![false; bats.len()];
    let logements: Vec<u32> = (0..bats.len() as u32)
        .filter(|&i| {
            let b = &bats[i as usize];
            b.usage == Usage::Logement && b.capacite > 0 && dedans(&plan.cadre, b.pos)
        })
        .collect();
    let mut lieux: Vec<Lieu> = vec![];
    let mut habitants: Vec<u32> = vec![];
    let mut rayon_col = vec![0.0f64; k];
    let mut reste = plan.colons;
    for (ci, c) in cs.iter().enumerate() {
        let quota = (plan.colons / k + usize::from(ci < plan.colons % k)).min(reste);
        let mut cand: Vec<(u64, u32)> = logements
            .iter()
            .filter(|&&i| !pris[i as usize])
            .map(|&i| ((d2(bats[i as usize].pos, *c) * 16.0) as u64, i))
            .collect();
        cand.sort_unstable();
        let mut n = 0usize;
        for (dd, i) in cand {
            if n >= quota {
                break;
            }
            let b = &bats[i as usize];
            let m = (b.capacite as usize).min(quota - n);
            pris[i as usize] = true;
            lieux.push(Lieu {
                genre: Genre::Logement,
                pos: b.pos,
                batiment: i,
                capacite: b.capacite,
                colonie: ci as u16,
                tile: 0,
                case: 0,
                phase: 0,
            });
            habitants.push(m as u32);
            n += m;
            rayon_col[ci] = rayon_col[ci].max((dd as f64 / 16.0).sqrt());
        }
        reste -= n;
    }
    // Lieux de travail : bâtiments de travail dans le rayon de la colonie la
    // plus proche.
    let n_logements = lieux.len();
    for (i, b) in bats.iter().enumerate() {
        if b.usage != Usage::Travail || b.capacite == 0 || !dedans(&plan.cadre, b.pos) {
            continue;
        }
        let (ci, dd) = cs
            .iter()
            .enumerate()
            .map(|(ci, c)| (ci, d2(b.pos, *c)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        if dd.sqrt() <= rayon_col[ci] + 200.0 {
            lieux.push(Lieu {
                genre: Genre::Travail,
                pos: b.pos,
                batiment: i as u32,
                capacite: b.capacite,
                colonie: ci as u16,
                tile: 0,
                case: 0,
                phase: 0,
            });
        }
    }
    // Chantiers : un par carré de 500 m habité, au centre des logements.
    let mut carres: HashMap<(i64, i64), (f64, f64, f64, u16)> = HashMap::new();
    for l in &lieux[..n_logements] {
        let cle = ((l.pos[0] / 500.0).floor() as i64, (l.pos[1] / 500.0).floor() as i64);
        let e = carres.entry(cle).or_insert((0.0, 0.0, 0.0, l.colonie));
        e.0 += l.pos[0];
        e.1 += l.pos[1];
        e.2 += 1.0;
    }
    let mut cles: Vec<_> = carres.into_iter().collect();
    cles.sort_by_key(|(k, _)| *k);
    for (_, (sx, sy, n, col)) in cles {
        lieux.push(Lieu {
            genre: Genre::Chantier,
            pos: [sx / n, sy / n],
            batiment: NUL,
            capacite: u32::MAX,
            colonie: col,
            tile: 0,
            case: 0,
            phase: 0,
        });
    }
    // Tiles, cases, phases.
    let mut tiles: Vec<(i64, i64)> = vec![];
    let mut tile_de: HashMap<(i64, i64), u32> = HashMap::new();
    for (li, l) in lieux.iter_mut().enumerate() {
        let (tx, ty) = ((l.pos[0] / 1000.0).floor() as i64, (l.pos[1] / 1000.0).floor() as i64);
        let t = *tile_de.entry((tx, ty)).or_insert_with(|| {
            tiles.push((tx, ty));
            tiles.len() as u32 - 1
        });
        l.tile = t;
        let (cx, cy) = (
            ((l.pos[0] - tx as f64 * 1000.0) / 100.0).floor().clamp(0.0, 9.0) as u8,
            ((l.pos[1] - ty as f64 * 1000.0) / 100.0).floor().clamp(0.0, 9.0) as u8,
        );
        l.case = cy * 10 + cx;
        l.phase = (hacher(plan.graine ^ 0x9A5E, li as u64) % plan.periode as u64) as u32;
    }
    // Grille des lieux de travail (cases de 500 m) pour l'affectation.
    let mut grille: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    let mut chantier_de: HashMap<(i64, i64), u32> = HashMap::new();
    for (li, l) in lieux.iter().enumerate() {
        let cle = ((l.pos[0] / 500.0).floor() as i64, (l.pos[1] / 500.0).floor() as i64);
        match l.genre {
            Genre::Travail => grille.entry(cle).or_default().push(li as u32),
            Genre::Chantier => {
                chantier_de.insert(cle, li as u32);
            }
            Genre::Logement => {}
        }
    }
    let mut occupes = vec![0u32; lieux.len()];
    // Colons.
    let mut colons = Vec::with_capacity(plan.colons);
    let mut id = 0u32;
    for (li, &n) in habitants.iter().enumerate() {
        let l = &lieux[li];
        let cle = ((l.pos[0] / 500.0).floor() as i64, (l.pos[1] / 500.0).floor() as i64);
        for _ in 0..n {
            let h = hacher(plan.graine, id as u64);
            let mut travail = NUL;
            if unite(hacher(h, 1)) < plan.part_actifs {
                if unite(hacher(h, 2)) < plan.part_chantiers {
                    travail = chantier_de[&cle];
                } else {
                    // Un poste libre dans les 3 × 3 carrés autour, sinon
                    // plus loin, sinon un chantier.
                    'r: for r in 1..=4i64 {
                        let mut cands: Vec<u32> = vec![];
                        for dy in -r..=r {
                            for dx in -r..=r {
                                if let Some(v) = grille.get(&(cle.0 + dx, cle.1 + dy)) {
                                    cands.extend(v.iter().filter(|&&w| occupes[w as usize] < lieux[w as usize].capacite));
                                }
                            }
                        }
                        if !cands.is_empty() {
                            let w = cands[(hacher(h, 3 + r as u64) % cands.len() as u64) as usize];
                            occupes[w as usize] += 1;
                            travail = w;
                            break 'r;
                        }
                    }
                    if travail == NUL {
                        travail = chantier_de[&cle];
                    }
                }
            }
            let mut comp = 0u16;
            for (b, p) in [0.08f32, 0.05, 0.02, 0.04, 0.06, 0.03].iter().enumerate() {
                if unite(hacher(h, 20 + b as u64)) < *p {
                    comp |= 1 << b;
                }
            }
            let c = Colon {
                id,
                maj: 0,
                logement: li as u32,
                travail,
                j: [
                    q(unite(hacher(h, 30)) * 0.3, hacher(h, 31)),
                    q(unite(hacher(h, 32)) * 0.2, hacher(h, 33)),
                    q(0.3 + unite(hacher(h, 34)) * 0.3, hacher(h, 35)),
                    q(0.6 + unite(hacher(h, 36)) * 0.2, hacher(h, 37)),
                    q(unite(hacher(h, 38)) * 0.2, hacher(h, 39)),
                ],
                // État de milieu de partie : expérience déjà acquise.
                experience: (unite(hacher(h, 40)) * 2_000.0) as u16,
                competences: comp,
                malade: 0,
                drapeaux: 0,
            };
            colons.push((li as u32, c));
            id += 1;
        }
    }
    let mut imp = Implantation { lieux, colons, tiles, colonies: k, centres: cs };
    trier_par_phase(&mut imp);
    imp
}

/// Renumérote les lieux par phase : les lieux mis à jour au même tick sont
/// contigus, ce qui permet de les découper en lots sans `unsafe`.
pub fn trier_par_phase(imp: &mut Implantation) {
    let mut ordre: Vec<u32> = (0..imp.lieux.len() as u32).collect();
    ordre.sort_by_key(|&i| (imp.lieux[i as usize].phase, i));
    let mut nouveau = vec![0u32; ordre.len()];
    for (n, &a) in ordre.iter().enumerate() {
        nouveau[a as usize] = n as u32;
    }
    imp.lieux = ordre.iter().map(|&a| imp.lieux[a as usize].clone()).collect();
    for (l, c) in &mut imp.colons {
        *l = nouveau[*l as usize];
        c.logement = nouveau[c.logement as usize];
        if c.travail != NUL {
            c.travail = nouveau[c.travail as usize];
        }
    }
    imp.colons.sort_by_key(|(l, c)| (*l, c.id));
}
