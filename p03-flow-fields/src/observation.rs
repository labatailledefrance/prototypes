//! Mode observation : crédibilité du mouvement sur la première tile.
//!
//! Toutes les 10 s de jeu, on mesure contre un **oracle** (champ exact à
//! 1 m vers les humains du moment, ou vers le bruit) :
//! - la **progression** des zombies qui ont une cible : de combien leur
//!   distance à pied à la cible a baissé en 10 s ;
//! - les **bloqués** : moins de 2 m de progression en 10 s, encore à plus de
//!   20 m de la cible (le zombie piétine derrière un immeuble) ;
//! - l'**écart** dû à la période de recalcul : part des zombies dont la
//!   direction donnée par le champ courant s'écarte de plus de 45° de celle
//!   d'un champ recalculé à l'instant ;
//! - les zombies **dans les murs** et les densités.
//!
//! Et toutes les `fenetre` secondes, une image : bâti en gris, barricades en
//! rouge, densité de passage des zombies sur la fenêtre (du jaune au violet,
//! échelle logarithmique), humains en vert.

use std::io::Write;
use std::path::Path;

use crate::Args;
use crate::champ::{Champ, Terrain};
use crate::sim::{self, ATTIRE, PERDU, POURSUITE, RELAIS, Simulation};
use crate::ville::COTE;

pub const ENTETE: &str = "etiquette,scenario,champ,marge,resolution,regle,periode_champ,recalcul,reservation,t_s,errance,poursuite,relayes,attire,perdu,profond,proches,suivis,bloques_pct,bloques_hors_foule_pct,progression_ms,ecart_pct,murs,densite_max,densite_contact";

/// Oracles exacts à 1 m, sans marge : vers les humains du moment (zombies
/// proches), vers les sources que la règle fait suivre (humains observés
/// depuis moins d'une minute, à leur position observée), vers le bruit.
struct Oracle {
    humains: (Terrain, Champ),
    sources: Option<(Terrain, Champ)>,
    bruits: Option<(Terrain, Champ)>,
}

impl Oracle {
    fn new(sim: &Simulation) -> Oracle {
        let t = &sim.tiles().tiles[0];
        let src = sim.positions_sources(0);
        Oracle {
            humains: sim::champ_1m(&t.ville, &t.humains.pos),
            sources: (!src.is_empty()).then(|| sim::champ_1m(&t.ville, &src)),
            bruits: (!t.bruits.is_empty()).then(|| sim::champ_1m(&t.ville, &t.bruits)),
        }
    }
    /// Distance à pied à ce que l'état fait suivre.
    fn distance(&self, etat: u8, p: [f32; 2]) -> Option<f32> {
        let (t, c) = if etat == ATTIRE {
            self.bruits.as_ref()?
        } else {
            self.sources.as_ref()?
        };
        c.metres(t, p)
    }
    fn distance_humains(&self, p: [f32; 2]) -> Option<f32> {
        self.humains.1.metres(&self.humains.0, p)
    }
}

fn ecrire_image(sim: &Simulation, densite: &[u32], fenetre_ticks: u64, chemin: &Path) {
    let t = &sim.tiles().tiles[0];
    // Échelle fixe : saturée à un zombie présent sur le mètre carré pendant
    // toute la fenêtre.
    let max = (fenetre_ticks.max(1)) as f32;
    let mut img = vec![0u8; COTE * COTE * 3];
    for y in 0..COTE {
        for x in 0..COTE {
            // Nord en haut : la ligne 0 de l'image est y = 999.
            let o = ((COTE - 1 - y) * COTE + x) * 3;
            let (xi, yi) = (x as i32, y as i32);
            let mut c = if t.ville.barricade(xi, yi) {
                [220, 30, 30]
            } else if t.ville.bati(xi, yi) {
                [196, 196, 196]
            } else {
                [255, 255, 255]
            };
            let n = densite[y * COTE + x];
            if n > 0 && !t.ville.bati(xi, yi) {
                let v = ((1.0 + n as f32).ln() / (1.0 + max).ln()).min(1.0);
                let a = [255.0, 214.0, 102.0];
                let b = [90.0, 0.0, 110.0];
                for k in 0..3 {
                    c[k] = (a[k] + (b[k] - a[k]) * v) as u8;
                }
            }
            img[o..o + 3].copy_from_slice(&c);
        }
    }
    let mut point = |p: [f32; 2], c: [u8; 3], r: i32| {
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (p[0] as i32 + dx, p[1] as i32 + dy);
                if x >= 0 && y >= 0 && (x as usize) < COTE && (y as usize) < COTE {
                    let o = ((COTE - 1 - y as usize) * COTE + x as usize) * 3;
                    img[o..o + 3].copy_from_slice(&c);
                }
            }
        }
    };
    for &h in &t.humains.pos {
        point(h, [0, 150, 60], 1);
    }
    if let Some(r) = &t.repli {
        point(r.dest, [0, 90, 220], 4);
    }
    let mut f = std::fs::File::create(chemin).expect("image");
    write!(f, "P6\n{COTE} {COTE}\n255\n").unwrap();
    f.write_all(&img).unwrap();
}

pub fn observer(a: &Args, sim: &mut Simulation, dossier: &Path) {
    std::fs::create_dir_all(dossier).expect("dossier d'observation");
    let n = a.zombies;
    let mut densite = vec![0u32; COTE * COTE];
    // Suivi : (distance à la cible, état) au point de mesure précédent.
    let mut suivi: Vec<Option<(f32, u8)>> = vec![None; n];
    let ticks = a.duree as u64 * 20;
    for t in 0..=ticks {
        if a.barricade_a.is_some_and(|b| b as u64 * 20 == t) {
            sim.poser_barricades();
        }
        let s = t / 20;
        if t > 0 && t % (a.fenetre as u64 * 20) == 0 {
            let nom = format!("{}-{:03}s.ppm", a.etiquette, s);
            ecrire_image(sim, &densite, a.fenetre as u64 * 20, &dossier.join(nom));
            densite.fill(0);
        }
        if t % 200 == 0 {
            let vues: Vec<sim::Vue> = sim.vues().into_iter().filter(|z| z.tile == 0).collect();
            let nouvel = Oracle::new(sim);
            let frais = sim.champ_frais(0);
            let tile = &sim.tiles().tiles[0];
            let (mut suivis, mut bloques, mut progres) = (0usize, 0usize, 0f64);
            let mut hors_foule = 0usize;
            let occ = &sim.occupation(0).vois;
            let (mut guides, mut ecarts) = (0usize, 0usize);
            let mut proches = 0usize;
            let mut nouveau = vec![None; n];
            for z in &vues {
                let r = z.rang as usize;
                let actif = matches!(z.etat, POURSUITE | RELAIS | ATTIRE | PERDU);
                if let Some((d0, e0)) = suivi[r]
                    && actif
                    && let Some(d1) = nouvel.distance(e0, z.pos)
                {
                    suivis += 1;
                    progres += (d0 - d1) as f64;
                    if d0 - d1 < 2.0 && d1 > 20.0 {
                        bloques += 1;
                        // Hors foule : au plus 2 zombies sur sa case de
                        // l'index (bloqué par le chemin, pas par un bouchon).
                        let (cx, cy) = occ.coord(z.pos);
                        if occ.occupants(cx, cy) <= 2 {
                            hors_foule += 1;
                        }
                    }
                }
                if nouvel.distance_humains(z.pos).is_some_and(|d| d <= 20.0) {
                    proches += 1;
                }
                if matches!(z.etat, POURSUITE | RELAIS | ATTIRE)
                    && let Some(d) = nouvel.distance(z.etat, z.pos)
                    && d > 20.0
                {
                    nouveau[r] = Some((d, z.etat));
                }
                if matches!(z.etat, POURSUITE | RELAIS | PERDU) && sim::d2(z.pos, z.cible) > 15.0 * 15.0 {
                    if let (Some(u), Some(v)) =
                        (tile.proies.direction(&tile.terrain, z.pos), frais.direction(&tile.terrain, z.pos))
                    {
                        guides += 1;
                        if u[0] * v[0] + u[1] * v[1] < std::f32::consts::FRAC_1_SQRT_2 {
                            ecarts += 1;
                        }
                    }
                }
            }
            suivi = nouveau;
            let b = sim.bilan();
            let pct = |x: usize, y: usize| if y > 0 { 100.0 * x as f64 / y as f64 } else { 0.0 };
            let ligne = format!(
                "{},{:?},{:?},{},{},{:?},{},{:?},{:?},{},{},{},{},{},{},{},{},{},{:.2},{:.2},{:.3},{:.2},{},{},{}",
                a.etiquette,
                a.scenario,
                a.champ,
                !a.sans_marge,
                a.resolution,
                a.regle,
                a.periode_champ,
                a.recalcul,
                a.reservation,
                s,
                b.errance,
                b.poursuite,
                b.relais,
                b.attire,
                b.perdu,
                b.profond,
                proches,
                suivis,
                pct(bloques, suivis),
                pct(hors_foule, suivis),
                if suivis > 0 { progres / suivis as f64 / 10.0 } else { 0.0 },
                pct(ecarts, guides),
                b.murs,
                b.densite_max,
                b.densite_contact
            );
            println!("{ligne}");
            eprintln!("{ligne}");
        }
        if t == ticks {
            break;
        }
        sim.tick();
        for z in sim.vues() {
            if z.tile == 0 && (z.pos[0] as usize) < COTE && (z.pos[1] as usize) < COTE {
                densite[z.pos[1] as usize * COTE + z.pos[0] as usize] += 1;
            }
        }
    }
}
