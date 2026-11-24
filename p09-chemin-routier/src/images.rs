//! Vues de dessus (PNG) : réseau en gris, itinéraires en couleur, ponts
//! détruits en noir. Pour la vérification à l'œil.

use crate::{Args, Monde, PRINCIPALES, charger, hier, Journal};
use std::io::BufWriter;

struct Toile {
    l: usize,
    h: usize,
    px: Vec<[u8; 3]>,
    x0: f64,
    y1: f64,
    ech: f64, // pixels par 1/16 m
}

impl Toile {
    fn new(b: [f64; 4], l: usize) -> Toile {
        let ech = l as f64 / (b[2] - b[0]);
        let h = ((b[3] - b[1]) * ech).ceil() as usize;
        Toile { l, h, px: vec![[255, 255, 255]; l * h], x0: b[0], y1: b[3], ech }
    }
    fn pt(&self, p: [i64; 2]) -> (f64, f64) {
        ((p[0] as f64 - self.x0) * self.ech, (self.y1 - p[1] as f64) * self.ech)
    }
    fn plot(&mut self, x: i64, y: i64, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as usize) < self.l && (y as usize) < self.h {
            self.px[y as usize * self.l + x as usize] = c;
        }
    }
    fn ligne(&mut self, a: [i64; 2], b: [i64; 2], c: [u8; 3], ep: i64) {
        let (ax, ay) = self.pt(a);
        let (bx, by) = self.pt(b);
        let n = ((bx - ax).abs().max((by - ay).abs()).ceil() as i64).max(1);
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let (x, y) = ((ax + t * (bx - ax)) as i64, (ay + t * (by - ay)) as i64);
            for dx in 0..ep {
                for dy in 0..ep {
                    self.plot(x + dx - ep / 2, y + dy - ep / 2, c);
                }
            }
        }
    }
    fn croix(&mut self, p: [i64; 2], c: [u8; 3]) {
        let (x, y) = self.pt(p);
        for d in -9..=9i64 {
            for e in -1..=1 {
                self.plot(x as i64 + d, y as i64 + d + e, c);
                self.plot(x as i64 + d, y as i64 - d + e, c);
            }
        }
    }
    fn ecrire(&self, f: &std::path::Path) {
        let w = BufWriter::new(std::fs::File::create(f).unwrap());
        let mut e = png::Encoder::new(w, self.l as u32, self.h as u32);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header().unwrap();
        let d: Vec<u8> = self.px.iter().flatten().copied().collect();
        w.write_image_data(&d).unwrap();
    }
}

fn reseau(t: &mut Toile, m: &Monde) {
    for (e, f) in m.g.frag.iter().enumerate() {
        let a = &m.g.attr[e];
        let c = match a.importance {
            1 | 2 => [120, 120, 120],
            3 | 4 => [165, 165, 165],
            5 => [200, 200, 200],
            _ => [228, 228, 228],
        };
        let pts = &m.fs[*f as usize].pts;
        for s in pts.windows(2) {
            t.ligne([s[0][0], s[0][1]], [s[1][0], s[1][1]], c, 1);
        }
    }
}

fn trace(t: &mut Toile, m: &Monde, arcs: &[u32], c: [u8; 3], ep: i64) {
    for &a in arcs {
        let pts = &m.fs[m.g.frag[(a / 2) as usize] as usize].pts;
        for s in pts.windows(2) {
            t.ligne([s[0][0], s[0][1]], [s[1][0], s[1][1]], c, ep);
        }
    }
}

fn bbox(m: &Monde, arcs: &[&[u32]], marge: f64) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for arcs in arcs {
        for &a in *arcs {
            for p in &m.fs[m.g.frag[(a / 2) as usize] as usize].pts {
                b[0] = b[0].min(p[0] as f64);
                b[1] = b[1].min(p[1] as f64);
                b[2] = b[2].max(p[0] as f64);
                b[3] = b[3].max(p[1] as f64);
            }
        }
    }
    let (dx, dy) = ((b[2] - b[0]) * marge, (b[3] - b[1]) * marge);
    [b[0] - dx, b[1] - dy, b[2] + dx, b[3] + dy]
}

const BLEU: [u8; 3] = [30, 90, 220];
const ROUGE: [u8; 3] = [220, 30, 30];
const ORANGE: [u8; 3] = [240, 150, 0];
const VERT: [u8; 3] = [0, 160, 60];
const NOIR: [u8; 3] = [0, 0, 0];

pub fn images(a: &Args) {
    let dir = a.sortie.join("images");
    std::fs::create_dir_all(&dir).unwrap();
    let mut j = Journal { lignes: String::new() };
    let m = charger(a, &mut j);
    let (topo, _, _) = hier::Topo::construire(&m.csr, &m.g.xy);
    let mts: Vec<cch::Metric> = m.w.iter().map(|(_, w)| topo.personnaliser(&topo.poids(w))).collect();
    let vue = topo.cch.view();
    let mut q = hier::RequeteCch::new(&vue);
    let lieu = |n: &str| m.lieux.iter().position(|l| l.0 == n).unwrap();
    let idx = |n: &str| m.w.iter().position(|x| x.0 == n).unwrap();
    for (k, tr) in a.trajets.split(',').filter(|x| !x.is_empty()).enumerate() {
        let (sa, sb) = tr.split_once('>').unwrap();
        let nom = tr.replace('>', "-");
        let mut r = Vec::new();
        for nm in ["camion", "porte-char", "leclerc", "voiture", "voiture-code"] {
            let i = idx(nm);
            let (s, t) = (m.lieux_m[i][lieu(sa)], m.lieux_m[i][lieu(sb)]);
            r.push(q.trajet(&mts[i], &m.csr, &m.w[i].1, s, t).map(|t| t.arcs).unwrap_or_default());
        }
        let (s, t) = (m.lieux_m[1][lieu(sa)], m.lieux_m[1][lieu(sb)]);
        // 1. Classes : camion (bleu), porte-char (orange), Leclerc (rouge).
        let b = bbox(&m, &[&r[0], &r[1], &r[2]], 0.08);
        let mut toile = Toile::new(b, 1600);
        reseau(&mut toile, &m);
        trace(&mut toile, &m, &r[1], ORANGE, 7);
        trace(&mut toile, &m, &r[2], ROUGE, 5);
        trace(&mut toile, &m, &r[0], BLEU, 3);
        toile.ecrire(&dir.join(format!("{}-{nom}-classes.png", a.etiquette)));
        // 2. Sens : code de la route (vert) contre chaussées séparées (bleu).
        if a.etiquette.contains("paris") {
            let b = bbox(&m, &[&r[3], &r[4]], 0.08);
            let mut toile = Toile::new(b, 1600);
            reseau(&mut toile, &m);
            trace(&mut toile, &m, &r[4], VERT, 5);
            trace(&mut toile, &m, &r[3], BLEU, 3);
            toile.ecrire(&dir.join(format!("{}-{nom}-sens.png", a.etiquette)));
        }
        // 3. Pont détruit : le pont le plus important du trajet du camion.
        if k < 3 {
            let pont = r[0]
                .iter()
                .filter_map(|&a| m.g.ouvrage[(a / 2) as usize].map(|o| (m.g.attr[(a / 2) as usize].importance, m.g.ouvrages[o as usize].pont)))
                .filter_map(|(i, p)| p.map(|p| (i, p)))
                .min();
            if let Some((_, pid)) = pont {
                let aretes: Vec<u32> = (0..m.g.m() as u32).filter(|&e| m.g.ouvrage[e as usize].is_some_and(|o| m.g.ouvrages[o as usize].pont == Some(pid))).collect();
                let mut w = m.w[1].1.clone();
                let mut en = topo.poids(&w);
                let mut mt = mts[1].clone();
                let mut pos = Vec::new();
                for &e in &aretes {
                    for d in 0..2 {
                        let arc = 2 * e as usize + d;
                        w[arc] = crate::classes::INF;
                        en[topo.pos_arc[arc] as usize] = crate::classes::INF;
                        pos.push(topo.pos_arc[arc]);
                    }
                }
                topo.personnaliser_partiel(&en, &mut mt, &pos);
                let apres = q.trajet(&mt, &m.csr, &w, s, t).map(|t| t.arcs).unwrap_or_default();
                let b = bbox(&m, &[&r[0], &apres], 0.08);
                let mut toile = Toile::new(b, 1600);
                reseau(&mut toile, &m);
                trace(&mut toile, &m, &r[0], BLEU, 7);
                trace(&mut toile, &m, &apres, VERT, 3);
                for &e in &aretes {
                    let p = &m.fs[m.g.frag[e as usize] as usize].pts[0];
                    toile.croix([p[0], p[1]], NOIR);
                }
                toile.ecrire(&dir.join(format!("{}-{nom}-pont-detruit.png", a.etiquette)));
                println!("{nom} : pont détruit {pid}");
            }
        }
    }
    // Ponts refusés au Leclerc (rouge) et au porte-char (orange) sur tout le réseau.
    {
        let b = {
            let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
            for p in &m.g.xy {
                b[0] = b[0].min(p[0] as f64);
                b[1] = b[1].min(p[1] as f64);
                b[2] = b[2].max(p[0] as f64);
                b[3] = b[3].max(p[1] as f64);
            }
            b
        };
        let mut toile = Toile::new(b, 1600);
        reseau(&mut toile, &m);
        for e in 0..m.g.m() {
            if let Some(o) = m.g.ouvrage[e] {
                let c = m.g.ouvrages[o as usize].charge_t;
                let col = if c < m.cls[3].masse_t {
                    Some(ROUGE)
                } else if c < m.cls[2].masse_t {
                    Some(ORANGE)
                } else {
                    None
                };
                if let Some(col) = col {
                    let p = m.fs[m.g.frag[e] as usize].pts[0];
                    let (x, y) = toile.pt([p[0], p[1]]);
                    for dx in -2..=2 {
                        for dy in -2..=2 {
                            toile.plot(x as i64 + dx, y as i64 + dy, col);
                        }
                    }
                }
            }
        }
        toile.ecrire(&dir.join(format!("{}-ponts-refuses.png", a.etiquette)));
    }
    let _ = PRINCIPALES;
}
