//! Tests sur graphes synthétiques, et micro-bancs (`#[ignore]`) :
//! `cargo test --release banc -- --ignored --nocapture`.

use crate::algos::{self, Borne, Csr, Etat};
use crate::graphe::{self, Graphe};
use crate::hier::Topo;
use crate::mesure::Alea;
use crate::tuiles::{Attr, Morceau, TileId};

/// Grille n × n, pas de 100 m, poids pseudo-aléatoires (graine fixe).
fn grille(n: usize, graine: u64) -> (Graphe, Vec<u32>) {
    let mut g = Graphe { xy: vec![], u: vec![], v: vec![], attr: vec![], long_mm: vec![], id: vec![], r0: vec![], ouvrage: vec![], ouvrages: vec![], frag: vec![] };
    for y in 0..n {
        for x in 0..n {
            g.xy.push([(x * 1600) as i64, (y * 1600) as i64]);
        }
    }
    let mut a = Alea(graine);
    let mut w = Vec::new();
    for y in 0..n {
        for x in 0..n {
            for (dx, dy) in [(1, 0), (0, 1)] {
                if x + dx < n && y + dy < n {
                    g.u.push((y * n + x) as u32);
                    g.v.push(((y + dy) * n + x + dx) as u32);
                    g.attr.push(Attr::default());
                    g.long_mm.push(100_000);
                    g.id.push(g.u.len() as u64);
                    g.r0.push(0);
                    g.ouvrage.push(None);
                    g.frag.push(0);
                    // Quelques sens uniques.
                    let t = 3600 + a.sous(4000) as u32;
                    let sens = a.sous(10);
                    w.push(if sens == 0 { crate::classes::INF } else { t });
                    w.push(if sens == 1 { crate::classes::INF } else { t });
                }
            }
        }
    }
    (g, w)
}

#[test]
fn methodes_exactes_sur_grille() {
    let (g, w) = grille(40, 1);
    let c = Csr::de(&g);
    let (topo, _, _) = Topo::construire(&c, &g.xy);
    let m = topo.personnaliser(&topo.poids(&w));
    let vue = topo.cch.view();
    let mut q = crate::hier::RequeteCch::new(&vue);
    let mut e = Etat::new(g.n());
    let mut a = Alea(5);
    for _ in 0..300 {
        let (s, t) = (a.sous(g.n()) as u32, a.sous(g.n()) as u32);
        let d = algos::astar(&c, &w, &mut e, s, t, &Borne::Zero).map(|t| t.duree);
        let b = algos::bidir(&c, &w, &mut e, s, t).map(|t| t.duree);
        let h = algos::astar(&c, &w, &mut e, s, t, &Borne::Euclide { xy: &g.xy, vmax: 100 }).map(|t| t.duree);
        let k = q.trajet(&m, &c, &w, s, t).map(|t| t.duree);
        assert_eq!(d, b);
        assert_eq!(d, h);
        assert_eq!(d, k);
    }
}

#[test]
fn personnalisation_partielle_identique() {
    let (g, mut w) = grille(30, 2);
    let c = Csr::de(&g);
    let (topo, _, _) = Topo::construire(&c, &g.xy);
    let mut en = topo.poids(&w);
    let mut m = topo.personnaliser(&en);
    let mut a = Alea(9);
    for tour in 0..20 {
        let mut pos = Vec::new();
        for _ in 0..(1 + tour * 3) {
            let arc = a.sous(w.len());
            // Coupure, réouverture ou changement de poids.
            w[arc] = match a.sous(3) {
                0 => crate::classes::INF,
                1 => 1000 + a.sous(9000) as u32,
                _ => 5000,
            };
            en[topo.pos_arc[arc] as usize] = w[arc];
            pos.push(topo.pos_arc[arc]);
        }
        topo.personnaliser_partiel(&en, &mut m, &pos);
        assert!(m == topo.personnaliser(&en), "tour {tour}");
    }
}

#[test]
fn recousure_independante_du_decoupage() {
    // Un tronçon coupé en 6 morceaux sur 3 tiles ; deux tronçons voisins.
    let mut ms = Vec::new();
    for (id, y) in [(7u64, 0i64), (8, 3200)] {
        for r in 0..6u16 {
            let x0 = r as i64 * 1000;
            ms.push(Morceau { tile: TileId { ix: r as i32 / 2, iy: 0 }, id, rang: r, attr: Attr::default(), pts: vec![[x0, y, 0], [x0 + 1000, y, 0]] });
        }
    }
    let (tout, st) = graphe::recoudre(graphe::de_morceaux(ms.clone()));
    assert_eq!(tout.len(), 2);
    assert_eq!(st.jointures, 10);
    let (a, b): (Vec<_>, Vec<_>) = ms.into_iter().partition(|m| m.tile.ix < 1);
    let (fa, _) = graphe::recoudre(graphe::de_morceaux(a));
    let (fb, _) = graphe::recoudre(graphe::de_morceaux(b));
    let (fab, _) = graphe::recoudre(fa.into_iter().chain(fb).collect());
    assert_eq!(graphe::empreinte_fragments(&fab), graphe::empreinte_fragments(&tout));
}

#[test]
#[ignore]
fn banc_personnalisation() {
    // Coût par arc : complète contre partielle, sur une grille de 200 × 200.
    let (g, mut w) = grille(200, 3);
    let c = Csr::de(&g);
    let (topo, t_o, t_s) = Topo::construire(&c, &g.xy);
    let mut en = topo.poids(&w);
    let pool1 = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let t = std::time::Instant::now();
    let mut m = pool1.install(|| topo.personnaliser(&en));
    let tc = t.elapsed().as_secs_f64() * 1e3;
    let arcs = topo.cch.cch_arc_count();
    eprintln!("banc grille noeuds {} arcs_cch {arcs} ordre_s {t_o:.2} structure_s {t_s:.3} complete_1t_ms {tc:.2} ns_par_arc {:.0}", g.n(), tc * 1e6 / arcs as f64);
    let mut a = Alea(4);
    let mut vu = crate::hier::Brouillon::new(&topo);
    for n in [1usize, 10, 100, 1000] {
        let mut pos = Vec::new();
        for _ in 0..n {
            let arc = a.sous(w.len());
            w[arc] = crate::classes::INF;
            en[topo.pos_arc[arc] as usize] = w[arc];
            pos.push(topo.pos_arc[arc]);
        }
        let t = std::time::Instant::now();
        let r = topo.personnaliser_partiel_avec(&en, &mut m, &pos, &mut vu);
        let tp = t.elapsed().as_secs_f64() * 1e3;
        eprintln!("banc partielle coupures {n} arcs {r} ms {tp:.3} ns_par_arc {:.0}", tp * 1e6 / r.max(1) as f64);
    }
}
