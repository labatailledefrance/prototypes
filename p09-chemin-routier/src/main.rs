//! P-09 — Recherche de chemin routière.
//!
//! Modes : `campagne` (scénarios 1 à 6 sur un jeu de tiles), `determinisme`
//! (empreintes, scénario 7), `images` (vues de dessus).

mod algos;
mod classes;
mod graphe;
mod hier;
mod images;
mod mesure;
mod tuiles;
mod valeurs_ponts;
#[cfg(test)]
mod tests;

use algos::{Borne, Csr, Etat, Reperes, Trajet};
use classes::{Classe, INF, Refus, Sens};
use clap::Parser;
use graphe::{Fragment, Graphe};
use mesure::{Alea, moyenne, ms, pct};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use tuiles::{Pont, TileId};

#[derive(Parser, Clone)]
pub struct Args {
    #[arg(long)]
    pub mode: String,
    /// Dossier des tiles de P-07 (contient `paquets/`).
    #[arg(long)]
    pub tiles: PathBuf,
    #[arg(long)]
    pub sortie: PathBuf,
    #[arg(long, default_value = "manche")]
    pub etiquette: String,
    #[arg(long, default_value_t = 0)]
    pub threads: usize,
    /// Ordre de lecture des tiles : direct, inverse, melange.
    #[arg(long, default_value = "direct")]
    pub ordre: String,
    #[arg(long, default_value_t = 10000)]
    pub paires: usize,
    /// Paires chronométrées une à une sur un thread (toutes méthodes).
    #[arg(long, default_value_t = 1000)]
    pub banc: usize,
    /// Longueur des routes de P-07 (tiles du département), km.
    #[arg(long, default_value_t = 0.0)]
    pub controle_km: f64,
    /// Lieux : nom=x,y;… (Lambert-93, m). Le premier sert de racine.
    #[arg(long, default_value = "")]
    pub lieux: String,
    /// Trajets typiques : a>b,c>d…
    #[arg(long, default_value = "")]
    pub trajets: String,
    #[arg(long, default_value_t = 1000)]
    pub convois: usize,
    #[arg(long, default_value_t = 16)]
    pub reperes: usize,
}

pub struct Journal {
    pub lignes: String,
}

impl Journal {
    pub fn kv(&mut self, k: &str, v: impl std::fmt::Display) {
        println!("{k} {v}");
        writeln!(self.lignes, "{k},{v}").unwrap();
    }
}

pub struct Monde {
    pub fs: Vec<Fragment>,
    pub g: Graphe,
    pub ponts: Vec<Pont>,
    pub csr: Csr,
    pub cls: Vec<Classe>,
    /// Métriques : (nom, poids par arc).
    pub w: Vec<(String, Vec<u32>)>,
    pub lieux: Vec<(String, u32)>,
    /// Lieux accrochés par métrique : lieux_m[k][i].
    pub lieux_m: Vec<Vec<u32>>,
    pub racine: u32,
}

pub const PRINCIPALES: usize = 4; // voiture, camion, porte-char, leclerc

fn ordonner<T>(v: &mut Vec<T>, ordre: &str) {
    match ordre {
        "inverse" => v.reverse(),
        "melange" => {
            let mut a = Alea(77);
            for i in (1..v.len()).rev() {
                let j = a.sous(i + 1);
                v.swap(i, j);
            }
        }
        _ => {}
    }
}

pub fn charger(a: &Args, j: &mut Journal) -> Monde {
    let (mut tiles, t_lire) = ms(|| tuiles::lire_paquets(&a.tiles.join("paquets")));
    ordonner(&mut tiles, &a.ordre);
    j.kv("tiles", tiles.len());
    j.kv("lecture_paquets_ms", format!("{t_lire:.0}"));
    let oct_r: usize = tiles.iter().map(|t| t.octets_reseau_z).sum();
    let oct_p: usize = tiles.iter().map(|t| t.octets_ponts_z).sum();
    j.kv("section_reseau_octets_zstd", oct_r);
    j.kv("section_ponts_octets_zstd", oct_p);
    let ((par_tile, ponts, nv), t_dec) = ms(|| {
        let r: Vec<(Vec<tuiles::Morceau>, usize, Vec<Pont>)> = tiles
            .par_iter()
            .map(|t| {
                let (m, nv) = t.reseau.as_ref().map(|b| tuiles::reseau(t.id, b)).unwrap_or_default();
                let p = t.ponts.as_ref().map(|b| tuiles::ponts(t.id, b)).unwrap_or_default();
                (m, nv, p)
            })
            .collect();
        let nv: usize = r.iter().map(|x| x.1).sum();
        let mut ponts: Vec<Pont> = r.iter().flat_map(|x| x.2.iter().cloned()).collect();
        ponts.sort_by_key(|p| p.id);
        (r.into_iter().map(|x| x.0).collect::<Vec<_>>(), ponts, nv)
    });
    j.kv("decodage_ms", format!("{t_dec:.0}"));
    let nb_morceaux: usize = par_tile.iter().map(|v| v.len()).sum();
    j.kv("morceaux_routes", nb_morceaux);
    j.kv("morceaux_voies_ferrees", nv);
    j.kv("ponts_lus", ponts.len());
    let tile_ids: Vec<TileId> = tiles.iter().map(|t| t.id).collect();
    drop(tiles);

    // Recousure : par tile, puis entre tiles.
    let ((fs, st_g, st_t), t_rec) = ms(|| {
        let loc: Vec<(Vec<Fragment>, graphe::StatsRecousure)> = par_tile.par_iter().map(|ms| graphe::recoudre(graphe::de_morceaux(ms.clone()))).collect();
        let mut st_t = graphe::StatsRecousure::default();
        for (_, s) in &loc {
            st_t.jointures += s.jointures;
            st_t.doublons += s.doublons;
        }
        let tout: Vec<Fragment> = loc.into_iter().flat_map(|x| x.0).collect();
        let (fs, st_g) = graphe::recoudre(tout);
        (fs, st_g, st_t)
    });
    j.kv("recousure_ms", format!("{t_rec:.0}"));
    j.kv("recousure_jointures_dans_tiles", st_t.jointures);
    j.kv("recousure_jointures_entre_tiles", st_g.jointures);
    j.kv("recousure_doublons", st_t.doublons + st_g.doublons);
    j.kv("recousure_ruptures", st_g.ruptures);
    j.kv("recousure_trous", st_g.trous);
    j.kv("recousure_attributs_differents", st_g.attr_differents);
    j.kv("fragments", fs.len());
    let ids: BTreeSet<u64> = fs.iter().map(|f| f.id).collect();
    j.kv("troncons", ids.len());
    let emp = graphe::empreinte_fragments(&fs);
    j.kv("empreinte_fragments", &emp);
    // Contrôle : recousure directe de tous les morceaux, et par deux
    // « départements » fictifs (moitiés nord et sud) recousus séparément.
    {
        let tout: Vec<tuiles::Morceau> = par_tile.iter().flatten().cloned().collect();
        let (d, _) = graphe::recoudre(graphe::de_morceaux(tout.clone()));
        j.kv("controle_recousure_directe_identique", (graphe::empreinte_fragments(&d) == emp) as u8);
        let mut iys: Vec<i32> = tile_ids.iter().map(|t| t.iy).collect();
        iys.sort();
        let med = iys[iys.len() / 2];
        let (na, nb): (Vec<_>, Vec<_>) = tout.into_iter().partition(|m| m.tile.iy < med);
        let (fa, _) = graphe::recoudre(graphe::de_morceaux(na));
        let (fb, _) = graphe::recoudre(graphe::de_morceaux(nb));
        let bord = fa.len() + fb.len();
        let (fab, sab) = graphe::recoudre(fa.into_iter().chain(fb).collect());
        j.kv("controle_departements_coupure_iy", med);
        j.kv("controle_departements_jointures_au_bord", sab.jointures);
        j.kv("controle_departements_fragments_avant", bord);
        j.kv("controle_departements_identique", (graphe::empreinte_fragments(&fab) == emp) as u8);
    }
    drop(par_tile);
    let km: f64 = fs.iter().map(|f| graphe::longueur_m(&f.pts)).sum::<f64>() / 1000.0;
    j.kv("longueur_totale_km", format!("{km:.3}"));
    if a.controle_km > 0.0 {
        j.kv("longueur_p07_km", format!("{:.3}", a.controle_km));
        j.kv("longueur_ecart_relatif", format!("{:.2e}", (km - a.controle_km) / a.controle_km));
    }
    let mut par_nature = [0f64; 12];
    for f in &fs {
        par_nature[f.attr.nature.min(11) as usize] += graphe::longueur_m(&f.pts) / 1000.0;
    }
    for (k, n) in graphe::NATURES.iter().enumerate() {
        j.kv(&format!("km_{}", n.replace([' ', ','], "_")), format!("{:.1}", par_nature[k]));
    }
    let complet = graphe::construire(&fs, |_| true);
    j.kv("graphe_complet_noeuds", complet.n());
    j.kv("graphe_complet_aretes", complet.m());
    drop(complet);

    // Graphe routier (réseau carrossable).
    let main = classes::valeurs_main();
    let ((mut g, stp), t_g) = ms(|| {
        let mut g = graphe::construire(&fs, |f| graphe::carrossable(&f.attr));
        let st = graphe::associer_ponts(&mut g, &fs, &ponts, |p, at, l| classes::charge(p, at, l, &main));
        (g, st)
    });
    j.kv("graphe_construction_ms", format!("{t_g:.0}"));
    j.kv("graphe_noeuds", g.n());
    j.kv("graphe_aretes", g.m());
    let mut deg = vec![0u32; g.n()];
    for e in 0..g.m() {
        deg[g.u[e] as usize] += 1;
        deg[g.v[e] as usize] += 1;
    }
    for d in 1..=4 {
        j.kv(&format!("noeuds_degre_{d}"), deg.iter().filter(|&&x| x == d).count());
    }
    j.kv("noeuds_degre_5_et_plus", deg.iter().filter(|&&x| x >= 5).count());
    j.kv("aretes_boucles", (0..g.m()).filter(|&e| g.u[e] == g.v[e]).count());
    let comp = graphe::composantes(&g);
    let mut taille: BTreeMap<u32, usize> = BTreeMap::new();
    for &c in &comp {
        *taille.entry(c).or_default() += 1;
    }
    let plus_grande = taille.values().copied().max().unwrap_or(0);
    j.kv("composantes", taille.len());
    j.kv("plus_grande_composante_part", format!("{:.4}", plus_grande as f64 / g.n() as f64));
    j.kv("ponts_associes", stp.ponts_associes);
    j.kv("aretes_sur_ouvrage", stp.aretes_sur_ouvrage);
    j.kv("aretes_pont_bdtopo", stp.aretes_pont);
    j.kv("aretes_ouvrage_implicite", stp.aretes_implicites);
    j.kv("empreinte_graphe", graphe::empreinte(&g));
    let (rss, hwm) = mesure::rss_mo();
    j.kv("rss_mo", format!("{rss:.0}"));
    j.kv("rss_max_mo", format!("{hwm:.0}"));

    let cls = classes::classes();
    let csr = Csr::de(&g);
    let mut w: Vec<(String, Vec<u32>)> = cls.iter().map(|c| (c.nom.to_string(), classes::metrique(&g, c, Sens::Physique))).collect();
    for k in [3usize, 2] {
        let mut c = cls[k].clone();
        c.masse_t = 0;
        w.push((format!("{}-libre", cls[k].nom), classes::metrique(&g, &c, Sens::Physique)));
    }
    w.push(("voiture-code".into(), classes::metrique(&g, &cls[0], Sens::Code)));
    // Lieux : accrochés, pour chaque métrique, au nœud le plus proche de la
    // composante fortement connexe principale de cette métrique.
    let mut lieux = Vec::new();
    let mut lieux_m: Vec<Vec<u32>> = Vec::new();
    if !a.lieux.is_empty() {
        let coords: Vec<(String, f64, f64)> = a
            .lieux
            .split(';')
            .map(|l| {
                let (nom, c) = l.split_once('=').unwrap();
                let (x, y) = c.split_once(',').unwrap();
                (nom.to_string(), x.parse::<f64>().unwrap() * 16.0, y.parse::<f64>().unwrap() * 16.0)
            })
            .collect();
        let proche = |ok: &dyn Fn(usize) -> bool, x: f64, y: f64| {
            (0..g.n())
                .filter(|&v| ok(v))
                .min_by_key(|&v| {
                    let (dx, dy) = (g.xy[v][0] as f64 - x, g.xy[v][1] as f64 - y);
                    ((dx * dx + dy * dy) as i64, v)
                })
                .unwrap() as u32
        };
        for (_, w) in &w {
            let mut ok = vec![false; g.n()];
            for e in 0..g.m() {
                if w[2 * e] < INF || w[2 * e + 1] < INF {
                    ok[g.u[e] as usize] = true;
                    ok[g.v[e] as usize] = true;
                }
            }
            let r = proche(&|v| ok[v], coords[0].1, coords[0].2);
            let cfc = algos::cfc_de(&csr, w, r);
            lieux_m.push(coords.iter().map(|c| proche(&|v| cfc[v], c.1, c.2)).collect());
        }
        lieux = coords.iter().enumerate().map(|(i, c)| (c.0.clone(), lieux_m[0][i])).collect();
    }
    let racine = lieux.first().map(|l| l.1).unwrap_or(0);
    let _ = &mut g;
    Monde { fs, g, ponts, csr, cls, w, lieux, lieux_m, racine }
}

/// Paires aléatoires (graine fixe) dans la composante fortement connexe de
/// la racine, pour la métrique de la voiture.
pub fn paires(m: &Monde, n: usize) -> (Vec<(u32, u32)>, usize) {
    let cfc = algos::cfc_de(&m.csr, &m.w[0].1, m.racine);
    let noeuds: Vec<u32> = (0..m.g.n() as u32).filter(|&v| cfc[v as usize]).collect();
    let mut a = Alea(2026);
    ((0..n).map(|_| (noeuds[a.sous(noeuds.len())], noeuds[a.sous(noeuds.len())])).collect(), noeuds.len())
}

fn ecrire(chemin: &std::path::Path, contenu: &str) {
    std::fs::write(chemin, contenu).unwrap();
}

/// Longueur (km) d'un trajet.
pub fn km(m: &Monde, arcs: &[u32]) -> f64 {
    arcs.iter().map(|&a| m.g.long_mm[(a / 2) as usize] as f64).sum::<f64>() / 1e6
}

fn empreinte_trajets(t: &[Option<Trajet>]) -> String {
    let mut h = blake3::Hasher::new();
    for x in t {
        match x {
            None => {
                h.update(&[0xFF; 4]);
            }
            Some(t) => {
                h.update(&t.duree.to_le_bytes());
                for a in &t.arcs {
                    h.update(&a.to_le_bytes());
                }
                h.update(&[0xFE; 4]);
            }
        }
    }
    h.finalize().to_hex()[..16].to_string()
}

fn empreinte_metrique(mt: &cch::Metric) -> String {
    let mut h = blake3::Hasher::new();
    h.update(&mesure::u32_octets(&mt.forward));
    h.update(&mesure::u32_octets(&mt.backward));
    h.finalize().to_hex()[..16].to_string()
}

fn dijkstra_tous(m: &Monde, w: &[u32], p: &[(u32, u32)]) -> Vec<Option<Trajet>> {
    p.par_iter()
        .map_init(|| Etat::new(m.g.n()), |e, &(s, t)| algos::astar(&m.csr, w, e, s, t, &Borne::Zero))
        .collect()
}

fn cch_tous(m: &Monde, topo: &hier::Topo, mt: &cch::Metric, w: &[u32], p: &[(u32, u32)]) -> Vec<Option<Trajet>> {
    let vue = topo.cch.view();
    p.par_iter()
        .map_init(|| hier::RequeteCch::new(&vue), |q, &(s, t)| q.trajet(mt, &m.csr, w, s, t))
        .collect()
}

fn duree(t: &Option<Trajet>) -> u32 {
    t.as_ref().map_or(INF, |t| t.duree)
}

// ------------------------------------------------------------------ campagne

fn campagne(a: &Args) {
    std::fs::create_dir_all(&a.sortie).unwrap();
    let mut j = Journal { lignes: String::from("cle,valeur\n") };
    let (m, t_total) = ms(|| charger(a, &mut j));
    j.kv("chargement_total_ms", format!("{t_total:.0}"));
    let g = &m.g;

    // Ponts : un par ouvrage (charge, attributs), et distribution.
    let mut csv_ponts = String::from("pont,importance,largeur_dm,longueur_dm,franchit,toponyme,aretes,charge_t\n");
    let mut par_pont: BTreeMap<Option<u64>, (u16, usize)> = BTreeMap::new();
    let mut implicites: Vec<u16> = Vec::new();
    for e in 0..g.m() {
        if let Some(o) = g.ouvrage[e] {
            let o = &g.ouvrages[o as usize];
            match o.pont {
                Some(_) => {
                    let x = par_pont.entry(o.pont).or_insert((o.charge_t, 0));
                    x.0 = x.0.min(o.charge_t);
                    x.1 += 1;
                }
                None => implicites.push(o.charge_t),
            }
        }
    }
    let mut dist: BTreeMap<u16, (usize, usize)> = BTreeMap::new();
    for p in &m.ponts {
        if let Some(&(c, n)) = par_pont.get(&Some(p.id)) {
            writeln!(csv_ponts, "{},{},{},{},{},\"{}\",{},{}", p.id, p.importance, p.largeur_dm, p.longueur_dm, p.franchit, p.toponyme.replace('"', "'"), n, c).unwrap();
            dist.entry(c).or_default().0 += 1;
        }
    }
    for c in implicites {
        dist.entry(c).or_default().1 += 1;
    }
    ecrire(&a.sortie.join("ponts.csv"), &csv_ponts);
    let mut csv_dist = String::from("charge_t,ponts_bdtopo,aretes_ouvrage_implicite\n");
    for (c, (p, i)) in dist.iter().rev() {
        writeln!(csv_dist, "{c},{p},{i}").unwrap();
    }
    ecrire(&a.sortie.join("ponts-distribution.csv"), &csv_dist);

    // Refus par classe.
    let mut csv_refus = String::from("classe,aretes_permises,nature,acces,largeur,hauteur,poids,pont,ponts_bdtopo_refuses,km_permis\n");
    for c in &m.cls {
        let mut n = BTreeMap::new();
        let mut ponts_ref = BTreeSet::new();
        let mut kmp = 0.0;
        for e in 0..g.m() {
            let o = g.ouvrage[e].map(|o| &g.ouvrages[o as usize]);
            let r = classes::refus(c, &g.attr[e], o);
            *n.entry(format!("{r:?}")).or_insert(0usize) += 1;
            if r == Refus::Pont {
                if let Some(p) = o.and_then(|o| o.pont) {
                    ponts_ref.insert(p);
                }
            }
            if r == Refus::Aucun {
                kmp += g.long_mm[e] as f64 / 1e6;
            }
        }
        let f = |k: &str| n.get(k).copied().unwrap_or(0);
        writeln!(csv_refus, "{},{},{},{},{},{},{},{},{},{:.0}", c.nom, f("Aucun"), f("Nature"), f("Acces"), f("Largeur"), f("Hauteur"), f("Poids"), f("Pont"), ponts_ref.len(), kmp).unwrap();
    }
    ecrire(&a.sortie.join("refus.csv"), &csv_refus);

    // ------------------------------------------------ Prétraitements
    let mut csv_pre = String::from("methode,metrique,etape,ms,octets,octets_zstd\n");
    let ((topo, t_ordre, t_struct), t_topo) = ms(|| hier::Topo::construire(&m.csr, &g.xy));
    let (hmax, hmoy) = topo.profondeurs();
    j.kv("cch_arcs", topo.cch.cch_arc_count());
    j.kv("cch_arcs_par_noeud", format!("{:.2}", topo.cch.cch_arc_count() as f64 / g.n() as f64));
    j.kv("cch_hauteur_arbre", hmax);
    j.kv("cch_profondeur_moyenne", format!("{hmoy:.0}"));
    j.kv("cch_topologie_ms", format!("{t_topo:.0}"));
    let ordre_oct = mesure::u32_octets(&topo.cch.order);
    writeln!(csv_pre, "cch,-,ordre,{:.0},{},{}", t_ordre * 1000.0, ordre_oct.len(), mesure::zstd_octets(&ordre_oct)).unwrap();
    let st = topo.octets_structure();
    writeln!(csv_pre, "cch,-,structure,{:.0},{},", t_struct * 1000.0, st).unwrap();
    let mut metriques: Vec<cch::Metric> = Vec::new();
    let mut entrees: Vec<Vec<u32>> = Vec::new();
    for (nom, w) in &m.w {
        let we = topo.poids(w);
        let mut ts = Vec::new();
        let mut mt = None;
        for _ in 0..5 {
            let (x, t) = ms(|| topo.personnaliser(&we));
            ts.push(t);
            mt = Some(x);
        }
        let mt = mt.unwrap();
        let mut oct = mesure::u32_octets(&mt.forward);
        oct.extend(mesure::u32_octets(&mt.backward));
        writeln!(csv_pre, "cch,{nom},personnalisation,{:.1},{},{}", pct(&ts, 50.0), oct.len(), mesure::zstd_octets(&oct)).unwrap();
        // Personnalisation sur un seul thread (référence).
        let pool1 = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let (_, t1) = ms(|| pool1.install(|| topo.personnaliser(&we)));
        writeln!(csv_pre, "cch,{nom},personnalisation_1_thread,{t1:.1},,").unwrap();
        metriques.push(mt);
        entrees.push(we);
    }
    // CH (fast_paths) pour les métriques principales.
    let mut chs = Vec::new();
    for k in 0..PRINCIPALES {
        let (fg, t) = ms(|| hier::ch_preparer(&m.csr, &m.w[k].1, None));
        let o = fast_paths::get_node_ordering(&fg);
        let (_, t2) = ms(|| hier::ch_preparer(&m.csr, &m.w[k].1, Some(&o)));
        let oct = (fg.get_num_out_edges() + fg.get_num_in_edges()) * 16 + g.n() * 16;
        writeln!(csv_pre, "ch,{},preparation,{t:.0},{oct},", m.w[k].0).unwrap();
        writeln!(csv_pre, "ch,{},preparation_avec_ordre,{t2:.0},,", m.w[k].0).unwrap();
        chs.push((fg, o));
    }
    // ALT.
    let mut alts = Vec::new();
    for k in 0..PRINCIPALES {
        let (r, t) = ms(|| Reperes::choisir(&m.csr, &m.w[k].1, m.racine, a.reperes));
        writeln!(csv_pre, "alt,{},reperes_{},{t:.0},{},", m.w[k].0, a.reperes, r.octets()).unwrap();
        alts.push(r);
    }
    let (rss, hwm) = mesure::rss_mo();
    j.kv("rss_apres_pretraitements_mo", format!("{rss:.0}"));
    j.kv("rss_max_apres_pretraitements_mo", format!("{hwm:.0}"));
    ecrire(&a.sortie.join("pretraitements.csv"), &csv_pre);

    // ------------------------------------------------ Requêtes
    let (p, ncfc) = paires(&m, a.paires);
    j.kv("noeuds_cfc_voiture", ncfc);
    let mut refs: Vec<Vec<Option<Trajet>>> = Vec::new();
    let mut cchs: Vec<Vec<Option<Trajet>>> = Vec::new();
    for k in 0..m.w.len() {
        let r = if k < PRINCIPALES { dijkstra_tous(&m, &m.w[k].1, &p) } else { Vec::new() };
        let c = cch_tous(&m, &topo, &metriques[k], &m.w[k].1, &p);
        if k < PRINCIPALES {
            let ecarts = (0..p.len()).filter(|&i| duree(&r[i]) != duree(&c[i])).count();
            let imposs = r.iter().filter(|x| x.is_none()).count();
            j.kv(&format!("exact_cch_ecarts_{}", m.w[k].0), ecarts);
            j.kv(&format!("impossibles_{}", m.w[k].0), imposs);
            j.kv(&format!("empreinte_dijkstra_{}", m.w[k].0), empreinte_trajets(&r));
            // CH sur toutes les paires.
            let fg = &chs[k].0;
            let dch: Vec<u32> = p
                .par_iter()
                .map_init(|| fast_paths::create_calculator(fg), |pc, &(s, t)| pc.calc_path(fg, s as usize, t as usize).map_or(INF, |sp| if sp.get_weight() >= INF as usize { INF } else { sp.get_weight() as u32 }))
                .collect();
            j.kv(&format!("exact_ch_ecarts_{}", m.w[k].0), (0..p.len()).filter(|&i| dch[i] != duree(&r[i])).count());
        }
        j.kv(&format!("empreinte_cch_{}", m.w[k].0), empreinte_trajets(&c));
        refs.push(r);
        cchs.push(c);
    }
    // Chronométrage une par une, sur un thread.
    let mut csv_req = String::from("metrique,methode,paire,us,explores,duree_ms\n");
    let mut resume = String::from("metrique,methode,requetes,mediane_us,p99_us,max_us,explores_moyens,ecarts\n");
    let vue = topo.cch.view();
    for k in 0..PRINCIPALES {
        let w = &m.w[k].1;
        let vmax = classes::vmax_metrique(g, &m.cls[k]);
        let mut e = Etat::new(g.n());
        let mut q = hier::RequeteCch::new(&vue);
        let fg = &chs[k].0;
        let mut pc = fast_paths::create_calculator(fg);
        for meth in ["dijkstra", "bidir", "astar", "alt", "ch", "cch"] {
            let nb = match meth {
                "ch" | "cch" => p.len(),
                _ => a.banc.min(p.len()),
            };
            let mut us = Vec::with_capacity(nb);
            let mut expl = Vec::with_capacity(nb);
            let mut ecarts = 0;
            for (i, &(s, t)) in p.iter().take(nb).enumerate() {
                let t0 = std::time::Instant::now();
                let (d, x) = match meth {
                    "dijkstra" => {
                        let r = algos::astar(&m.csr, w, &mut e, s, t, &Borne::Zero);
                        (duree(&r), r.map_or(0, |r| r.explores))
                    }
                    "bidir" => {
                        let r = algos::bidir(&m.csr, w, &mut e, s, t);
                        (duree(&r), r.map_or(0, |r| r.explores))
                    }
                    "astar" => {
                        let r = algos::astar(&m.csr, w, &mut e, s, t, &Borne::Euclide { xy: &g.xy, vmax });
                        (duree(&r), r.map_or(0, |r| r.explores))
                    }
                    "alt" => {
                        let r = algos::astar(&m.csr, w, &mut e, s, t, &Borne::Alt { reperes: &alts[k] });
                        (duree(&r), r.map_or(0, |r| r.explores))
                    }
                    "ch" => {
                        let r = pc.calc_path(fg, s as usize, t as usize);
                        (r.map_or(INF, |sp| if sp.get_weight() >= INF as usize { INF } else { sp.get_weight() as u32 }), 0)
                    }
                    _ => {
                        let r = q.trajet(&metriques[k], &m.csr, w, s, t);
                        (duree(&r), r.map_or(0, |r| r.explores))
                    }
                };
                let dt = t0.elapsed().as_secs_f64() * 1e6;
                if d != duree(&refs[k][i]) {
                    ecarts += 1;
                }
                us.push(dt);
                expl.push(x as f64);
                if i < 2000 {
                    writeln!(csv_req, "{},{meth},{i},{dt:.1},{x},{d}", m.w[k].0).unwrap();
                }
            }
            writeln!(resume, "{},{meth},{nb},{:.1},{:.1},{:.1},{:.0},{ecarts}", m.w[k].0, pct(&us, 50.0), pct(&us, 99.0), pct(&us, 100.0), moyenne(&expl)).unwrap();
            println!("{} {meth} médiane {:.1} µs p99 {:.1} µs", m.w[k].0, pct(&us, 50.0), pct(&us, 99.0));
        }
    }
    ecrire(&a.sortie.join("requetes.csv"), &csv_req);
    ecrire(&a.sortie.join("requetes-resume.csv"), &resume);

    // ------------------------------------------------ Charge limite
    let idx = |nom: &str| m.w.iter().position(|x| x.0 == nom).unwrap();
    let mut csv_ch = String::from("ensemble,comparaison,paires_finies_ref,chemins_differents,impossibles,detour_km_moyen,detour_km_p99,detour_duree_moyen,detour_duree_p99\n");
    // « principal » : les deux extrémités sur le réseau du porte-char sans
    // limite de charge (routes à deux voies et plus).
    let kpl = idx("porte-char-libre");
    let mut princ = vec![false; g.n()];
    for e in 0..g.m() {
        if m.w[kpl].1[2 * e] < INF || m.w[kpl].1[2 * e + 1] < INF {
            princ[g.u[e] as usize] = true;
            princ[g.v[e] as usize] = true;
        }
    }
    for ens in ["toutes", "principal"] {
        let sel: Vec<usize> = (0..p.len()).filter(|&i| ens == "toutes" || (princ[p[i].0 as usize] && princ[p[i].1 as usize] && cchs[kpl][i].is_some())).collect();
        for (ref_, alt) in [("camion", "leclerc"), ("camion", "porte-char"), ("leclerc-libre", "leclerc"), ("porte-char-libre", "porte-char"), ("voiture", "voiture-code")] {
            let (ka, kb) = (idx(ref_), idx(alt));
            let (mut fin, mut diff, mut imp) = (0, 0, 0);
            let mut dk = Vec::new();
            let mut dd = Vec::new();
            for &i in &sel {
                let (ra, rb) = (&cchs[ka][i], &cchs[kb][i]);
                if let Some(ra) = ra {
                    fin += 1;
                    match rb {
                        None => imp += 1,
                        Some(rb) => {
                            if ra.arcs != rb.arcs {
                                diff += 1;
                            }
                            let (la, lb) = (km(&m, &ra.arcs), km(&m, &rb.arcs));
                            if la > 0.0 {
                                dk.push(lb / la - 1.0);
                            }
                            if ra.duree > 0 {
                                dd.push(rb.duree as f64 / ra.duree as f64 - 1.0);
                            }
                        }
                    }
                }
            }
            writeln!(csv_ch, "{ens},{ref_}>{alt},{fin},{diff},{imp},{:.4},{:.4},{:.4},{:.4}", moyenne(&dk), pct(&dk, 99.0), moyenne(&dd), pct(&dd, 99.0)).unwrap();
        }
    }
    // Ponts refusés au Leclerc et au porte-char sur les trajets des camions.
    {
        let kc = idx("camion");
        let mut cnt: BTreeMap<u64, usize> = BTreeMap::new();
        for t in cchs[kc].iter().flatten() {
            let mut vus = BTreeSet::new();
            for &arc in &t.arcs {
                if let Some(o) = g.ouvrage[(arc / 2) as usize] {
                    let o = &g.ouvrages[o as usize];
                    if let Some(pid) = o.pont {
                        if vus.insert(pid) {
                            *cnt.entry(pid).or_default() += 1;
                        }
                    }
                }
            }
        }
        for (nom, masse) in [("leclerc", m.cls[3].masse_t), ("porte-char", m.cls[2].masse_t)] {
            let mut refuses = 0;
            let mut traversees = 0;
            for (&pid, &n) in &cnt {
                let c = (0..g.m()).find_map(|e| g.ouvrage[e].map(|o| &g.ouvrages[o as usize]).filter(|o| o.pont == Some(pid)).map(|o| o.charge_t)).unwrap();
                if c < masse {
                    refuses += 1;
                    traversees += n;
                }
            }
            j.kv(&format!("ponts_des_camions_refuses_{nom}"), refuses);
            j.kv(&format!("traversees_refusees_{nom}"), traversees);
        }
        j.kv("ponts_traverses_par_les_camions", cnt.len());
    }
    ecrire(&a.sortie.join("charge.csv"), &csv_ch);

    // Trajets typiques.
    let mut csv_typ = String::from("trajet,metrique,duree_min,km,ponts,charge_min_t,arcs\n");
    let li = |n: &str| m.lieux.iter().position(|l| l.0 == n).unwrap();
    let mut q = hier::RequeteCch::new(&vue);
    for tr in a.trajets.split(',').filter(|x| !x.is_empty()) {
        let (sa, sb) = tr.split_once('>').unwrap();
        for k in 0..m.w.len() {
            let (s, t) = (m.lieux_m[k][li(sa)], m.lieux_m[k][li(sb)]);
            match q.trajet(&metriques[k], &m.csr, &m.w[k].1, s, t) {
                None => writeln!(csv_typ, "{tr},{},impossible,,,,", m.w[k].0).unwrap(),
                Some(r) => {
                    let os: Vec<u16> = r.arcs.iter().filter_map(|&a| g.ouvrage[(a / 2) as usize]).map(|o| g.ouvrages[o as usize].charge_t).collect();
                    writeln!(csv_typ, "{tr},{},{:.1},{:.1},{},{},{}", m.w[k].0, r.duree as f64 / 60000.0, km(&m, &r.arcs), os.len(), os.iter().min().copied().unwrap_or(0), r.arcs.len()).unwrap();
                }
            }
        }
    }
    ecrire(&a.sortie.join("typiques.csv"), &csv_typ);

    // ------------------------------------------------ Modifications
    modifications(a, &m, &topo, &metriques, &entrees, &chs, &cchs, &p, &mut j);
    // ------------------------------------------------ Charge de jeu
    jeu(a, &m, &topo, &metriques, &entrees, &mut j);
    // ------------------------------------------------ Suivi en jeu
    suivi(a, &m, &topo, &metriques, &mut j);

    let (rss, hwm) = mesure::rss_mo();
    j.kv("rss_fin_mo", format!("{rss:.0}"));
    j.kv("rss_max_fin_mo", format!("{hwm:.0}"));
    ecrire(&a.sortie.join("construction.csv"), &j.lignes);
}

/// Arcs d'entrée (positions CSR) de toutes les arêtes données.
fn positions(topo: &hier::Topo, aretes: &[u32]) -> Vec<u32> {
    aretes.iter().flat_map(|&e| [topo.pos_arc[2 * e as usize], topo.pos_arc[2 * e as usize + 1]]).collect()
}

/// Applique une modification (arêtes coupées ou rouvertes) à toutes les
/// métriques ; rend (ms de personnalisation partielle, arcs recalculés) par
/// métrique.
fn appliquer(topo: &hier::Topo, br: &mut hier::Brouillon, metriques: &mut [cch::Metric], entrees: &mut [Vec<u32>], ws: &mut [Vec<u32>], aretes: &[u32], couper: bool, base: &[(String, Vec<u32>)]) -> Vec<(f64, usize)> {
    let pos = positions(topo, aretes);
    let mut out = Vec::new();
    for k in 0..metriques.len() {
        for &e in aretes {
            for d in 0..2 {
                let arc = 2 * e as usize + d;
                let v = if couper { INF } else { base[k].1[arc] };
                ws[k][arc] = v;
                entrees[k][topo.pos_arc[arc] as usize] = v;
            }
        }
        let (n, t) = ms(|| topo.personnaliser_partiel_avec(&entrees[k], &mut metriques[k], &pos, br));
        out.push((t, n));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn modifications(
    a: &Args,
    m: &Monde,
    topo: &hier::Topo,
    metriques0: &[cch::Metric],
    entrees0: &[Vec<u32>],
    chs: &[(fast_paths::FastGraph, Vec<usize>)],
    cchs: &[Vec<Option<Trajet>>],
    p: &[(u32, u32)],
    j: &mut Journal,
) {
    let g = &m.g;
    let kc = 1; // camion
    // Usage des arêtes par les trajets des camions.
    let mut usage = vec![0u32; g.m()];
    for t in cchs[kc].iter().flatten() {
        for &arc in &t.arcs {
            usage[(arc / 2) as usize] += 1;
        }
    }
    // Pont d'un axe majeur : le pont BD TOPO d'importance ≤ 2 le plus emprunté.
    let mut par_pont: BTreeMap<u64, (u32, Vec<u32>)> = BTreeMap::new();
    for e in 0..g.m() {
        if let Some(o) = g.ouvrage[e] {
            let o = &g.ouvrages[o as usize];
            if let Some(pid) = o.pont {
                if g.attr[e].importance <= 2 {
                    let x = par_pont.entry(pid).or_default();
                    x.0 = x.0.max(usage[e]);
                    x.1.push(e as u32);
                }
            }
        }
    }
    let (pont, (_, aretes_pont)) = par_pont.iter().max_by_key(|(id, x)| (x.0, std::cmp::Reverse(**id))).map(|(a, b)| (*a, b.clone())).unwrap_or((0, (0, vec![])));
    let toponyme = m.ponts.iter().find(|x| x.id == pont).map(|x| x.toponyme.clone()).unwrap_or_default();
    j.kv("pont_detruit", pont);
    j.kv("pont_detruit_toponyme", format!("\"{toponyme}\""));
    j.kv("pont_detruit_aretes", aretes_pont.len());
    j.kv("pont_detruit_trajets_camion", usage[aretes_pont.first().copied().unwrap_or(0) as usize]);
    // Routes coupées : arêtes empruntées, tirées avec une graine fixe.
    let empruntees: Vec<u32> = (0..g.m() as u32).filter(|&e| usage[e as usize] > 0 && g.ouvrage[e as usize].is_none()).collect();
    let mut al = Alea(99);
    let mut tirees: Vec<u32> = Vec::new();
    while tirees.len() < 1000.min(empruntees.len()) {
        let e = empruntees[al.sous(empruntees.len())];
        if !tirees.contains(&e) {
            tirees.push(e);
        }
    }
    let mut csv = String::from("scenario,metrique,aretes,partiel_ms,arcs_recalcules,complet_ms,identique_complet,ch_avec_ordre_ms,trajets_touches,replanif_ms,replanif_us_par_trajet,ecarts_dijkstra\n");
    let mut scenarios: Vec<(String, Vec<u32>)> = vec![("pont".into(), aretes_pont.clone())];
    for n in [1usize, 10, 100, 1000] {
        scenarios.push((format!("coupures_{n}"), tirees[..n.min(tirees.len())].to_vec()));
    }
    let mut br = hier::Brouillon::new(topo);
    for (nom, aretes) in &scenarios {
        let mut mt: Vec<cch::Metric> = metriques0.to_vec();
        let mut en: Vec<Vec<u32>> = entrees0.to_vec();
        let mut ws: Vec<Vec<u32>> = m.w.iter().map(|x| x.1.clone()).collect();
        let res = appliquer(topo, &mut br, &mut mt, &mut en, &mut ws, aretes, true, &m.w);
        let coupe: BTreeSet<u32> = aretes.iter().copied().collect();
        for k in 0..PRINCIPALES {
            let (full, tf) = ms(|| topo.personnaliser(&en[k]));
            let ident = full == mt[k];
            // CH : nouvelle préparation avec l'ordre existant.
            let (_, tch) = ms(|| hier::ch_preparer(&m.csr, &ws[k], Some(&chs[k].1)));
            // Trajets touchés et replanification.
            let touches: Vec<usize> = (0..p.len()).filter(|&i| cchs[k][i].as_ref().is_some_and(|t| t.arcs.iter().any(|&a| coupe.contains(&(a / 2))))).collect();
            let sub: Vec<(u32, u32)> = touches.iter().map(|&i| p[i]).collect();
            let vue = topo.cch.view();
            let mut q = hier::RequeteCch::new(&vue);
            let (nouv, tr) = ms(|| sub.iter().map(|&(s, t)| q.trajet(&mt[k], &m.csr, &ws[k], s, t)).collect::<Vec<_>>());
            // Contrôle : Dijkstra sur le réseau modifié.
            let refd = dijkstra_tous_w(m, &ws[k], &sub);
            let ecarts = (0..sub.len()).filter(|&i| duree(&nouv[i]) != duree(&refd[i])).count();
            writeln!(
                csv,
                "{nom},{},{},{:.3},{},{tf:.1},{},{tch:.0},{},{tr:.1},{:.1},{ecarts}",
                m.w[k].0,
                aretes.len(),
                res[k].0,
                res[k].1,
                ident as u8,
                touches.len(),
                tr * 1000.0 / touches.len().max(1) as f64
            )
            .unwrap();
        }
        // Réouverture : retour exact à la métrique d'origine.
        let res2 = appliquer(topo, &mut br, &mut mt, &mut en, &mut ws, aretes, false, &m.w);
        for k in 0..PRINCIPALES {
            writeln!(csv, "{nom}_reouverture,{},{},{:.3},{},,{},,,,,", m.w[k].0, aretes.len(), res2[k].0, res2[k].1, (mt[k] == metriques0[k]) as u8).unwrap();
        }
    }
    ecrire(&a.sortie.join("modifs.csv"), &csv);
}

fn dijkstra_tous_w(m: &Monde, w: &[u32], p: &[(u32, u32)]) -> Vec<Option<Trajet>> {
    dijkstra_tous(m, w, p)
}

/// Scénario 5 : 1 000 convois, 100 routes coupées sur leurs trajets ;
/// replanification dans la boucle (3 ms par tick) ou en tâche de fond.
fn jeu(a: &Args, m: &Monde, topo: &hier::Topo, metriques0: &[cch::Metric], entrees0: &[Vec<u32>], j: &mut Journal) {
    let g = &m.g;
    let cfc = algos::cfc_de(&m.csr, &m.w[0].1, m.racine);
    let noeuds: Vec<u32> = (0..g.n() as u32).filter(|&v| cfc[v as usize]).collect();
    let mut al = Alea(4242);
    // 60 % camions, 20 % voitures, 10 % porte-chars, 10 % Leclerc.
    let convois: Vec<(usize, u32, u32)> = (0..a.convois)
        .map(|i| {
            let k = match i % 10 {
                0..=5 => 1,
                6 | 7 => 0,
                8 => 2,
                _ => 3,
            };
            (k, noeuds[al.sous(noeuds.len())], noeuds[al.sous(noeuds.len())])
        })
        .collect();
    let vue = topo.cch.view();
    let (trajets, t_init) = ms(|| {
        convois
            .par_iter()
            .map_init(|| hier::RequeteCch::new(&vue), |q, &(k, s, t)| q.trajet(&metriques0[k], &m.csr, &m.w[k].1, s, t))
            .collect::<Vec<_>>()
    });
    j.kv("jeu_convois", convois.len());
    j.kv("jeu_calcul_initial_ms_16_threads", format!("{t_init:.1}"));
    let mut usage = vec![0u32; g.m()];
    for t in trajets.iter().flatten() {
        for &arc in &t.arcs {
            usage[(arc / 2) as usize] += 1;
        }
    }
    let empruntees: Vec<u32> = (0..g.m() as u32).filter(|&e| usage[e as usize] > 0).collect();
    let mut coupees = BTreeSet::new();
    while coupees.len() < 100.min(empruntees.len()) {
        coupees.insert(empruntees[al.sous(empruntees.len())]);
    }
    let aretes: Vec<u32> = coupees.iter().copied().collect();
    let mut br = hier::Brouillon::new(topo);
    let mut mt: Vec<cch::Metric> = metriques0.to_vec();
    let mut en: Vec<Vec<u32>> = entrees0.to_vec();
    let mut ws: Vec<Vec<u32>> = m.w.iter().map(|x| x.1.clone()).collect();
    let (res, t_maj) = ms(|| appliquer(topo, &mut br, &mut mt[..PRINCIPALES], &mut en[..PRINCIPALES], &mut ws[..PRINCIPALES], &aretes, true, &m.w));
    j.kv("jeu_maj_4_metriques_ms", format!("{t_maj:.2}"));
    j.kv("jeu_maj_arcs_recalcules", res.iter().map(|r| r.1).sum::<usize>());
    // Détection : index arête → convois (construit au calcul des trajets).
    let mut index: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let (_, t_index) = ms(|| {
        for (i, t) in trajets.iter().enumerate() {
            if let Some(t) = t {
                for &arc in &t.arcs {
                    index.entry(arc / 2).or_default().push(i as u32);
                }
            }
        }
    });
    let (touches, t_det) = ms(|| {
        let mut s = BTreeSet::new();
        for e in &aretes {
            if let Some(v) = index.get(e) {
                s.extend(v.iter().copied());
            }
        }
        s.into_iter().collect::<Vec<u32>>()
    });
    j.kv("jeu_index_construction_ms", format!("{t_index:.1}"));
    j.kv("jeu_detection_ms", format!("{t_det:.3}"));
    j.kv("jeu_convois_touches", touches.len());
    let mut csv = String::from("cas,replanifications,cpu_ms,ticks_a_3ms,fond_ms_4_threads,fond_ms_15_threads,jeu_s_a_x100_4_threads\n");
    for (cas, liste) in [("coupures_100", touches.clone()), ("tous", (0..convois.len() as u32).collect::<Vec<_>>())] {
        // Dans la boucle de simulation : budget de 3 ms par tick.
        let mut q = hier::RequeteCch::new(&vue);
        let mut ticks = 0usize;
        let mut k = 0usize;
        let mut cpu = 0.0;
        while k < liste.len() {
            ticks += 1;
            let t0 = std::time::Instant::now();
            while k < liste.len() && t0.elapsed().as_secs_f64() * 1000.0 < 3.0 {
                let (c, s, t) = convois[liste[k] as usize];
                std::hint::black_box(q.trajet(&mt[c], &m.csr, &ws[c], s, t));
                k += 1;
            }
            cpu += t0.elapsed().as_secs_f64() * 1000.0;
        }
        // En tâche de fond.
        let fond = |n: usize| {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap();
            // Une requête CCH par thread (allouée une fois, comme dans le
            // jeu), puis répartition dynamique des convois.
            let suivant = std::sync::atomic::AtomicUsize::new(0);
            let (_, t) = ms(|| {
                pool.install(|| {
                    rayon::scope(|sc| {
                        for _ in 0..n {
                            sc.spawn(|_| {
                                let mut q = hier::RequeteCch::new(&vue);
                                loop {
                                    let k = suivant.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    if k >= liste.len() {
                                        break;
                                    }
                                    let (c, s, t) = convois[liste[k] as usize];
                                    std::hint::black_box(q.trajet(&mt[c], &m.csr, &ws[c], s, t));
                                }
                            });
                        }
                    })
                })
            });
            t
        };
        let (f4, f15) = (fond(4), fond(15));
        writeln!(csv, "{cas},{},{cpu:.1},{ticks},{f4:.1},{f15:.1},{:.1}", liste.len(), f4 / 1000.0 * 100.0).unwrap();
    }
    // Mémoire d'un itinéraire : arcs en u32, et différences zstd.
    let arcs: usize = trajets.iter().flatten().map(|t| t.arcs.len()).sum();
    j.kv("jeu_arcs_par_trajet_moyen", format!("{:.0}", arcs as f64 / trajets.len().max(1) as f64));
    ecrire(&a.sortie.join("jeu.csv"), &csv);
}

/// Scénario 8 : suivre un itinéraire en tile active (morceaux, positions
/// locales f32) et en tile inerte (trajet abstrait) ; sauvegarde.
fn suivi(a: &Args, m: &Monde, topo: &hier::Topo, metriques: &[cch::Metric], j: &mut Journal) {
    let g = &m.g;
    let Some((s, t)) = a.trajets.split(',').next().and_then(|tr| tr.split_once('>')).map(|(x, y)| {
        let f = |n: &str| m.lieux_m[1][m.lieux.iter().position(|l| l.0 == n).unwrap()];
        (f(x), f(y))
    }) else {
        return;
    };
    let vue = topo.cch.view();
    let mut q = hier::RequeteCch::new(&vue);
    let k = 1;
    let Some(r) = q.trajet(&metriques[k], &m.csr, &m.w[k].1, s, t) else {
        j.kv("suivi", "impossible");
        return;
    };
    // Tile active : la suite des (tile, morceau) et les points globaux.
    let (pts, t_geo) = ms(|| {
        let mut pts: Vec<[i64; 2]> = Vec::new();
        let mut tiles: Vec<TileId> = Vec::new();
        for &arc in &r.arcs {
            let e = (arc / 2) as usize;
            let f = &m.fs[g.frag[e] as usize];
            let mut p: Vec<[i64; 2]> = f.pts.iter().map(|x| [x[0], x[1]]).collect();
            if arc % 2 == 1 {
                p.reverse();
            }
            if let Some(l) = pts.last() {
                assert_eq!(*l, p[0], "itinéraire discontinu");
                pts.extend_from_slice(&p[1..]);
            } else {
                pts.extend(p);
            }
            for mo in &f.morceaux {
                if tiles.last() != Some(&mo.0) {
                    tiles.push(mo.0);
                }
            }
        }
        (pts, tiles)
    });
    // Positions locales f32 centrées : écart maximal à la position exacte.
    let mut ecart_max = 0f64;
    for p in &pts.0 {
        let tl = TileId::de_q(p[0], p[1]);
        let (cx, cy) = tl.centre_q();
        let lx = ((p[0] - cx) as f64 / 16.0) as f32;
        let ly = ((p[1] - cy) as f64 / 16.0) as f32;
        let ex = (lx as f64 - (p[0] - cx) as f64 / 16.0).abs().max((ly as f64 - (p[1] - cy) as f64 / 16.0).abs());
        ecart_max = ecart_max.max(ex);
        assert!(lx.abs() <= 500.0 && ly.abs() <= 500.0);
    }
    j.kv("suivi_arcs", r.arcs.len());
    j.kv("suivi_points", pts.0.len());
    j.kv("suivi_tiles_traversees", pts.1.len());
    j.kv("suivi_geometrie_ms", format!("{t_geo:.3}"));
    j.kv("suivi_ecart_f32_max_m", format!("{ecart_max:.2e}"));
    // Tile inerte : durées cumulées ; position au temps τ par recherche
    // dichotomique.
    let mut cumul = Vec::with_capacity(r.arcs.len());
    let mut c = 0u64;
    for &arc in &r.arcs {
        c += m.w[k].1[arc as usize] as u64;
        cumul.push(c);
    }
    let (_, t_pos) = ms(|| {
        let mut acc = 0usize;
        for i in 0..100_000u64 {
            let tau = i * c / 100_000;
            acc += cumul.partition_point(|&x| x <= tau);
        }
        std::hint::black_box(acc)
    });
    j.kv("suivi_position_inerte_ns", format!("{:.0}", t_pos * 1e6 / 100_000.0));
    // Sauvegarde (A-13) : une modification = identifiant du tronçon, rang de
    // début, état.
    let mut b = Vec::new();
    let mut prev = 0u64;
    let mut ids: Vec<(u64, u16)> = (0..100).map(|i| (g.id[i * g.m() / 100], g.r0[i * g.m() / 100])).collect();
    ids.sort();
    for (id, r0) in ids {
        let mut d = id - prev;
        prev = id;
        loop {
            let o = (d & 0x7F) as u8;
            d >>= 7;
            if d == 0 {
                b.push(o);
                break;
            }
            b.push(o | 0x80);
        }
        b.push(r0 as u8);
        b.push(1);
    }
    j.kv("sauvegarde_100_modifications_octets", b.len());
    j.kv("sauvegarde_100_modifications_zstd", mesure::zstd_octets(&b));
    let mut arcs_b = Vec::new();
    for &x in &r.arcs {
        arcs_b.extend(x.to_le_bytes());
    }
    j.kv("itineraire_octets_brut", arcs_b.len());
    j.kv("itineraire_octets_zstd", mesure::zstd_octets(&arcs_b));
    let _ = a;
}

// ------------------------------------------------------------ déterminisme

fn determinisme(a: &Args) {
    std::fs::create_dir_all(&a.sortie).unwrap();
    let mut j = Journal { lignes: String::new() };
    let (m, t) = ms(|| charger(a, &mut j));
    let (topo, _, _) = hier::Topo::construire(&m.csr, &m.g.xy);
    let mut hm = blake3::Hasher::new();
    let mut mts = Vec::new();
    for (_, w) in &m.w {
        let mt = topo.personnaliser(&topo.poids(w));
        hm.update(empreinte_metrique(&mt).as_bytes());
        mts.push(mt);
    }
    let (p, _) = paires(&m, a.paires.min(2000));
    let mut hc = blake3::Hasher::new();
    let mut hd = blake3::Hasher::new();
    for k in 0..PRINCIPALES {
        hc.update(empreinte_trajets(&cch_tous(&m, &topo, &mts[k], &m.w[k].1, &p)).as_bytes());
        hd.update(empreinte_trajets(&dijkstra_tous(&m, &m.w[k].1, &p[..500.min(p.len())])).as_bytes());
    }
    let ordre = blake3::hash(&mesure::u32_octets(&topo.cch.order)).to_hex()[..16].to_string();
    let ligne = format!(
        "{},{},{},{},{},{},{},{},{},{:.0}\n",
        a.etiquette,
        rayon::current_num_threads(),
        a.ordre,
        graphe::empreinte_fragments(&m.fs),
        graphe::empreinte(&m.g),
        ordre,
        &hm.finalize().to_hex()[..16],
        &hc.finalize().to_hex()[..16],
        &hd.finalize().to_hex()[..16],
        t
    );
    print!("{ligne}");
    let f = a.sortie.join("determinisme.csv");
    let neuf = !f.exists();
    let mut o = std::fs::OpenOptions::new().create(true).append(true).open(f).unwrap();
    if neuf {
        o.write_all(b"etiquette,threads,ordre,fragments,graphe,ordre_cch,metriques,trajets_cch,trajets_dijkstra,chargement_ms\n").unwrap();
    }
    o.write_all(ligne.as_bytes()).unwrap();
}

fn main() {
    let a = Args::parse();
    if a.threads > 0 {
        rayon::ThreadPoolBuilder::new().num_threads(a.threads).build_global().unwrap();
    }
    match a.mode.as_str() {
        "campagne" => campagne(&a),
        "determinisme" => determinisme(&a),
        "images" => images::images(&a),
        "fichier" => fichier(&a),
        _ => panic!("mode inconnu"),
    }
}

// ------------------------------------------------------------ fichier livré

fn varint(b: &mut Vec<u8>, mut v: u64) {
    loop {
        let o = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            b.push(o);
            return;
        }
        b.push(o | 0x80);
    }
}

fn zz(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Graphe routier tel que la chaîne le livrerait (un fichier national) :
/// nœuds, arêtes, dictionnaire d'attributs, charges, ordre CCH. Mesure sa
/// taille et le chargement en jeu (décodage, structure depuis l'ordre,
/// personnalisation des classes).
fn fichier(a: &Args) {
    std::fs::create_dir_all(&a.sortie).unwrap();
    let mut j = Journal { lignes: String::from("cle,valeur\n") };
    let m = charger(a, &mut j);
    let g = &m.g;
    let (topo, _, _) = hier::Topo::construire(&m.csr, &g.xy);
    let mut b = Vec::new();
    varint(&mut b, g.n() as u64);
    let (mut px, mut py) = (0i64, 0i64);
    for p in &g.xy {
        varint(&mut b, zz(p[1] - py));
        varint(&mut b, zz(p[0] - px));
        (px, py) = (p[0], p[1]);
    }
    let mut dict: Vec<tuiles::Attr> = g.attr.clone();
    dict.sort_by_key(|x| (x.nature, x.importance, x.pos_sol, x.largeur_dm, x.voies, x.sens, x.acces, x.urbain, x.vitesse, x.poids_dt, x.hauteur_dm, x.largeur_r_dm));
    dict.dedup();
    let cle = |x: &tuiles::Attr| (x.nature, x.importance, x.pos_sol, x.largeur_dm, x.voies, x.sens, x.acces, x.urbain, x.vitesse, x.poids_dt, x.hauteur_dm, x.largeur_r_dm);
    varint(&mut b, dict.len() as u64);
    for x in &dict {
        b.extend([x.nature, x.importance, x.pos_sol as u8, x.voies, x.sens, x.acces | (x.urbain as u8) << 7, x.vitesse]);
        for v in [x.largeur_dm, x.poids_dt, x.hauteur_dm, x.largeur_r_dm] {
            varint(&mut b, v as u64);
        }
    }
    varint(&mut b, g.m() as u64);
    let mut pid = 0i64;
    for e in 0..g.m() {
        varint(&mut b, zz(g.u[e] as i64 - g.v[e] as i64));
        varint(&mut b, g.v[e] as u64);
        varint(&mut b, zz(g.id[e] as i64 - pid));
        pid = g.id[e] as i64;
        varint(&mut b, g.r0[e] as u64);
        varint(&mut b, dict.binary_search_by_key(&cle(&g.attr[e]), cle).unwrap() as u64);
        varint(&mut b, g.long_mm[e] as u64 / 10);
        b.push(g.ouvrage[e].map_or(0, |o| g.ouvrages[o as usize].charge_t.min(255) as u8));
    }
    let brut_graphe = b.len();
    let z_graphe = mesure::zstd_octets(&b);
    let ordre = mesure::u32_octets(&topo.cch.order);
    let z_ordre = mesure::zstd_octets(&ordre);
    j.kv("fichier_graphe_brut", brut_graphe);
    j.kv("fichier_graphe_zstd", z_graphe);
    j.kv("fichier_ordre_brut", ordre.len());
    j.kv("fichier_ordre_zstd", z_ordre);
    j.kv("fichier_attributs_distincts", dict.len());
    // Chargement en jeu : décompression, décodage (approché par un passage
    // sur les octets), structure depuis l'ordre, personnalisation.
    let zb = zstd::bulk::compress(&b, 19).unwrap();
    let (_, t_dec) = ms(|| std::hint::black_box(zstd::bulk::decompress(&zb, brut_graphe).unwrap()));
    let graph = cch::graph::Graph { first_out: topo.first_out.clone(), head: topo.head.clone(), weight: vec![1; topo.head.len()] };
    let (cch2, t_build) = ms(|| cch::Cch::build(&graph, &topo.cch.order));
    let (_, t_cust) = ms(|| {
        for k in 0..PRINCIPALES {
            std::hint::black_box(cch2.customize(&topo.poids(&m.w[k].1)));
        }
    });
    j.kv("chargement_decompression_ms", format!("{t_dec:.1}"));
    j.kv("chargement_structure_depuis_ordre_ms", format!("{t_build:.1}"));
    j.kv("chargement_personnalisation_4_classes_ms", format!("{t_cust:.1}"));
    // Nouvelle traversée (pont du génie hors d'un pont existant) : une arête
    // de plus, même ordre, structure reconstruite.
    let mut fo = topo.first_out.clone();
    let mut hd = topo.head.clone();
    let (s, t) = (m.lieux_m[1][0], m.lieux_m[1][1]);
    hd.insert(fo[s as usize + 1] as usize, t);
    for x in fo.iter_mut().skip(s as usize + 1) {
        *x += 1;
    }
    let g2 = cch::graph::Graph { first_out: fo, head: hd.clone(), weight: vec![1; hd.len()] };
    let (c3, t_aj) = ms(|| cch::Cch::build(&g2, &topo.cch.order));
    j.kv("nouvelle_arete_structure_ms", format!("{t_aj:.1}"));
    j.kv("nouvelle_arete_arcs_cch_en_plus", c3.cch_arc_count() as i64 - topo.cch.cch_arc_count() as i64);
    ecrire(&a.sortie.join("fichier.csv"), &j.lignes);
}
