//! Simulation à 20 Hz selon le repère : déterminisme, coût, mémoire, dérive,
//! taille de sauvegarde.
//!
//! - **A** : ECEF f64 (position 3D, cap tangent), retour sur la sphère, tile
//!   retrouvée par projection à chaque tick.
//! - **B** : grille projetée du territoire en f64 (Lambert-93 ou UTM).
//! - **C** : tile entière + position locale f32 centrée sur la tile (±500 m).
//!
//! Même marche pour les trois : cap tourné d'un petit angle selon un tirage
//! haché (graine, tick, identifiant) et selon la case de 2 m d'un champ par
//! tile, pas de 1,4 m/s / 20.

use bevy::ecs::batching::BatchingStrategy;
use bevy::math::{DVec2, DVec3};
use bevy::prelude::*;

use crate::geo::{self, TileId};
use crate::stats::{melange, Fnv};

pub const PAS: f32 = 1.4 / 20.0;
const COS: f32 = 0.999_847_7; // cos(1°)
const SIN: f32 = 0.017_452_406; // sin(1°)
pub const CHAMP: usize = 500;

#[derive(Resource)]
pub struct Champ(pub Vec<u8>);

impl Champ {
    pub fn nouveau() -> Champ {
        Champ((0..CHAMP * CHAMP).map(|i| (melange(i as u64) & 3) as u8).collect())
    }
}

#[derive(Resource, Clone, Copy)]
pub struct Tick(pub u64);

#[derive(Resource, Clone, Copy)]
pub struct Lot(pub usize);

#[derive(Component, Clone, Copy)]
pub struct MarcheurC {
    pub id: u32,
    pub tx: i32,
    pub ty: i32,
    pub pos: Vec2,
    pub dir: Vec2,
}

#[derive(Component, Clone, Copy)]
pub struct MarcheurB {
    pub id: u32,
    pub pos: DVec2,
    pub dir: Vec2,
}

#[derive(Component, Clone, Copy)]
pub struct MarcheurA {
    pub id: u32,
    pub pos: DVec3,
    pub dir: DVec3,
}

#[inline]
fn tourner(d: Vec2, gauche: bool) -> Vec2 {
    let s = if gauche { SIN } else { -SIN };
    let r = Vec2::new(d.x * COS - d.y * s, d.x * s + d.y * COS);
    // Renormalisation sans trigonométrie (racine carrée IEEE exacte).
    r / r.length()
}

#[inline]
fn decision(id: u32, tick: u64, case: u8) -> bool {
    let h = melange(((id as u64) << 32) ^ tick);
    ((h & 1) as u8 ^ (case & 1)) == 1
}

/// Pas de C : passage de tile exact des deux côtés (Sterbenz, coordonnées centrées).
#[inline]
pub fn pas_c(m: &mut MarcheurC, champ: &[u8], tick: u64) {
    let cx = (((m.pos.x + 500.0) * 0.5) as usize).min(CHAMP - 1);
    let cy = (((m.pos.y + 500.0) * 0.5) as usize).min(CHAMP - 1);
    m.dir = tourner(m.dir, decision(m.id, tick, champ[cy * CHAMP + cx]));
    m.pos += m.dir * PAS;
    if m.pos.x >= 500.0 {
        m.pos.x -= 1000.0;
        m.tx += 1;
    } else if m.pos.x < -500.0 {
        m.pos.x += 1000.0;
        m.tx -= 1;
    }
    if m.pos.y >= 500.0 {
        m.pos.y -= 1000.0;
        m.ty += 1;
    } else if m.pos.y < -500.0 {
        m.pos.y += 1000.0;
        m.ty -= 1;
    }
}

#[inline]
pub fn pas_b(m: &mut MarcheurB, champ: &[u8], tick: u64) {
    // Troncature : coordonnées positives (Lambert-93, UTM), pas de `floor`.
    let tx = (m.pos.x * 0.001) as i64;
    let ty = (m.pos.y * 0.001) as i64;
    let cx = (((m.pos.x - tx as f64 * 1000.0) * 0.5) as usize).min(CHAMP - 1);
    let cy = (((m.pos.y - ty as f64 * 1000.0) * 0.5) as usize).min(CHAMP - 1);
    m.dir = tourner(m.dir, decision(m.id, tick, champ[cy * CHAMP + cx]));
    m.pos += m.dir.as_dvec2() * PAS as f64;
}

#[inline]
pub fn pas_b_floor(m: &mut MarcheurB, champ: &[u8], tick: u64) {
    let tx = (m.pos.x * 0.001).floor();
    let ty = (m.pos.y * 0.001).floor();
    let cx = (((m.pos.x - tx * 1000.0) * 0.5).floor() as usize).min(CHAMP - 1);
    let cy = (((m.pos.y - ty * 1000.0) * 0.5).floor() as usize).min(CHAMP - 1);
    m.dir = tourner(m.dir, decision(m.id, tick, champ[cy * CHAMP + cx]));
    m.pos += m.dir.as_dvec2() * PAS as f64;
}

/// Pas de A : il faut la tile et la case, donc projeter (Lambert-93) à chaque tick.
#[inline]
pub fn pas_a(m: &mut MarcheurA, champ: &[u8], tick: u64, proj: &geo::Projection) {
    let n = m.pos.normalize();
    let g = geo::Geo { lat: n.z.asin().to_degrees(), lon: n.y.atan2(n.x).to_degrees(), h: 0.0 };
    let p = proj.avant(g);
    let tx = (p.x * 0.001) as i64;
    let ty = (p.y * 0.001) as i64;
    let cx = (((p.x - tx as f64 * 1000.0) * 0.5) as usize).min(CHAMP - 1);
    let cy = (((p.y - ty as f64 * 1000.0) * 0.5) as usize).min(CHAMP - 1);
    let gauche = decision(m.id, tick, champ[cy * CHAMP + cx]);
    let s = if gauche { SIN as f64 } else { -SIN as f64 };
    // Rotation du cap autour de la verticale (Rodrigues).
    let d = m.dir * COS as f64 + n.cross(m.dir) * s;
    m.pos += d * PAS as f64;
    let r = 6_371_008.8;
    m.pos *= r / m.pos.length();
    let n2 = m.pos / r;
    m.dir = (d - n2 * d.dot(n2)).normalize();
}

fn systeme_c(champ: Res<Champ>, mut tick: ResMut<Tick>, lot: Res<Lot>, mut q: Query<&mut MarcheurC>) {
    let t = tick.0;
    q.par_iter_mut().batching_strategy(BatchingStrategy::new().min_batch_size(lot.0)).for_each(|mut m| pas_c(&mut m, &champ.0, t));
    tick.0 += 1;
}

fn systeme_b(champ: Res<Champ>, mut tick: ResMut<Tick>, lot: Res<Lot>, mut q: Query<&mut MarcheurB>) {
    let t = tick.0;
    q.par_iter_mut().batching_strategy(BatchingStrategy::new().min_batch_size(lot.0)).for_each(|mut m| pas_b(&mut m, &champ.0, t));
    tick.0 += 1;
}

#[derive(Resource)]
pub struct ProjA(pub geo::Projection);

fn systeme_a(champ: Res<Champ>, mut tick: ResMut<Tick>, lot: Res<Lot>, proj: Res<ProjA>, mut q: Query<&mut MarcheurA>) {
    let t = tick.0;
    q.par_iter_mut().batching_strategy(BatchingStrategy::new().min_batch_size(lot.0)).for_each(|mut m| pas_a(&mut m, &champ.0, t, &proj.0));
    tick.0 += 1;
}

/// Départ : n marcheurs dans les 10 × 10 tiles autour du lieu.
pub fn depart(n: usize, lieu: &str) -> (Vec<MarcheurC>, Vec<MarcheurB>, Vec<MarcheurA>) {
    let l = geo::lieu(lieu);
    let z = geo::zone_par_epsg(l.epsg);
    let g = z.proj.avant(l.g);
    let t0 = TileId::de_grille(l.epsg, g);
    let (mut c, mut b, mut a) = (vec![], vec![], vec![]);
    for i in 0..n {
        let r = melange(i as u64 ^ 0x5eed);
        let (dx, dy) = ((r % 10) as i32 - 5, ((r >> 8) % 10) as i32 - 5);
        let lx = ((r >> 16) % 100_000) as f32 / 100.0 - 500.0;
        let ly = ((r >> 36) % 100_000) as f32 / 100.0 - 500.0;
        let ang = ((r >> 56) % 64) as f32 / 64.0 * std::f32::consts::TAU;
        let dir = Vec2::new(ang.cos(), ang.sin());
        let id = i as u32;
        c.push(MarcheurC { id, tx: t0.ix + dx, ty: t0.iy + dy, pos: Vec2::new(lx, ly), dir });
        let pg = DVec2::new((t0.ix + dx) as f64 * 1000.0 + 500.0 + lx as f64, (t0.iy + dy) as f64 * 1000.0 + 500.0 + ly as f64);
        b.push(MarcheurB { id, pos: pg, dir });
        let pe = z.proj.grille_vers_ecef(pg, 0.0).normalize() * 6_371_008.8;
        let gg = z.proj.inverse(pg);
        let rr = geo::bevy_depuis_ecef(gg).transpose();
        let d3 = (rr.col(0) * dir.x as f64 - rr.col(2) * dir.y as f64).normalize();
        a.push(MarcheurA { id, pos: pe, dir: d3 });
    }
    (c, b, a)
}

pub fn somme_c(ms: &[MarcheurC], t0: Option<(i32, i32)>) -> u64 {
    let mut v: Vec<_> = ms.to_vec();
    v.sort_by_key(|m| m.id);
    let mut h = Fnv::default();
    for m in v {
        let (ox, oy) = t0.unwrap_or((0, 0));
        h.u32((m.tx - ox) as u32);
        h.u32((m.ty - oy) as u32);
        h.u32(m.pos.x.to_bits());
        h.u32(m.pos.y.to_bits());
        h.u32(m.dir.x.to_bits());
        h.u32(m.dir.y.to_bits());
    }
    h.0
}

pub fn somme_b(ms: &[MarcheurB], o: DVec2) -> u64 {
    let mut v: Vec<_> = ms.to_vec();
    v.sort_by_key(|m| m.id);
    let mut h = Fnv::default();
    for m in v {
        h.u64((m.pos.x - o.x).to_bits());
        h.u64((m.pos.y - o.y).to_bits());
    }
    h.0
}

pub fn somme_a(ms: &[MarcheurA]) -> u64 {
    let mut v: Vec<_> = ms.to_vec();
    v.sort_by_key(|m| m.id);
    let mut h = Fnv::default();
    for m in v {
        h.u64(m.pos.x.to_bits());
        h.u64(m.pos.y.to_bits());
        h.u64(m.pos.z.to_bits());
    }
    h.0
}

pub struct Resultat {
    pub somme: u64,
    pub ticks_ms: Vec<f64>,
    pub octets_position: usize,
}

/// Lance la simulation dans un `App` Bevy (exécuteur multi-thread, requêtes
/// parallèles avec taille de lot minimale). `rendu` ajoute les systèmes de
/// rendu relatif à la caméra (scène du soldat à ce lieu, recalage à chaque
/// frame) dans le même `App`.
pub fn lancer(repr: char, n: usize, ticks: u64, threads: usize, lot: usize, lieu: &str, rendu: Option<&str>) -> Resultat {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(TaskPoolPlugin { task_pool_options: TaskPoolOptions::with_num_threads(threads) }));
    app.add_plugins(TransformPlugin);
    app.insert_resource(Champ::nouveau()).insert_resource(Tick(0)).insert_resource(Lot(lot));
    if let Some(l) = rendu {
        crate::monde::installer(
            &mut app,
            &crate::monde::Options {
                strat: crate::monde::Strategie::ChaqueFrame,
                scenario: crate::monde::Scenario::Soldat { lieu: l.into(), distance: 25.0 },
                t0: 0.0,
                acceleration: 1.0,
                lot,
                fenetre: false,
            },
        );
        app.world_mut().resource_mut::<crate::monde::Releve>().actif = true;
    }
    let (c, b, a) = depart(n, lieu);
    let l = geo::lieu(lieu);
    let octets = match repr {
        'c' => {
            app.world_mut().spawn_batch(c);
            app.add_systems(Update, systeme_c);
            std::mem::size_of::<Vec2>()
        }
        'b' => {
            app.world_mut().spawn_batch(b);
            app.add_systems(Update, systeme_b);
            std::mem::size_of::<DVec2>()
        }
        _ => {
            app.world_mut().spawn_batch(a);
            app.insert_resource(ProjA(geo::zone_par_epsg(l.epsg).proj));
            app.add_systems(Update, systeme_a);
            std::mem::size_of::<DVec3>()
        }
    };
    app.finish();
    app.cleanup();
    let mut ticks_ms = vec![];
    for _ in 0..ticks {
        let t = std::time::Instant::now();
        app.update();
        ticks_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let w = app.world_mut();
    let somme = match repr {
        'c' => {
            let v: Vec<MarcheurC> = w.query::<&MarcheurC>().iter(w).copied().collect();
            let z = geo::zone_par_epsg(l.epsg);
            let t0 = TileId::de_grille(l.epsg, z.proj.avant(l.g));
            somme_c(&v, Some((t0.ix, t0.iy)))
        }
        'b' => {
            let v: Vec<MarcheurB> = w.query::<&MarcheurB>().iter(w).copied().collect();
            let z = geo::zone_par_epsg(l.epsg);
            let t0 = TileId::de_grille(l.epsg, z.proj.avant(l.g));
            somme_b(&v, t0.coin())
        }
        _ => {
            let v: Vec<MarcheurA> = w.query::<&MarcheurA>().iter(w).copied().collect();
            somme_a(&v)
        }
    };
    Resultat { somme, ticks_ms, octets_position: octets }
}

// ------------------------------------------------------------------ Dérive

/// Marche en ligne droite vers l'est sur `dist` mètres : écart final avec le
/// calcul exact (n pas de la même longueur f32), pour C (centré et coin) et B.
pub fn derive(vitesse: f32, dist: f64) -> [(String, f64, f64); 3] {
    let pas = vitesse / 20.0;
    let n = (dist / pas as f64) as u64;
    let exact = n as f64 * pas as f64;
    // C centré.
    let (mut x, mut t) = (-500.0f32 + 0.123, 0i64);
    for _ in 0..n {
        x += pas;
        if x >= 500.0 {
            x -= 1000.0;
            t += 1;
        }
    }
    let c = t as f64 * 1000.0 + x as f64 - (-500.0f64 + 0.123f32 as f64);
    // C depuis le coin.
    let (mut x2, mut t2) = (0.123f32, 0i64);
    for _ in 0..n {
        x2 += pas;
        if x2 >= 1000.0 {
            x2 -= 1000.0;
            t2 += 1;
        }
    }
    let c2 = t2 as f64 * 1000.0 + x2 as f64 - 0.123f32 as f64;
    // B : Lambert-93 de Brest, f64.
    let x0 = 145_000.123f64;
    let mut xb = x0;
    for _ in 0..n {
        xb += pas as f64;
    }
    let b = xb - x0;
    [
        ("c-centre".into(), c - exact, (c - exact) / exact * 100.0),
        ("c-coin".into(), c2 - exact, (c2 - exact) / exact * 100.0),
        ("b-f64".into(), b - exact, (b - exact) / exact * 100.0),
    ]
}

// ------------------------------------------------------------------ Sauvegarde

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn zz(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Taille par position (octets bruts, zstd) pour une tile dense.
pub fn sauvegarde(n: usize) -> Vec<(String, f64, f64)> {
    let (mut c, mut b, _) = depart(n * 100, "Brest");
    // Positions après 600 ticks de marche (mantisses quelconques).
    let champ = Champ::nouveau();
    for k in 0..600 {
        for m in c.iter_mut() {
            pas_c(m, &champ.0, k);
        }
        for m in b.iter_mut() {
            pas_b(m, &champ.0, k);
        }
    }
    // Une seule tile : celle du premier marcheur.
    let (tx, ty) = (c[0].tx, c[0].ty);
    let cs: Vec<MarcheurC> = c.iter().copied().filter(|m| m.tx == tx && m.ty == ty).take(n).collect();
    let ids: std::collections::HashSet<u32> = cs.iter().map(|m| m.id).collect();
    let bs: Vec<MarcheurB> = b.iter().copied().filter(|m| ids.contains(&m.id)).collect();
    let l = geo::lieu("Brest");
    let z = geo::zone_par_epsg(l.epsg);
    let es: Vec<DVec3> = bs.iter().map(|m| z.proj.grille_vers_ecef(m.pos, 31.0)).collect();
    let nn = cs.len() as f64;
    let zst = |v: &[u8]| zstd::bulk::compress(v, 19).unwrap().len() as f64 / nn;
    let mut res = vec![];

    // Clés quantifiées triées, écarts en varint.
    let trie = |mut cles: Vec<Vec<i64>>| -> Vec<u8> {
        cles.sort();
        let mut out = vec![];
        let mut prec = vec![0i64; cles[0].len()];
        for k in cles {
            for (i, v) in k.iter().enumerate() {
                varint(&mut out, zz(v - prec[i]));
            }
            prec = k;
        }
        out
    };
    // C : u16 au 1/64 m depuis le coin (A-13).
    let q: Vec<Vec<i64>> = cs.iter().map(|m| vec![((m.pos.y + 500.0) * 64.0) as i64, ((m.pos.x + 500.0) * 64.0) as i64]).collect();
    let mut brut = vec![];
    for k in &q {
        brut.extend_from_slice(&(k[1] as u16).to_le_bytes());
        brut.extend_from_slice(&(k[0] as u16).to_le_bytes());
    }
    res.push(("c-u16-1/64m".into(), brut.len() as f64 / nn, zst(&trie(q.clone()))));
    // C : f32 exacts (bits).
    let mut brut = vec![];
    for m in &cs {
        brut.extend_from_slice(&m.pos.x.to_bits().to_le_bytes());
        brut.extend_from_slice(&m.pos.y.to_bits().to_le_bytes());
    }
    let qb: Vec<Vec<i64>> = cs.iter().map(|m| vec![m.pos.y.to_bits() as i64, m.pos.x.to_bits() as i64]).collect();
    res.push(("c-f32-exact".into(), brut.len() as f64 / nn, zst(&trie(qb)).min(zst(&brut))));
    // B : grille absolue au 1/64 m (i32) et f64 exacts.
    let qa: Vec<Vec<i64>> = bs.iter().map(|m| vec![(m.pos.y * 64.0) as i64, (m.pos.x * 64.0) as i64]).collect();
    res.push(("b-i32-1/64m".into(), 8.0, zst(&trie(qa))));
    let mut brut = vec![];
    for m in &bs {
        brut.extend_from_slice(&m.pos.x.to_bits().to_le_bytes());
        brut.extend_from_slice(&m.pos.y.to_bits().to_le_bytes());
    }
    let qf: Vec<Vec<i64>> = bs.iter().map(|m| vec![m.pos.y.to_bits() as i64, m.pos.x.to_bits() as i64]).collect();
    res.push(("b-f64-exact".into(), 16.0, zst(&trie(qf)).min(zst(&brut))));
    // A : ECEF au 1/64 m (3 × i32) et f64 exacts.
    let qe: Vec<Vec<i64>> = es.iter().map(|p| vec![(p.z * 64.0) as i64, (p.y * 64.0) as i64, (p.x * 64.0) as i64]).collect();
    res.push(("a-i32-1/64m".into(), 12.0, zst(&trie(qe))));
    let mut brut = vec![];
    for p in &es {
        for v in [p.x, p.y, p.z] {
            brut.extend_from_slice(&v.to_bits().to_le_bytes());
        }
    }
    res.push(("a-f64-exact".into(), 24.0, zst(&brut)));
    res
}
