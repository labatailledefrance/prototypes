//! Méthodes hiérarchiques.
//!
//! - **CCH** (contraction hierarchies personnalisables) : ordre par
//!   dissection emboîtée (inertial flow, crate `cch`, portage de RoutingKit),
//!   structure construite une fois pour la topologie ; chaque classe de
//!   véhicule est une **métrique** (poids des arcs) qui se « personnalise »
//!   sur cette structure. Analogie : on calcule une fois la topologie d'un
//!   réseau (OSPF : la base d'états de liens), puis on ne recalcule que les
//!   coûts quand un lien change.
//! - **Personnalisation partielle** (écrite ici, absente de la crate) : après
//!   la modification de quelques arcs, seuls les raccourcis qui en dépendent
//!   sont recalculés, dans l'ordre des rangs. Résultat identique, bit à bit,
//!   à une personnalisation complète (vérifié à chaque scénario).
//! - **CH** classiques (crate `fast_paths`) : un prétraitement complet par
//!   métrique ; une modification impose de le refaire (au mieux avec l'ordre
//!   déjà calculé).

use crate::algos::{Csr, Trajet};
use crate::classes::INF;
use cch::{Cch, Metric};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

const INVALIDE: u32 = u32::MAX;

/// Topologie CCH du graphe, avec la correspondance arc ↔ arc d'entrée.
pub struct Topo {
    pub cch: Cch,
    /// Position de chaque arc (2e, 2e + 1) dans le CSR d'entrée de la crate.
    pub pos_arc: Vec<u32>,
    /// Arc correspondant à chaque position du CSR d'entrée.
    pub arc_de_pos: Vec<u32>,
    pub first_out: Vec<u32>,
    pub head: Vec<u32>,
}

impl Topo {
    /// Graphe d'entrée de la crate : CSR par queue, arcs dans l'ordre de leur
    /// identifiant. Coordonnées (km, f32) pour l'ordre géométrique.
    pub fn construire(c: &Csr, xy: &[[i64; 2]]) -> (Topo, f64, f64) {
        let n = c.n;
        let mut first_out = vec![0u32; n + 1];
        let mut head = Vec::with_capacity(c.tete.len());
        let mut arc_de_pos = Vec::with_capacity(c.tete.len());
        let mut pos_arc = vec![0u32; c.tete.len()];
        for x in 0..n {
            let mut arcs: Vec<u32> = c.out_arc[c.out_deb[x] as usize..c.out_deb[x + 1] as usize].to_vec();
            arcs.sort_unstable();
            for a in arcs {
                pos_arc[a as usize] = head.len() as u32;
                arc_de_pos.push(a);
                head.push(c.tete[a as usize]);
            }
            first_out[x + 1] = head.len() as u32;
        }
        let tail: Vec<u32> = (0..n as u32).flat_map(|x| std::iter::repeat_n(x, (first_out[x as usize + 1] - first_out[x as usize]) as usize)).collect();
        let lat: Vec<f32> = xy.iter().map(|p| (p[1] as f64 / 16000.0) as f32).collect();
        let lon: Vec<f32> = xy.iter().map(|p| (p[0] as f64 / 16000.0) as f32).collect();
        let t = std::time::Instant::now();
        let order = cch::inertial_order(n as u32, &tail, &head, &lat, &lon);
        let t_ordre = t.elapsed().as_secs_f64();
        let graph = cch::graph::Graph { first_out: first_out.clone(), head: head.clone(), weight: vec![1; head.len()] };
        let t = std::time::Instant::now();
        let cch = Cch::build(&graph, &order);
        let t_struct = t.elapsed().as_secs_f64();
        (Topo { cch, pos_arc, arc_de_pos, first_out, head }, t_ordre, t_struct)
    }

    /// Poids dans l'ordre du CSR d'entrée.
    pub fn poids(&self, w: &[u32]) -> Vec<u32> {
        self.arc_de_pos.iter().map(|&a| w[a as usize]).collect()
    }

    pub fn personnaliser(&self, w_entree: &[u32]) -> Metric {
        self.cch.customize(w_entree)
    }

    pub fn octets_structure(&self) -> usize {
        let c = &self.cch;
        4 * (c.rank.len()
            + c.order.len()
            + c.elimination_tree_parent.len()
            + c.up_first_out.len()
            + c.up_head.len()
            + c.up_tail.len()
            + c.down_first_out.len()
            + c.down_head.len()
            + c.down_to_up.len()
            + c.input_arc_to_cch_arc.len()
            + c.forward_input_arc_of_cch.len()
            + c.backward_input_arc_of_cch.len()
            + c.first_extra_forward_input_arc_of_cch.len()
            + c.extra_forward_input_arc_of_cch.len()
            + c.first_extra_backward_input_arc_of_cch.len()
            + c.extra_backward_input_arc_of_cch.len())
    }

    /// Hauteur de l'arbre d'élimination (profondeur maximale) et moyenne de
    /// la profondeur (espace de recherche d'une requête).
    pub fn profondeurs(&self) -> (u32, f64) {
        let c = &self.cch;
        let n = c.node_count();
        let mut prof = vec![0u32; n];
        // Les parents ont un rang plus grand : on remonte du sommet.
        for x in (0..n).rev() {
            let p = c.elimination_tree_parent[x];
            if p != INVALIDE {
                prof[x] = prof[p as usize] + 1;
            }
        }
        let max = prof.iter().copied().max().unwrap_or(0);
        let moy = prof.iter().map(|&p| p as f64 + 1.0).sum::<f64>() / n.max(1) as f64;
        (max, moy)
    }

    /// Valeur d'un arc CCH à partir de ses arcs d'entrée (sans triangles).
    fn base(&self, w: &[u32], a: usize) -> (u32, u32) {
        let c = &self.cch;
        let mut f = INF;
        let mut b = INF;
        let fi = c.forward_input_arc_of_cch[a];
        if fi != INVALIDE {
            f = w[fi as usize];
        }
        let bi = c.backward_input_arc_of_cch[a];
        if bi != INVALIDE {
            b = w[bi as usize];
        }
        for j in c.first_extra_forward_input_arc_of_cch[a]..c.first_extra_forward_input_arc_of_cch[a + 1] {
            f = f.min(w[c.extra_forward_input_arc_of_cch[j as usize] as usize]);
        }
        for j in c.first_extra_backward_input_arc_of_cch[a]..c.first_extra_backward_input_arc_of_cch[a + 1] {
            b = b.min(w[c.extra_backward_input_arc_of_cch[j as usize] as usize]);
        }
        (f, b)
    }

    /// Arc montant x → z (x < z en rang), s'il existe.
    fn arc(&self, x: u32, z: u32) -> Option<usize> {
        let c = &self.cch;
        let (d, f) = (c.up_first_out[x as usize] as usize, c.up_first_out[x as usize + 1] as usize);
        c.up_head[d..f].binary_search(&z).ok().map(|k| d + k)
    }

    /// Personnalisation partielle : `w` contient déjà les nouveaux poids
    /// (ordre d'entrée) ; `changes` les positions modifiées. Met `m` à jour et
    /// rend le nombre d'arcs CCH recalculés.
    pub fn personnaliser_partiel(&self, w: &[u32], m: &mut Metric, changes: &[u32]) -> usize {
        let mut b = Brouillon::new(self);
        self.personnaliser_partiel_avec(w, m, changes, &mut b)
    }

    /// Idem avec un brouillon réutilisé. Travail **par nœud**, comme la
    /// personnalisation complète : un nœud x marqué recalcule tous ses arcs
    /// montants (base, puis triangles inférieurs y < x) ; si l'un change, les
    /// nœuds dont un arc montant a ce côté dans un triangle sont marqués. Les
    /// nœuds sont traités par rang croissant : un arc ne dépend que d'arcs
    /// dont la queue a un rang plus petit.
    pub fn personnaliser_partiel_avec(&self, w: &[u32], m: &mut Metric, changes: &[u32], br: &mut Brouillon) -> usize {
        let c = &self.cch;
        let mut tas: BinaryHeap<Reverse<u32>> = BinaryHeap::new();
        let mut marquer = |x: u32, br: &mut Brouillon, tas: &mut BinaryHeap<Reverse<u32>>| {
            if !br.marque[x as usize] {
                br.marque[x as usize] = true;
                tas.push(Reverse(x));
            }
        };
        for &p in changes {
            let a = c.input_arc_to_cch_arc[p as usize];
            if a != INVALIDE {
                marquer(c.up_tail[a as usize], br, &mut tas);
            }
        }
        let mut recalc = 0usize;
        let mut ancien: Vec<(u32, u32)> = Vec::new();
        while let Some(Reverse(x)) = tas.pop() {
            br.marque[x as usize] = false;
            let (d, f) = (c.up_first_out[x as usize] as usize, c.up_first_out[x as usize + 1] as usize);
            recalc += f - d;
            ancien.clear();
            for a in d..f {
                ancien.push((m.forward[a], m.backward[a]));
                let (bf, bb) = self.base(w, a);
                m.forward[a] = bf;
                m.backward[a] = bb;
                br.cache[c.up_head[a] as usize] = a as u32;
            }
            for k in c.down_first_out[x as usize]..c.down_first_out[x as usize + 1] {
                let bas = c.down_to_up[k as usize] as usize; // y → x
                let y = c.down_head[k as usize] as usize;
                let y0 = c.up_first_out[y];
                let mut cur = c.up_first_out[y + 1];
                while cur > y0 {
                    cur -= 1;
                    let mil = cur as usize; // y → z
                    let z = c.up_head[mil];
                    if z <= x {
                        break;
                    }
                    let haut = br.cache[z as usize] as usize; // x → z
                    let fc = m.backward[bas].wrapping_add(m.forward[mil]);
                    let bc = m.forward[bas].wrapping_add(m.backward[mil]);
                    if fc < m.forward[haut] {
                        m.forward[haut] = fc;
                    }
                    if bc < m.backward[haut] {
                        m.backward[haut] = bc;
                    }
                }
            }
            // Arcs changés de x : z ∈ Zc. Triangles de sommet bas x dont un
            // côté inférieur a changé : côté haut (min(z, u), max(z, u)).
            let mut zmax = None;
            for a in d..f {
                if (m.forward[a], m.backward[a]) != ancien[a - d] {
                    let z = c.up_head[a];
                    zmax = Some(z);
                    // z est la queue d'un côté haut s'il existe u > z.
                    if a + 1 < f {
                        marquer(z, br, &mut tas);
                    }
                }
            }
            if let Some(zm) = zmax {
                for a in d..f {
                    let u = c.up_head[a];
                    if u >= zm {
                        break;
                    }
                    marquer(u, br, &mut tas);
                }
            }
        }
        recalc
    }
}

/// Tableaux de travail de la personnalisation partielle (un par thread).
pub struct Brouillon {
    marque: Vec<bool>,
    cache: Vec<u32>,
}

impl Brouillon {
    pub fn new(t: &Topo) -> Brouillon {
        let n = t.cch.node_count();
        Brouillon { marque: vec![false; n], cache: vec![0; n] }
    }
}

/// Requête CCH réutilisable : chemin en nœuds (crate), puis en arcs du
/// graphe (arc de poids minimal entre deux nœuds consécutifs, égalité par
/// identifiant d'arc).
pub struct RequeteCch<'a> {
    vue: &'a cch::CchView<'a>,
    pq: cch::PathQuery<'a>,
}

impl<'a> RequeteCch<'a> {
    pub fn new(vue: &'a cch::CchView<'a>) -> Self {
        RequeteCch { vue, pq: cch::PathQuery::new(vue) }
    }
    pub fn trajet(&mut self, m: &Metric, csr: &Csr, w: &[u32], s: u32, t: u32) -> Option<Trajet> {
        let noeuds = self.pq.path(&m.view(), s, t)?;
        let mut arcs = Vec::with_capacity(noeuds.len());
        let mut duree = 0u32;
        for p in noeuds.windows(2) {
            let (a, b) = (p[0], p[1]);
            let mut best = (INF, u32::MAX);
            for k in csr.out_deb[a as usize]..csr.out_deb[a as usize + 1] {
                let arc = csr.out_arc[k as usize];
                if csr.tete[arc as usize] == b && (w[arc as usize], arc) < best {
                    best = (w[arc as usize], arc);
                }
            }
            debug_assert!(best.0 < INF);
            duree += best.0;
            arcs.push(best.1);
        }
        // Espace de recherche : ancêtres de s et de t dans l'arbre.
        let c = self.vue;
        let mut explores = 0u32;
        for x0 in [s, t] {
            let mut x = c.rank[x0 as usize];
            while x != INVALIDE {
                explores += 1;
                x = c.elimination_tree_parent[x as usize];
            }
        }
        Some(Trajet { duree, arcs, explores })
    }
}

/// CH classiques (fast_paths) pour une métrique.
pub fn ch_preparer(c: &Csr, w: &[u32], ordre: Option<&[usize]>) -> fast_paths::FastGraph {
    let mut ig = fast_paths::InputGraph::new();
    for a in 0..w.len() {
        if w[a] < INF && c.queue[a] != c.tete[a] {
            ig.add_edge(c.queue[a] as usize, c.tete[a] as usize, w[a] as usize);
        }
    }
    // Un arc fictif nul sur le dernier nœud garantit le nombre de nœuds.
    let n = c.n;
    if n >= 2 {
        ig.add_edge(n - 1, n - 2, (INF as usize) * 4);
    }
    ig.freeze();
    match ordre {
        Some(o) => fast_paths::prepare_with_order(&ig, o).unwrap(),
        None => fast_paths::prepare(&ig),
    }
}
