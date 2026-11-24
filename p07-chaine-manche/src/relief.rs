//! Relief : RGE ALTI (ASC) lu **en flux** dans l'archive 7z de l'IGN, sans
//! la décompresser sur disque, et converti en grilles de nœuds par tile.
//!
//! - Les dalles ASC donnent l'altitude au **centre** des pixels, sur les
//!   multiples du pas (1 ou 5 m), bord ouest et bord nord de la dalle inclus.
//!   Les valeurs ont deux décimales : elles sont lues en **centimètres
//!   entiers**, sans passer par un flottant (déterminisme exact).
//! - Une tile stocke ses **nœuds** aux multiples de `r` m, bords compris :
//!   (1000/r + 1)² nœuds. Les nœuds du bord est et du bord sud viennent des
//!   dalles voisines : deux tiles voisines ont exactement les mêmes valeurs
//!   sur leur bord commun, donc aucune marche (contrôle `marches`).
//! - 2 m et 5 m sont **décimés** du 1 m (un nœud sur r), pas moyennés : la
//!   valeur d'un nœud ne dépend que de sa position globale.
//! - Encodage : quantification (pas de `q` cm), prédiction MED (LOCO-I, celle
//!   de JPEG-LS), résidus en zigzag sur deux plans d'octets, puis zstd. Les
//!   nœuds sans donnée (mer) sont un bitmap à part ; leur valeur est la
//!   prédiction, pour ne rien coûter.
//!
//! Analogie : la grille de nœuds est un journal de séquences partagé entre
//! deux répliques ; chaque nœud de bord a une seule source de vérité.

use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Mutex;

pub const SANS: i32 = i32::MIN;

/// Une dalle décodée : altitudes en cm, ligne 0 au nord.
pub struct Dalle {
    /// Centre du pixel nord-ouest (m, entiers).
    pub x_no: i64,
    pub y_no: i64,
    pub pas: i64,
    pub n: usize,
    pub cm: Vec<i32>,
}

impl Dalle {
    #[inline]
    pub fn get(&self, x: i64, y: i64) -> Option<i32> {
        let (dx, dy) = (x - self.x_no, self.y_no - y);
        if dx < 0 || dy < 0 || dx % self.pas != 0 || dy % self.pas != 0 {
            return None;
        }
        let (i, j) = ((dx / self.pas) as usize, (dy / self.pas) as usize);
        if i >= self.n || j >= self.n {
            return None;
        }
        Some(self.cm[j * self.n + i])
    }
}

/// Analyse d'un fichier ASC (en-tête ESRI puis valeurs).
pub fn lire_asc(t: &[u8]) -> Dalle {
    let mut p = 0usize;
    let mut entete: HashMap<String, f64> = HashMap::new();
    for _ in 0..6 {
        while t[p].is_ascii_whitespace() {
            p += 1;
        }
        let d = p;
        while !t[p].is_ascii_whitespace() {
            p += 1;
        }
        let cle = String::from_utf8_lossy(&t[d..p]).to_ascii_lowercase();
        while t[p] == b' ' || t[p] == b'\t' {
            p += 1;
        }
        let d = p;
        while !t[p].is_ascii_whitespace() {
            p += 1;
        }
        entete.insert(cle, String::from_utf8_lossy(&t[d..p]).parse().unwrap());
    }
    let n = entete["ncols"] as usize;
    assert_eq!(n, entete["nrows"] as usize);
    let pas = entete["cellsize"];
    let nodata = (entete["nodata_value"] * 100.0).round() as i64;
    let x_no = (entete["xllcorner"] + pas / 2.0).round() as i64;
    let y_no = (entete["yllcorner"] + pas / 2.0 + (n as f64 - 1.0) * pas).round() as i64;
    let mut cm = Vec::with_capacity(n * n);
    let len = t.len();
    while cm.len() < n * n {
        while p < len && t[p].is_ascii_whitespace() {
            p += 1;
        }
        let neg = t[p] == b'-';
        if neg {
            p += 1;
        }
        let mut ent: i64 = 0;
        while p < len && t[p].is_ascii_digit() {
            ent = ent * 10 + (t[p] - b'0') as i64;
            p += 1;
        }
        let mut frac: i64 = 0;
        let mut nd = 0;
        if p < len && t[p] == b'.' {
            p += 1;
            while p < len && t[p].is_ascii_digit() {
                if nd < 3 {
                    frac = frac * 10 + (t[p] - b'0') as i64;
                }
                nd += 1;
                p += 1;
            }
        }
        // Centimètres : deux décimales exactes ; une troisième est arrondie.
        let v = match nd.min(3) {
            0 => ent * 100,
            1 => ent * 100 + frac * 10,
            2 => ent * 100 + frac,
            _ => ent * 100 + (frac + 5) / 10,
        };
        let v = if neg { -v } else { v };
        cm.push(if v == nodata { SANS } else { v as i32 });
    }
    Dalle { x_no, y_no, pas: pas.round() as i64, n, cm }
}

/// Parcourt toutes les dalles ASC d'une archive 7z, en parallèle par bloc
/// (chaque bloc LZMA2 est un flux indépendant), et appelle `f` sur chacune.
/// Renvoie (nombre de dalles, octets décompressés).
pub fn parcourir_7z(chemin: &Path, f: impl Fn(Dalle) + Sync) -> (usize, u64) {
    use sevenz_rust2::{Archive, BlockDecoder, Password};
    let mut fic = File::open(chemin).expect("archive RGE ALTI");
    let archive = Archive::read(&mut fic, &Password::empty()).expect("lecture de l'archive");
    let nb = archive.blocks.len();
    let compte = Mutex::new((0usize, 0u64));
    (0..nb).into_par_iter().for_each(|b| {
        let mut fic = File::open(chemin).unwrap();
        let mdp = Password::empty();
        let dec = BlockDecoder::new(1, b, &archive, &mdp, &mut fic);
        let mut tampon: Vec<u8> = Vec::new();
        dec.for_each_entries(&mut |e, r| {
            if e.name().ends_with(".asc") {
                tampon.clear();
                r.read_to_end(&mut tampon)?;
                let n = tampon.len() as u64;
                f(lire_asc(&tampon));
                let mut c = compte.lock().unwrap();
                c.0 += 1;
                c.1 += n;
            } else {
                std::io::copy(r, &mut std::io::sink())?;
            }
            Ok(true)
        })
        .expect("décompression d'un bloc");
    });
    compte.into_inner().unwrap()
}

// ------------------------------------------------------------ Grilles de nœuds

/// Grille de nœuds d'une tile au pas `r` : (n+1)² valeurs en cm (ou SANS),
/// ligne j = 0 au **sud** (y = y0), comme les autres couches.
#[derive(Clone)]
pub struct Noeuds {
    pub r: usize,
    pub m: usize, // n + 1
    pub cm: Vec<i32>,
}

impl Noeuds {
    pub fn new(r: usize) -> Noeuds {
        let m = 1000 / r + 1;
        Noeuds { r, m, cm: vec![SANS; m * m] }
    }
    #[inline]
    pub fn get(&self, i: usize, j: usize) -> i32 {
        self.cm[j * self.m + i]
    }
}

/// Ce qu'une dalle de 1 km laisse pour finir les tiles voisines : sa colonne
/// ouest (x = x0, pour la tile ouest) et sa ligne nord (y = y0 + 1000, pour
/// la tile nord), au pas r.
pub struct Bords {
    pub ouest: Vec<i32>, // j = 0..=n, valeurs y0 + j·r (j = 0 inconnu : SANS)
    pub nord: Vec<i32>,  // i = 0..=n
}

/// Nœuds intérieurs d'une tile depuis sa dalle de 1 m (bords est et sud
/// manquants), et ses bords à exporter.
pub fn noeuds_de_dalle(d: &Dalle, x0: i64, y0: i64, r: usize) -> (Noeuds, Bords) {
    let mut g = Noeuds::new(r);
    let m = g.m;
    let ri = r as i64;
    for j in 0..m {
        for i in 0..m {
            if let Some(v) = d.get(x0 + i as i64 * ri, y0 + j as i64 * ri) {
                g.cm[j * m + i] = v;
            }
        }
    }
    let ouest = (0..m).map(|j| g.cm[j * m]).collect();
    let nord = (0..m).map(|i| g.cm[(m - 1) * m + i]).collect();
    (g, Bords { ouest, nord })
}

/// Complète les bords est et sud d'une grille avec les bords exportés des
/// voisines : est (ix+1, iy) → colonne ouest ; sud (ix, iy-1) → ligne nord ;
/// coin sud-est (ix+1, iy-1) → premier nœud de sa ligne nord.
pub fn completer(g: &mut Noeuds, est: Option<&Bords>, sud: Option<&Bords>, sud_est: Option<&Bords>) {
    let m = g.m;
    if let Some(b) = est {
        for j in 1..m {
            g.cm[j * m + m - 1] = b.ouest[j];
        }
    }
    if let Some(b) = sud {
        for i in 0..m - 1 {
            g.cm[i] = b.nord[i];
        }
    }
    if let Some(b) = sud_est {
        g.cm[m - 1] = b.nord[0];
    }
}

/// Encode une grille : quantification au pas `q` cm, MED, zigzag, deux plans.
/// Renvoie (octets bruts avant zstd, bitmap des nœuds sans donnée).
pub fn encoder(g: &Noeuds, q: i32) -> Vec<u8> {
    let m = g.m;
    let qv: Vec<i32> = g.cm.iter().map(|&v| if v == SANS { SANS } else { div_arrondi(v, q) }).collect();
    let mut sans = vec![0u8; (m * m).div_ceil(8)];
    let mut rec = vec![0i32; m * m];
    let mut bas = Vec::with_capacity(m * m);
    let mut haut = Vec::with_capacity(m * m);
    let mut esc: Vec<u8> = Vec::new();
    for j in 0..m {
        for i in 0..m {
            let k = j * m + i;
            let a = if i > 0 { rec[k - 1] } else if j > 0 { rec[k - m] } else { 0 };
            let b = if j > 0 { rec[k - m] } else { a };
            let c = if i > 0 && j > 0 { rec[k - m - 1] } else { b };
            let pred = med(a, b, c);
            let v = qv[k];
            let v = if v == SANS {
                sans[k >> 3] |= 1 << (k & 7);
                pred
            } else {
                v
            };
            rec[k] = v;
            let res = v.wrapping_sub(pred);
            let z = ((res << 1) ^ (res >> 31)) as u32;
            if z < 0xFFFF {
                bas.push(z as u8);
                haut.push((z >> 8) as u8);
            } else {
                bas.push(0xFF);
                haut.push(0xFF);
                esc.extend_from_slice(&z.to_le_bytes());
            }
        }
    }
    let mut out = Vec::with_capacity(16 + sans.len() + 2 * m * m + esc.len());
    out.extend_from_slice(&(m as u16).to_le_bytes());
    out.extend_from_slice(&(q as u16).to_le_bytes());
    out.extend_from_slice(&(esc.len() as u32).to_le_bytes());
    let tous_sans = sans.iter().all(|&b| b == 0xFF) || g.cm.iter().all(|&v| v == SANS);
    out.push(tous_sans as u8);
    if !tous_sans {
        out.extend_from_slice(&sans);
        out.extend_from_slice(&bas);
        out.extend_from_slice(&haut);
        out.extend_from_slice(&esc);
    }
    out
}

/// Décode une grille encodée : altitudes quantifiées (× q cm), SANS hors
/// données.
pub fn decoder(b: &[u8], r: usize) -> Noeuds {
    let m = u16::from_le_bytes([b[0], b[1]]) as usize;
    let q = u16::from_le_bytes([b[2], b[3]]) as i32;
    let ne = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let mut g = Noeuds { r, m, cm: vec![SANS; m * m] };
    if b[8] == 1 {
        return g;
    }
    let ns = (m * m).div_ceil(8);
    let sans = &b[9..9 + ns];
    let bas = &b[9 + ns..9 + ns + m * m];
    let haut = &b[9 + ns + m * m..9 + ns + 2 * m * m];
    let esc = &b[9 + ns + 2 * m * m..9 + ns + 2 * m * m + ne];
    let mut ke = 0;
    let mut rec = vec![0i32; m * m];
    for j in 0..m {
        for i in 0..m {
            let k = j * m + i;
            let a = if i > 0 { rec[k - 1] } else if j > 0 { rec[k - m] } else { 0 };
            let bb = if j > 0 { rec[k - m] } else { a };
            let c = if i > 0 && j > 0 { rec[k - m - 1] } else { bb };
            let pred = med(a, bb, c);
            let mut z = bas[k] as u32 | (haut[k] as u32) << 8;
            if z == 0xFFFF {
                z = u32::from_le_bytes(esc[ke..ke + 4].try_into().unwrap());
                ke += 4;
            }
            let res = ((z >> 1) as i32) ^ -((z & 1) as i32);
            let v = pred.wrapping_add(res);
            rec[k] = v;
            if sans[k >> 3] >> (k & 7) & 1 == 0 {
                g.cm[k] = v * q;
            }
        }
    }
    g
}

#[inline]
fn med(a: i32, b: i32, c: i32) -> i32 {
    if c >= a.max(b) {
        a.min(b)
    } else if c <= a.min(b) {
        a.max(b)
    } else {
        a + b - c
    }
}

#[inline]
fn div_arrondi(v: i32, q: i32) -> i32 {
    if q == 1 {
        return v;
    }
    if v >= 0 { (v + q / 2) / q } else { -((-v + q / 2) / q) }
}

/// Erreur d'une grille au pas r (interpolation bilinéaire) par rapport aux
/// nœuds de 1 m de la même tile : (somme des carrés en cm², max en cm,
/// nombre de points). Seuls les points dont les quatre nœuds existent.
pub fn erreur(fine: &Noeuds, g: &Noeuds) -> (f64, i64, u64, Vec<u32>) {
    let r = g.r;
    let (mut s2, mut mx, mut n) = (0f64, 0i64, 0u64);
    let mut hist = vec![0u32; 64]; // par tranches de 5 cm
    for y in 0..1000usize {
        for x in 0..1000usize {
            let v = fine.get(x, y);
            if v == SANS {
                continue;
            }
            let (i, j) = (x / r, y / r);
            let (fx, fy) = ((x % r) as f64 / r as f64, (y % r) as f64 / r as f64);
            let (a, b, c, d) = (g.get(i, j), g.get(i + 1, j), g.get(i, j + 1), g.get(i + 1, j + 1));
            if a == SANS || b == SANS || c == SANS || d == SANS {
                continue;
            }
            let z = (a as f64 * (1.0 - fx) + b as f64 * fx) * (1.0 - fy) + (c as f64 * (1.0 - fx) + d as f64 * fx) * fy;
            let e = (z - v as f64).abs();
            s2 += e * e;
            mx = mx.max(e.round() as i64);
            hist[((e / 5.0) as usize).min(63)] += 1;
            n += 1;
        }
    }
    (s2, mx, n, hist)
}

/// Rééchantillonne une grille de 5 m native (dalles de 5 km) aux nœuds de la
/// tile : valeur du pixel natif au même point (les centres coïncident).
pub fn noeuds_natifs_5m(dalles: &HashMap<(i64, i64), Dalle>, x0: i64, y0: i64) -> Noeuds {
    let mut g = Noeuds::new(5);
    let m = g.m;
    for j in 0..m {
        for i in 0..m {
            let (x, y) = (x0 + 5 * i as i64, y0 + 5 * j as i64);
            // Dalle de 5 km qui contient le centre (x, y) : bord ouest et nord inclus.
            let xd = x.div_euclid(5000) * 5000;
            let yd = (y - 5).div_euclid(5000) * 5000 + 5000;
            if let Some(d) = dalles.get(&(xd, yd)) {
                if let Some(v) = d.get(x, y) {
                    g.cm[j * m + i] = v;
                }
            }
        }
    }
    g
}
