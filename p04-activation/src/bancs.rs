//! Précalcul (chaîne de données) et banc de l'activation, sur un thread.

use std::time::Instant;

use bevy_ecs::prelude::*;

use crate::Args;
use crate::alea::{Alea, hacher};
use crate::carte::{self, Carte};
use crate::champ::{ChantierTerrain, Regle, Terrain};
use crate::sauvegarde::{self, COTE_CORPS, Format, TuileSauvee, ZSauve};
use crate::sim::{self, Cerveau, Corps, Ident};
use crate::ville::{COTE, Ville};

pub const ENTETE_PRECALCUL: &str =
    "x_km,y_km,couverture,libres,zone_globale,zone_locale,locale_seule,globale_seule,local_ms,octets_zone,octets_exclusions,global_ms";

/// Chaîne de données : zones accessibles globales du bloc, écrites dans
/// `dossier_zones` ; comparaison avec la zone locale (P-03) de chaque tile.
pub fn precalculer(a: &Args) {
    let geo = crate::geo(a);
    let villes: Vec<Ville> = (0..geo.n())
        .map(|t| {
            let (x, y) = geo.km(t);
            Ville::charger(&a.dossier_tiles.join(format!("x{x}_y{y}.bin"))).expect("tile")
        })
        .collect();
    let (zones, stat) = carte::zones_globales(&villes, geo);
    eprintln!(
        "zones globales : {:.0} ms pour {} tiles ({} composantes, principale {} cases sur {} libres)",
        stat.ms,
        geo.n(),
        stat.composantes,
        stat.cases_principale,
        stat.cases_libres
    );
    std::fs::create_dir_all(&a.dossier_zones).expect("dossier des zones");
    for t in 0..geo.n() {
        let (x, y) = geo.km(t);
        let octets = carte::ecrire_zone(&a.dossier_zones.join(format!("x{x}_y{y}.zone")), &zones[t]).expect("zone");
        let t0 = Instant::now();
        let zl = carte::zone_locale(&villes[t]);
        let local_ms = t0.elapsed().as_secs_f64() * 1e3;
        let compte = |z: &[u64]| z.iter().map(|w| w.count_ones() as u64).sum::<u64>();
        let libres = (COTE * COTE) as u64 - (villes[t].couverture as f64 * (COTE * COTE) as f64).round() as u64;
        // Variante : ne stocker que les cases libres exclues (cours fermées).
        let exclus: Vec<u64> = (0..carte::MOTS)
            .map(|k| {
                let mut libre = 0u64;
                for b in 0..64 {
                    let i = k * 64 + b;
                    if i < COTE * COTE && !villes[t].bati((i % COTE) as i32, (i / COTE) as i32) {
                        libre |= 1 << b;
                    }
                }
                libre & !zones[t][k]
            })
            .collect();
        let brut: Vec<u8> = exclus.iter().flat_map(|w| w.to_le_bytes()).collect();
        let octets_exclus = lz4_flex::compress_prepend_size(&brut).len();
        let ls: u64 = zl.iter().zip(&zones[t]).map(|(l, g)| (l & !g).count_ones() as u64).sum();
        let gs: u64 = zl.iter().zip(&zones[t]).map(|(l, g)| (g & !l).count_ones() as u64).sum();
        println!(
            "{x},{y},{:.4},{libres},{},{},{ls},{gs},{local_ms:.2},{octets},{octets_exclus},{:.0}",
            villes[t].couverture,
            compte(&zones[t]),
            compte(&zl),
            stat.ms
        );
    }
}

pub const ENTETE_BANC: &str = "x_km,y_km,couverture,zombies,repetition,terrain_ms,terrain_ticks,terrain_lz4_octets,terrain_lz4_ms,placement_ms,naissance_ms,destruction_ms,instantane_ms,brut_octets,brut_enc_ms,brut_dec_ms,brutzstd_octets,brutzstd_enc_ms,brutzstd_dec_ms,compact_octets,compact_enc_ms,compact_dec_ms,lz4_octets,lz4_enc_ms,lz4_dec_ms,zstd_octets,zstd_enc_ms,zstd_dec_ms,ecriture_ms,ecriture_fsync_ms,lecture_ms,identique";

/// Banc, tile par tile (les plus bâties d'abord) : coût de chaque étape de
/// l'activation et de la désactivation, tailles de sauvegarde.
pub fn banc(a: &Args) {
    let geo = crate::geo(a);
    let (carte, _) = Carte::reelle(&a.dossier_tiles, geo, Some(&a.dossier_zones), a.densite).expect("bloc");
    let mut ordre: Vec<usize> = (0..geo.n()).collect();
    ordre.sort_by(|&x, &y| carte.tuiles[y].couverture.total_cmp(&carte.tuiles[x].couverture));
    let dossier = a.dossier_sauvegardes.join("banc");
    std::fs::create_dir_all(&dossier).expect("dossier");
    let p = sim::Params::default();
    for &t in ordre.iter().take(a.banc_tiles) {
        let tu = &carte.tuiles[t];
        let n = tu.population as usize;
        let (x, y) = geo.km(t);
        for rep in 0..a.repetitions {
            // Terrain de 2 m avec halo, d'un coup puis au budget par tick.
            let mut terrain = Terrain::vide(2, Regle::Majorite, true);
            let t0 = Instant::now();
            let mut ch = ChantierTerrain::default();
            terrain.avancer_chantier(&mut ch, u64::MAX, &|x, y| carte.fin(t, x, y));
            let terrain_ms = t0.elapsed().as_secs_f64() * 1e3;
            let mut t2 = Terrain::vide(2, Regle::Majorite, true);
            let mut ch2 = ChantierTerrain::default();
            let mut ticks = 1;
            while !t2.avancer_chantier(&mut ch2, p.budget_terrain, &|x, y| carte.fin(t, x, y)) {
                ticks += 1;
            }
            assert!(t2.cout == terrain.cout && t2.masque == terrain.masque, "terrain étalé différent");
            // Terrain précalculé ? Taille compressée et décompression.
            let brut: Vec<u8> = terrain.cout.iter().chain(&terrain.masque).copied().collect();
            let c = lz4_flex::compress_prepend_size(&brut);
            let t0 = Instant::now();
            let d = lz4_flex::decompress_size_prepended(&c).unwrap();
            let terrain_lz4_ms = t0.elapsed().as_secs_f64() * 1e3;
            assert_eq!(d, brut);
            // Placement.
            let t0 = Instant::now();
            let liste: Vec<([f32; 2], f32)> = (0..n).map(|i| sim::placer(&tu.ville, &tu.zone, t, i)).collect();
            let placement_ms = t0.elapsed().as_secs_f64() * 1e3;
            // Naissance et destruction des entités (monde neuf).
            let mut w = World::new();
            let t0 = Instant::now();
            let ents: Vec<Entity> = w
                .spawn_batch(liste.iter().enumerate().map(|(r, &(p, cap))| {
                    (
                        Corps { pos: [p[0] as f64, p[1] as f64], cap },
                        Cerveau { etat: sim::PROFOND, cible: [0.0; 2], alerte: 0, perdu_depuis: 0 },
                        Ident { slot: 0, rang: r as u32 },
                    )
                }))
                .collect();
            let naissance_ms = t0.elapsed().as_secs_f64() * 1e3;
            // Instantané : lecture de toutes les entités, normalisation.
            let t0 = Instant::now();
            let mut q = w.query::<(&Corps, &Ident)>();
            let mut l: Vec<(u32, [f64; 2], f32)> = q.iter(&w).map(|(c, id)| (id.rang, c.pos, c.cap)).collect();
            l.sort_unstable_by_key(|z| z.0);
            let mut zombies: Vec<ZSauve> = l.iter().map(|z| ZSauve::de([z.1[0] as f32, z.1[1] as f32], z.2)).collect();
            zombies.sort_unstable();
            let instantane_ms = t0.elapsed().as_secs_f64() * 1e3;
            let t0 = Instant::now();
            for e in ents {
                w.despawn(e);
            }
            let destruction_ms = t0.elapsed().as_secs_f64() * 1e3;
            // Contenu réaliste : corps autour d'une base défendue, anneau de
            // barricades entamé.
            let mut al = Alea::new(hacher(t as u64, 77));
            let mut corps = vec![0u16; COTE_CORPS * COTE_CORPS];
            for _ in 0..3000 {
                let (r, th) = (al.f32().sqrt() * 30.0, al.f32() * std::f32::consts::TAU);
                let (cx, cy) = ((50.0 + r * th.cos()) as usize, (50.0 + r * th.sin()) as usize);
                corps[cy * COTE_CORPS + cx] += 1;
            }
            let centre = sim::centre_accessible(&tu.ville, [500.0, 500.0]);
            let anneau = sim::anneau_barricades(&tu.ville, &tu.zone, centre, 80.0);
            let constructions = anneau.iter().map(|&(x, y)| (x, y, al.entre(0, 6000) as i32)).collect();
            let ts = TuileSauvee { x_km: x, y_km: y, tick: 1000, zombies, corps, constructions };
            let mut ligne = format!(
                "{x},{y},{:.4},{n},{rep},{terrain_ms:.3},{ticks},{},{terrain_lz4_ms:.3},{placement_ms:.3},{naissance_ms:.3},{destruction_ms:.3},{instantane_ms:.3}",
                tu.couverture,
                c.len()
            );
            let mut identique = true;
            for f in [Format::Brut, Format::BrutZstd, Format::Compact, Format::Lz4, Format::Zstd] {
                let t0 = Instant::now();
                let b = sauvegarde::encoder(&ts, f);
                let enc = t0.elapsed().as_secs_f64() * 1e3;
                let t0 = Instant::now();
                let r = sauvegarde::decoder(&b).expect("décodage");
                let dec = t0.elapsed().as_secs_f64() * 1e3;
                identique &= r == ts;
                ligne += &format!(",{},{enc:.3},{dec:.3}", b.len());
            }
            let b = sauvegarde::encoder(&ts, Format::Zstd);
            let chemin = dossier.join(format!("x{x}_y{y}.tile"));
            let ecrire = |fsync: bool| {
                use std::io::Write;
                let t0 = Instant::now();
                let mut f = std::fs::File::create(&chemin).unwrap();
                f.write_all(&b).unwrap();
                if fsync {
                    f.sync_all().unwrap();
                }
                t0.elapsed().as_secs_f64() * 1e3
            };
            let ecr = ecrire(false);
            let ecr_sync = ecrire(true);
            let t0 = Instant::now();
            let lu = std::fs::read(&chemin).unwrap();
            let lecture_ms = t0.elapsed().as_secs_f64() * 1e3;
            identique &= sauvegarde::decoder(&lu).unwrap() == ts && sauvegarde::somme(&sauvegarde::decoder(&lu).unwrap()) == sauvegarde::somme(&ts);
            ligne += &format!(",{ecr:.3},{ecr_sync:.3},{lecture_ms:.3},{identique}");
            println!("{ligne}");
            eprintln!(
                "x{x} y{y} ({:.0} %, {n} zombies) : terrain {terrain_ms:.2} ms ({ticks} ticks), placement {placement_ms:.2}, naissance {naissance_ms:.2}, destruction {destruction_ms:.2}, sauvegarde zstd {} o",
                tu.couverture * 100.0,
                b.len()
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dossier);
}

