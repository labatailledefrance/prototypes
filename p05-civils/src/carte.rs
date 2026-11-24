//! Carte du monde : un bloc de tiles réelles adjacentes, résident en mémoire.
//!
//! Ce qui ne dépend que de la carte vient de la chaîne de données (B-01) :
//! le bâti à 1 m (P-01) et, nouveauté de P-04, la **zone de rues
//! accessible**, calculée une fois pour tout le bloc (`zones_globales`).
//! S'y ajoutent les **constructions** (barricades et leur état), petites et
//! résidentes : une tile inactive est affichée avec ses constructions.
//!
//! Analogie : la carte est une image de base en lecture seule, montée sur
//! tous les nœuds ; les constructions sont une petite couche modifiable.

use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

use crate::alea::Alea;
use crate::champ::Fin;
use crate::ville::{COTE, Ville};

pub const COTE_I: i32 = COTE as i32;
/// Mots de 64 bits d'un bitmap de tile (1 bit par m²).
pub const MOTS: usize = (COTE * COTE).div_ceil(64);
/// Densité de référence : 20 000 zombies pour la tile la plus bâtie de P-01.
pub const COUVERTURE_REF: f32 = 0.6134;

/// Géométrie du bloc : `w × h` tiles, coin sud-ouest en (x0, y0) km
/// Lambert-93.
#[derive(Clone, Copy, Debug)]
pub struct Geo {
    pub w: usize,
    pub h: usize,
    pub x0: i64,
    pub y0: i64,
}

impl Geo {
    pub fn n(&self) -> usize {
        self.w * self.h
    }
    pub fn ixy(&self, t: usize) -> (i32, i32) {
        ((t % self.w) as i32, (t / self.w) as i32)
    }
    pub fn tile(&self, ix: i32, iy: i32) -> Option<usize> {
        (ix >= 0 && iy >= 0 && (ix as usize) < self.w && (iy as usize) < self.h)
            .then(|| iy as usize * self.w + ix as usize)
    }
    pub fn voisine(&self, t: usize, dx: i32, dy: i32) -> Option<usize> {
        let (x, y) = self.ixy(t);
        self.tile(x + dx, y + dy)
    }
    pub fn km(&self, t: usize) -> (i64, i64) {
        let (x, y) = self.ixy(t);
        (self.x0 + x as i64, self.y0 + y as i64)
    }
    /// Origine (coin sud-ouest) de la tile, en mètres Lambert-93.
    pub fn origine(&self, t: usize) -> [f64; 2] {
        let (x, y) = self.km(t);
        [x as f64 * 1000.0, y as f64 * 1000.0]
    }
    /// Tile qui contient le point (mètres Lambert-93).
    pub fn tile_de(&self, p: [f64; 2]) -> Option<usize> {
        let ix = (p[0] / 1000.0).floor() as i64 - self.x0;
        let iy = (p[1] / 1000.0).floor() as i64 - self.y0;
        self.tile(ix as i32, iy as i32)
    }
    /// Distance de Tchebychev entre deux tiles (en tiles).
    pub fn cheb(&self, a: usize, b: usize) -> i32 {
        let (ax, ay) = self.ixy(a);
        let (bx, by) = self.ixy(b);
        (ax - bx).abs().max((ay - by).abs())
    }
}

/// Une barricade : une case de 1 m, avec ses points de vie. Les points de
/// vie sont atomiques : les zombies les entament en parallèle pendant le
/// déplacement (somme commutative, donc résultat déterministe).
pub struct Barricade {
    pub x: u16,
    pub y: u16,
    pub pv: AtomicI32,
}

pub struct TuileCarte {
    pub ville: Ville,
    /// Cases de 1 m de la zone de rues accessible (bits).
    pub zone: Vec<u64>,
    pub couverture: f32,
    /// Zombies au début de la partie (compteur d'une tile jamais activée).
    pub population: u32,
    pub barricades: Vec<Barricade>,
    /// Case de 1 m (y × 1000 + x) → rang dans `barricades`.
    pub index_barricades: HashMap<u32, u32>,
}

#[derive(bevy_ecs::prelude::Resource)]
pub struct Carte {
    pub geo: Geo,
    pub tuiles: Vec<TuileCarte>,
}

#[inline(always)]
pub fn bit(b: &[u64], k: usize) -> bool {
    (b[k >> 6] >> (k & 63)) & 1 == 1
}
#[inline(always)]
pub fn poser_bit(b: &mut [u64], k: usize) {
    b[k >> 6] |= 1 << (k & 63);
}

const VOISINS8: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

/// Statistiques du précalcul des zones (chaîne de données).
#[derive(Default, Debug, Clone)]
pub struct StatZones {
    pub ms: f64,
    pub composantes: usize,
    pub cases_principale: usize,
    pub cases_libres: usize,
}

impl Carte {
    /// Bloc de tiles réelles de P-01. `zones` : dossier des zones
    /// précalculées (sinon, calculées ici pour tout le bloc).
    pub fn reelle(dossier: &Path, geo: Geo, zones: Option<&Path>, densite: f32) -> std::io::Result<(Carte, StatZones)> {
        let mut villes = Vec::with_capacity(geo.n());
        for t in 0..geo.n() {
            let (x, y) = geo.km(t);
            villes.push(Ville::charger(&dossier.join(format!("x{x}_y{y}.bin")))?);
        }
        let (zones, stat) = match zones {
            Some(d) => {
                let mut z = Vec::with_capacity(geo.n());
                for t in 0..geo.n() {
                    let (x, y) = geo.km(t);
                    z.push(lire_zone(&d.join(format!("x{x}_y{y}.zone")))?);
                }
                (z, StatZones::default())
            }
            None => zones_globales(&villes, geo),
        };
        Ok((Self::assembler(geo, villes, zones, densite), stat))
    }

    /// Bloc synthétique (tests) : villes générées, bordées d'une rue de
    /// 12 m pour que les tiles communiquent.
    pub fn synthetique(geo: Geo, zombies_ref: u32) -> Carte {
        let mut villes = Vec::with_capacity(geo.n());
        for t in 0..geo.n() {
            let mut v = Ville::generer(crate::alea::hacher(0xC17E, t as u64));
            v.degager_bords(6);
            villes.push(v);
        }
        let (zones, _) = zones_globales(&villes, geo);
        let mut c = Self::assembler(geo, villes, zones, 1.0);
        for t in &mut c.tuiles {
            t.population = zombies_ref;
        }
        c
    }

    fn assembler(geo: Geo, villes: Vec<Ville>, zones: Vec<Vec<u64>>, densite: f32) -> Carte {
        let tuiles = villes
            .into_iter()
            .zip(zones)
            .map(|(ville, zone)| {
                let couverture = ville.couverture;
                TuileCarte {
                    population: (20_000.0 * densite * couverture / COUVERTURE_REF).round() as u32,
                    ville,
                    zone,
                    couverture,
                    barricades: vec![],
                    index_barricades: HashMap::new(),
                }
            })
            .collect();
        Carte { geo, tuiles }
    }

    /// Case de 1 m bâtie ? Coordonnées locales à la tile `t`, éventuellement
    /// hors de la tile (lues dans la voisine). Hors du bloc : bâti.
    #[inline(always)]
    pub fn bati(&self, t: usize, x: i32, y: i32) -> bool {
        if (x as u32) < COTE as u32 && (y as u32) < COTE as u32 {
            return self.tuiles[t].ville.bati(x, y);
        }
        let (dx, dy) = (x.div_euclid(COTE_I), y.div_euclid(COTE_I));
        match self.geo.voisine(t, dx, dy) {
            Some(v) => self.tuiles[v].ville.bati(x - dx * COTE_I, y - dy * COTE_I),
            None => true,
        }
    }

    #[inline(always)]
    pub fn bati_p(&self, t: usize, p: [f32; 2]) -> bool {
        self.bati(t, p[0].floor() as i32, p[1].floor() as i32)
    }

    pub fn fin(&self, t: usize, x: i32, y: i32) -> Fin {
        let (dx, dy) = (x.div_euclid(COTE_I), y.div_euclid(COTE_I));
        match self.geo.voisine(t, dx, dy) {
            Some(v) => crate::champ::fin_de(&self.tuiles[v].ville, x - dx * COTE_I, y - dy * COTE_I),
            None => Fin::Bati,
        }
    }

    /// Case de 1 m dans la zone accessible (locale à `t`, ou voisine) ?
    #[inline(always)]
    pub fn dans_zone(&self, t: usize, x: i32, y: i32) -> bool {
        let (dx, dy) = (x.div_euclid(COTE_I), y.div_euclid(COTE_I));
        match self.geo.voisine(t, dx, dy) {
            Some(v) => {
                let (lx, ly) = ((x - dx * COTE_I) as usize, (y - dy * COTE_I) as usize);
                bit(&self.tuiles[v].zone, ly * COTE + lx)
            }
            None => false,
        }
    }

    /// Point (mètres Lambert-93) libre et accessible ?
    pub fn accessible_monde(&self, p: [f64; 2]) -> bool {
        let Some(t) = self.geo.tile_de(p) else { return false };
        let o = self.geo.origine(t);
        let (x, y) = ((p[0] - o[0]).floor() as i32, (p[1] - o[1]).floor() as i32);
        !self.tuiles[t].ville.bati(x, y) && bit(&self.tuiles[t].zone, y as usize * COTE + x as usize)
    }

    /// Barricade de la case de 1 m (locale à `t`, ou voisine) : (tile, rang).
    #[inline(always)]
    pub fn barricade(&self, t: usize, x: i32, y: i32) -> Option<(usize, u32)> {
        let (dx, dy) = (x.div_euclid(COTE_I), y.div_euclid(COTE_I));
        let v = self.geo.voisine(t, dx, dy)?;
        let tv = &self.tuiles[v];
        if tv.barricades.is_empty() {
            return None;
        }
        let (lx, ly) = (x - dx * COTE_I, y - dy * COTE_I);
        if !tv.ville.barricade(lx, ly) {
            return None;
        }
        tv.index_barricades.get(&(ly as u32 * COTE as u32 + lx as u32)).map(|&i| (v, i))
    }

    /// Pose des barricades (cases de 1 m locales à `t`).
    pub fn poser_barricades(&mut self, t: usize, cases: &[(u16, u16)], pv: i32) {
        let tu = &mut self.tuiles[t];
        for &(x, y) in cases {
            if tu.ville.bati(x as i32, y as i32) {
                continue;
            }
            tu.ville.poser_barricade(x as usize, y as usize);
            tu.index_barricades.insert(y as u32 * COTE as u32 + x as u32, tu.barricades.len() as u32);
            tu.barricades.push(Barricade { x, y, pv: AtomicI32::new(pv) });
        }
    }

    /// Détruit les barricades à 0 point de vie ; renvoie leurs cases.
    pub fn detruire_barricades(&mut self, t: usize) -> Vec<(u16, u16)> {
        let tu = &mut self.tuiles[t];
        let mut d = vec![];
        for b in &tu.barricades {
            if b.pv.load(Relaxed) <= 0 && tu.ville.barricade(b.x as i32, b.y as i32) {
                tu.ville.detruire_barricade(b.x as usize, b.y as usize);
                d.push((b.x, b.y));
            }
        }
        d
    }

    /// État des constructions de la tile : (x, y, points de vie restants ;
    /// 0 = détruite).
    pub fn constructions(&self, t: usize) -> Vec<(u16, u16, i32)> {
        self.tuiles[t]
            .barricades
            .iter()
            .map(|b| (b.x, b.y, b.pv.load(Relaxed).max(0)))
            .collect()
    }

    /// Cases de 1 m libres et accessibles du rectangle local
    /// [x0, x1[ × [y0, y1[ de la tile `t` (y × 1000 + x) ; le rectangle est
    /// élargi tant qu'il n'en contient aucune. Énumération plutôt que tirage
    /// par rejet : un coin de 20 × 20 m souvent bâti rendait le rejet
    /// 1 000 fois plus lent (P-04, migration).
    pub fn cases_libres(&self, t: usize, mut r: [f32; 4]) -> Vec<u32> {
        let tu = &self.tuiles[t];
        for _ in 0..8 {
            let mut v = vec![];
            for y in r[2].max(0.0) as usize..(r[3].min(1000.0) as usize) {
                for x in r[0].max(0.0) as usize..(r[1].min(1000.0) as usize) {
                    if bit(&tu.zone, y * COTE + x) && !tu.ville.bati(x as i32, y as i32) {
                        v.push((y * COTE + x) as u32);
                    }
                }
            }
            if !v.is_empty() {
                return v;
            }
            let (cx, cy) = ((r[0] + r[1]) / 2.0, (r[2] + r[3]) / 2.0);
            let (lx, ly) = (r[1] - r[0], r[3] - r[2]);
            r = [(cx - lx).max(0.0), (cx + lx).min(1000.0), (cy - ly).max(0.0), (cy + ly).min(1000.0)];
        }
        vec![500 * COTE as u32 + 500]
    }

    /// Point de la zone accessible tiré dans le rectangle local
    /// [x0, x1[ × [y0, y1[ de la tile `t`, de façon déterministe à partir de
    /// la graine ; élargit le rectangle si aucun tirage ne réussit.
    #[allow(dead_code)]
    pub fn tirer_dans(&self, t: usize, graine: u64, mut r: [f32; 4]) -> [f32; 2] {
        let tu = &self.tuiles[t];
        let mut a = Alea::new(graine);
        for _ in 0..8 {
            for _ in 0..4096 {
                let p = [r[0] + a.f32() * (r[1] - r[0]), r[2] + a.f32() * (r[3] - r[2])];
                let (x, y) = (p[0] as usize, p[1] as usize);
                if x < COTE && y < COTE && bit(&tu.zone, y * COTE + x) && !tu.ville.bati(x as i32, y as i32) {
                    return p;
                }
            }
            let (cx, cy) = ((r[0] + r[1]) / 2.0, (r[2] + r[3]) / 2.0);
            let (lx, ly) = (r[1] - r[0], r[3] - r[2]);
            r = [(cx - lx).max(0.0), (cx + lx).min(1000.0), (cy - ly).max(0.0), (cy + ly).min(1000.0)];
        }
        // Tile sans zone accessible (n'arrive pas sur les tiles de Paris).
        [500.0, 500.0]
    }
}

/// Zone accessible **globale** : plus grande composante connexe des cases de
/// 1 m libres de tout le bloc (8 voisins, pas de coin de mur coupé, comme
/// les champs). Une cour fermée dans une tile mais reliée à la rue par une
/// voisine est accessible ; seule une vue d'ensemble le sait. C'est le
/// travail de la chaîne de données.
pub fn zones_globales(villes: &[Ville], geo: Geo) -> (Vec<Vec<u64>>, StatZones) {
    let t0 = std::time::Instant::now();
    let (wm, hm) = (geo.w * COTE, geo.h * COTE);
    let bati = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x as usize >= wm || y as usize >= hm {
            return true;
        }
        let (tx, ty) = (x as usize / COTE, y as usize / COTE);
        villes[ty * geo.w + tx].bati((x as usize % COTE) as i32, (y as usize % COTE) as i32)
    };
    let mut etiq = vec![u32::MAX; wm * hm];
    let mut file = VecDeque::new();
    let (mut n, mut meilleure, mut taille_max, mut libres) = (0u32, u32::MAX, 0usize, 0usize);
    for depart in 0..wm * hm {
        let (x0, y0) = ((depart % wm) as i32, (depart / wm) as i32);
        if etiq[depart] != u32::MAX || bati(x0, y0) {
            continue;
        }
        etiq[depart] = n;
        file.push_back((x0, y0));
        let mut taille = 0usize;
        while let Some((x, y)) = file.pop_front() {
            taille += 1;
            for (dx, dy) in VOISINS8 {
                let (nx, ny) = (x + dx, y + dy);
                if bati(nx, ny) || (dx != 0 && dy != 0 && (bati(x + dx, y) || bati(x, y + dy))) {
                    continue;
                }
                let k = ny as usize * wm + nx as usize;
                if etiq[k] == u32::MAX {
                    etiq[k] = n;
                    file.push_back((nx, ny));
                }
            }
        }
        libres += taille;
        if taille > taille_max {
            (meilleure, taille_max) = (n, taille);
        }
        n += 1;
    }
    let mut zones = vec![vec![0u64; MOTS]; geo.n()];
    for y in 0..hm {
        for x in 0..wm {
            if etiq[y * wm + x] == meilleure {
                let t = (y / COTE) * geo.w + x / COTE;
                poser_bit(&mut zones[t], (y % COTE) * COTE + x % COTE);
            }
        }
    }
    let stat = StatZones {
        ms: t0.elapsed().as_secs_f64() * 1e3,
        composantes: n as usize,
        cases_principale: taille_max,
        cases_libres: libres,
    };
    (zones, stat)
}

/// Zone accessible **locale** (P-03) : plus grande composante de la tile
/// seule, par un champ exact à 1 m depuis le centre accessible. C'est le
/// calcul que ferait le jeu sans précalcul.
pub fn zone_locale(v: &Ville) -> Vec<u64> {
    let centre = crate::sim::centre_accessible(v, [500.0, 500.0]);
    let (t1, c1) = crate::sim::champ_1m(v, &[centre]);
    crate::sim::zone_de(&t1, &c1)
}

/// Écrit une zone compressée (LZ4) : format de la chaîne de données.
pub fn ecrire_zone(chemin: &Path, zone: &[u64]) -> std::io::Result<usize> {
    let brut: Vec<u8> = zone.iter().flat_map(|w| w.to_le_bytes()).collect();
    let c = lz4_flex::compress_prepend_size(&brut);
    std::fs::write(chemin, &c)?;
    Ok(c.len())
}

pub fn lire_zone(chemin: &Path) -> std::io::Result<Vec<u64>> {
    let c = std::fs::read(chemin)?;
    decoder_zone(&c)
}

pub fn decoder_zone(c: &[u8]) -> std::io::Result<Vec<u64>> {
    let brut = lz4_flex::decompress_size_prepended(c).map_err(|e| std::io::Error::other(e.to_string()))?;
    if brut.len() != MOTS * 8 {
        return Err(std::io::Error::other("zone de taille inattendue"));
    }
    Ok(brut.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect())
}

// ------------------------------------------------------------- Chemins

/// Chemin à pied (cases de 2 m entièrement libres et accessibles) entre deux
/// points du bloc, en mètres Lambert-93 ; pour déplacer les humains en
/// colonne. Dijkstra (coûts 5 / 7) sur le rectangle englobant élargi de 1 km.
pub fn chemin(c: &Carte, a: [f64; 2], b: [f64; 2]) -> Vec<[f64; 2]> {
    let g = c.geo;
    let (ox, oy) = (g.x0 as f64 * 1000.0, g.y0 as f64 * 1000.0);
    let (wm, hm) = ((g.w * COTE / 2) as i32, (g.h * COTE / 2) as i32);
    let cell = |p: [f64; 2]| (((p[0] - ox) / 2.0) as i32, ((p[1] - oy) / 2.0) as i32);
    let (ca, cb) = (cell(a), cell(b));
    let marge = 500;
    let (bx0, by0) = ((ca.0.min(cb.0) - marge).max(0), (ca.1.min(cb.1) - marge).max(0));
    let (bx1, by1) = ((ca.0.max(cb.0) + marge).min(wm - 1), (ca.1.max(cb.1) + marge).min(hm - 1));
    let (bw, bh) = ((bx1 - bx0 + 1) as usize, (by1 - by0 + 1) as usize);
    // Franchissabilité de chaque case de 2 m du rectangle, calculée une fois.
    let pass: Vec<bool> = (0..bw * bh)
        .map(|k| {
            let (cx, cy) = (bx0 + (k % bw) as i32, by0 + (k / bw) as i32);
            (0..2).all(|dy| {
                (0..2).all(|dx| {
                    let p = [ox + (cx * 2 + dx) as f64 + 0.5, oy + (cy * 2 + dy) as f64 + 0.5];
                    c.accessible_monde(p)
                })
            })
        })
        .collect();
    let libre = |cx: i32, cy: i32| -> bool {
        cx >= bx0 && cy >= by0 && cx <= bx1 && cy <= by1 && pass[(cy - by0) as usize * bw + (cx - bx0) as usize]
    };
    // Case libre la plus proche d'un point (spirale).
    let proche = |p: (i32, i32)| -> (i32, i32) {
        for r in 0..200i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dy.abs()) == r && libre(p.0 + dx, p.1 + dy) {
                        return (p.0 + dx, p.1 + dy);
                    }
                }
            }
        }
        panic!("aucune case libre près de {p:?}");
    };
    let (ca, cb) = (proche(ca), proche(cb));
    let idx = |x: i32, y: i32| (y - by0) as usize * bw + (x - bx0) as usize;
    let mut d = vec![u32::MAX; bw * bh];
    let mut tas = BinaryHeap::new();
    d[idx(cb.0, cb.1)] = 0;
    tas.push(std::cmp::Reverse((0u32, cb.0, cb.1)));
    while let Some(std::cmp::Reverse((du, x, y))) = tas.pop() {
        if du != d[idx(x, y)] {
            continue;
        }
        if (x, y) == ca {
            break;
        }
        for (dx, dy) in VOISINS8 {
            let (nx, ny) = (x + dx, y + dy);
            if !libre(nx, ny) || (dx != 0 && dy != 0 && (!libre(x + dx, y) || !libre(x, y + dy))) {
                continue;
            }
            let nd = du + if dx != 0 && dy != 0 { 7 } else { 5 };
            let k = idx(nx, ny);
            if nd < d[k] {
                d[k] = nd;
                tas.push(std::cmp::Reverse((nd, nx, ny)));
            }
        }
    }
    assert!(d[idx(ca.0, ca.1)] != u32::MAX, "pas de chemin entre {a:?} et {b:?}");
    let mut ch = vec![];
    let mut p = ca;
    loop {
        ch.push([ox + p.0 as f64 * 2.0 + 1.0, oy + p.1 as f64 * 2.0 + 1.0]);
        if p == cb {
            break;
        }
        let mut best = (d[idx(p.0, p.1)], p);
        for (dx, dy) in VOISINS8 {
            let (nx, ny) = (p.0 + dx, p.1 + dy);
            if nx < bx0 || ny < by0 || nx > bx1 || ny > by1 {
                continue;
            }
            if dx != 0 && dy != 0 && (!libre(p.0 + dx, p.1) || !libre(p.0, p.1 + dy)) {
                continue;
            }
            let v = d[idx(nx, ny)];
            if v < best.0 {
                best = (v, (nx, ny));
            }
        }
        assert!(best.1 != p, "descente bloquée");
        p = best.1;
    }
    ch
}
