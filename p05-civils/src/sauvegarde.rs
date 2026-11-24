//! Sauvegarde d'une tile désactivée, et journal de migration.
//!
//! **Normalisation** à la désactivation : chaque zombie redevient errant
//! (son état sera recalculé à la restauration : veille profonde s'il n'y a
//! aucun humain à portée) ; sa position est arrondie au 1/64 de mètre
//! (u16 sur 1 000 m) et son cap au 1/256 de tour ; la liste est triée par
//! position. C'est l'état normalisé qui est sauvegardé, et la restauration
//! le rend **bit à bit** (somme de contrôle avant écriture, après lecture).
//!
//! **Journal** : une tile sauvegardée n'est jamais réécrite par la
//! migration. Les arrivées et les départs s'ajoutent à un petit journal en
//! mémoire, rejoué à l'activation suivante, comme un journal d'écriture
//! anticipée (WAL) rejoué au redémarrage d'une base.

use crate::alea::{hacher, unite};
use crate::carte::Carte;

pub const MAGIC: &[u8; 4] = b"P04T";
pub const VERSION: u16 = 1;
/// Quanta par mètre.
pub const QUANTUM: f32 = 64.0;
/// Corps : cases de 10 m, 100 × 100 par tile.
pub const COTE_CORPS: usize = 100;

/// Un zombie sauvegardé : position au 1/64 m, cap au 1/256 de tour.
/// L'ordre des champs est celui du tri (y, puis x, puis cap).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZSauve {
    pub qy: u16,
    pub qx: u16,
    pub qcap: u8,
}

impl ZSauve {
    pub fn de(pos: [f32; 2], cap: f32) -> ZSauve {
        let q = |v: f32| (v * QUANTUM).floor().clamp(0.0, 1000.0 * QUANTUM - 1.0) as u16;
        let tour = cap.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
        ZSauve {
            qx: q(pos[0]),
            qy: q(pos[1]),
            qcap: ((tour * 256.0).floor() as u32 % 256) as u8,
        }
    }
    pub fn pos(&self) -> [f32; 2] {
        [(self.qx as f32 + 0.5) / QUANTUM, (self.qy as f32 + 0.5) / QUANTUM]
    }
    pub fn cap(&self) -> f32 {
        (self.qcap as f32 + 0.5) / 256.0 * std::f32::consts::TAU
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuileSauvee {
    pub x_km: i64,
    pub y_km: i64,
    pub tick: u64,
    pub zombies: Vec<ZSauve>,
    /// Carte des corps : un compteur par case de 10 m.
    pub corps: Vec<u16>,
    /// Constructions : (x, y, points de vie ; 0 = détruite).
    pub constructions: Vec<(u16, u16, i32)>,
}

/// Somme de contrôle du contenu (ordre compris).
pub fn somme(t: &TuileSauvee) -> u64 {
    let mut h = hacher(t.x_km as u64, t.y_km as u64 ^ t.tick.rotate_left(17));
    for z in &t.zombies {
        h = hacher(h, (z.qy as u64) << 24 | (z.qx as u64) << 8 | z.qcap as u64);
    }
    for (i, &c) in t.corps.iter().enumerate() {
        if c != 0 {
            h = hacher(h, (i as u64) << 16 | c as u64);
        }
    }
    for &(x, y, pv) in &t.constructions {
        h = hacher(h, (x as u64) << 48 | (y as u64) << 32 | pv as u32 as u64);
    }
    h
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// Enregistrements de taille fixe (5 octets par zombie, corps en
    /// tableau complet).
    Brut,
    /// Brut, compressé par zstd (niveau 3).
    BrutZstd,
    /// Deltas et entiers de longueur variable (positions triées, corps
    /// creux).
    Compact,
    /// Compact, puis LZ4.
    Lz4,
    /// Compact, puis zstd (niveau 3).
    Zstd,
}

fn varint(v: &mut Vec<u8>, mut x: u64) {
    while x >= 0x80 {
        v.push((x as u8) | 0x80);
        x >>= 7;
    }
    v.push(x as u8);
}

struct Lecteur<'a> {
    b: &'a [u8],
    i: usize,
}

impl Lecteur<'_> {
    fn octet(&mut self) -> std::io::Result<u8> {
        let x = *self.b.get(self.i).ok_or_else(|| std::io::Error::other("tronqué"))?;
        self.i += 1;
        Ok(x)
    }
    fn varint(&mut self) -> std::io::Result<u64> {
        let (mut x, mut s) = (0u64, 0);
        loop {
            let o = self.octet()?;
            x |= ((o & 0x7F) as u64) << s;
            if o & 0x80 == 0 {
                return Ok(x);
            }
            s += 7;
            if s > 63 {
                return Err(std::io::Error::other("varint invalide"));
            }
        }
    }
    fn u16(&mut self) -> std::io::Result<u16> {
        Ok(u16::from_le_bytes([self.octet()?, self.octet()?]))
    }
    fn i32(&mut self) -> std::io::Result<i32> {
        Ok(i32::from_le_bytes([self.octet()?, self.octet()?, self.octet()?, self.octet()?]))
    }
}

fn corps_complet(c: &[u16]) -> Vec<u16> {
    let mut v = vec![0u16; COTE_CORPS * COTE_CORPS];
    let k = c.len().min(v.len());
    v[..k].copy_from_slice(&c[..k]);
    v
}

fn charge_brute(t: &TuileSauvee) -> Vec<u8> {
    let mut v = Vec::with_capacity(16 + t.zombies.len() * 5 + 20_000 + t.constructions.len() * 8);
    v.extend_from_slice(&(t.zombies.len() as u32).to_le_bytes());
    for z in &t.zombies {
        v.extend_from_slice(&z.qx.to_le_bytes());
        v.extend_from_slice(&z.qy.to_le_bytes());
        v.push(z.qcap);
    }
    for c in corps_complet(&t.corps) {
        v.extend_from_slice(&c.to_le_bytes());
    }
    v.extend_from_slice(&(t.constructions.len() as u32).to_le_bytes());
    for &(x, y, pv) in &t.constructions {
        v.extend_from_slice(&x.to_le_bytes());
        v.extend_from_slice(&y.to_le_bytes());
        v.extend_from_slice(&pv.to_le_bytes());
    }
    v
}

fn lire_brute(b: &[u8], t: &mut TuileSauvee) -> std::io::Result<()> {
    let mut l = Lecteur { b, i: 0 };
    let n = u32::from_le_bytes([l.octet()?, l.octet()?, l.octet()?, l.octet()?]) as usize;
    t.zombies = (0..n)
        .map(|_| Ok(ZSauve { qx: l.u16()?, qy: l.u16()?, qcap: l.octet()? }))
        .collect::<std::io::Result<_>>()?;
    t.corps = (0..COTE_CORPS * COTE_CORPS).map(|_| l.u16()).collect::<std::io::Result<_>>()?;
    let nc = u32::from_le_bytes([l.octet()?, l.octet()?, l.octet()?, l.octet()?]) as usize;
    t.constructions = (0..nc).map(|_| Ok((l.u16()?, l.u16()?, l.i32()?))).collect::<std::io::Result<_>>()?;
    Ok(())
}

/// Charge compacte : zombies triés, Δy en entier variable, puis Δx (même
/// ligne) ou x ; corps creux (écart, valeur) ; constructions (x, y, pv).
fn charge_compacte(t: &TuileSauvee) -> Vec<u8> {
    debug_assert!(t.zombies.windows(2).all(|w| w[0] <= w[1]), "zombies non triés");
    let mut v = Vec::with_capacity(16 + t.zombies.len() * 4);
    varint(&mut v, t.zombies.len() as u64);
    let (mut py, mut px) = (0u16, 0u16);
    for z in &t.zombies {
        let dy = z.qy - py;
        varint(&mut v, dy as u64);
        if dy == 0 {
            varint(&mut v, (z.qx - px) as u64);
        } else {
            v.extend_from_slice(&z.qx.to_le_bytes());
        }
        v.push(z.qcap);
        (py, px) = (z.qy, z.qx);
    }
    let nz: Vec<(usize, u16)> = t.corps.iter().copied().enumerate().filter(|&(_, c)| c != 0).collect();
    varint(&mut v, nz.len() as u64);
    let mut prec = 0usize;
    for (i, c) in nz {
        varint(&mut v, (i - prec) as u64);
        varint(&mut v, c as u64);
        prec = i;
    }
    varint(&mut v, t.constructions.len() as u64);
    for &(x, y, pv) in &t.constructions {
        v.extend_from_slice(&x.to_le_bytes());
        v.extend_from_slice(&y.to_le_bytes());
        varint(&mut v, pv as u32 as u64);
    }
    v
}

fn lire_compacte(b: &[u8], t: &mut TuileSauvee) -> std::io::Result<()> {
    let mut l = Lecteur { b, i: 0 };
    let n = l.varint()? as usize;
    let mut z = Vec::with_capacity(n);
    let (mut py, mut px) = (0u16, 0u16);
    for _ in 0..n {
        let dy = l.varint()? as u16;
        let qy = py + dy;
        let qx = if dy == 0 { px + l.varint()? as u16 } else { l.u16()? };
        let qcap = l.octet()?;
        z.push(ZSauve { qy, qx, qcap });
        (py, px) = (qy, qx);
    }
    t.zombies = z;
    let mut corps = vec![0u16; COTE_CORPS * COTE_CORPS];
    let nz = l.varint()? as usize;
    let mut i = 0usize;
    for _ in 0..nz {
        i += l.varint()? as usize;
        *corps.get_mut(i).ok_or_else(|| std::io::Error::other("corps hors tile"))? = l.varint()? as u16;
    }
    t.corps = corps;
    let nc = l.varint()? as usize;
    t.constructions = (0..nc)
        .map(|_| Ok((l.u16()?, l.u16()?, l.varint()? as u32 as i32)))
        .collect::<std::io::Result<_>>()?;
    Ok(())
}

/// En-tête : magie, version, format, tile, tick, somme de contrôle du
/// contenu (vérifiée à la lecture), taille de la charge.
const ENTETE: usize = 4 + 2 + 1 + 1 + 8 + 8 + 8 + 8 + 4;

pub fn encoder(t: &TuileSauvee, f: Format) -> Vec<u8> {
    let charge = match f {
        Format::Brut => charge_brute(t),
        Format::BrutZstd => zstd::bulk::compress(&charge_brute(t), 3).expect("zstd"),
        Format::Compact => charge_compacte(t),
        Format::Lz4 => lz4_flex::compress_prepend_size(&charge_compacte(t)),
        Format::Zstd => zstd::bulk::compress(&charge_compacte(t), 3).expect("zstd"),
    };
    let mut v = Vec::with_capacity(ENTETE + charge.len());
    v.extend_from_slice(MAGIC);
    v.extend_from_slice(&VERSION.to_le_bytes());
    v.push(f as u8);
    v.push(0);
    v.extend_from_slice(&t.x_km.to_le_bytes());
    v.extend_from_slice(&t.y_km.to_le_bytes());
    v.extend_from_slice(&t.tick.to_le_bytes());
    v.extend_from_slice(&somme(t).to_le_bytes());
    v.extend_from_slice(&(charge.len() as u32).to_le_bytes());
    v.extend_from_slice(&charge);
    v
}

fn format_de(b: u8) -> std::io::Result<Format> {
    Ok(match b {
        0 => Format::Brut,
        1 => Format::BrutZstd,
        2 => Format::Compact,
        3 => Format::Lz4,
        4 => Format::Zstd,
        _ => return Err(std::io::Error::other("format inconnu")),
    })
}

/// Décode et vérifie (magie, version, taille, somme de contrôle).
pub fn decoder(b: &[u8]) -> std::io::Result<TuileSauvee> {
    let err = |m: &str| std::io::Error::other(m.to_string());
    if b.len() < ENTETE || &b[..4] != MAGIC {
        return Err(err("pas une sauvegarde de tile"));
    }
    if u16::from_le_bytes([b[4], b[5]]) != VERSION {
        return Err(err("version inconnue"));
    }
    let f = format_de(b[6])?;
    let lire8 = |i: usize| <[u8; 8]>::try_from(&b[i..i + 8]).unwrap();
    let mut t = TuileSauvee {
        x_km: i64::from_le_bytes(lire8(8)),
        y_km: i64::from_le_bytes(lire8(16)),
        tick: u64::from_le_bytes(lire8(24)),
        zombies: vec![],
        corps: vec![],
        constructions: vec![],
    };
    let attendue = u64::from_le_bytes(lire8(32));
    let n = u32::from_le_bytes(b[40..44].try_into().unwrap()) as usize;
    let charge = b.get(ENTETE..ENTETE + n).ok_or_else(|| err("charge tronquée"))?;
    match f {
        Format::Brut => lire_brute(charge, &mut t)?,
        Format::BrutZstd => {
            let d = zstd::bulk::decompress(charge, 64 << 20).map_err(|e| err(&e.to_string()))?;
            lire_brute(&d, &mut t)?
        }
        Format::Compact => lire_compacte(charge, &mut t)?,
        Format::Lz4 => {
            let d = lz4_flex::decompress_size_prepended(charge).map_err(|e| err(&e.to_string()))?;
            lire_compacte(&d, &mut t)?
        }
        Format::Zstd => {
            let d = zstd::bulk::decompress(charge, 64 << 20).map_err(|e| err(&e.to_string()))?;
            lire_compacte(&d, &mut t)?
        }
    }
    if somme(&t) != attendue {
        return Err(err("somme de contrôle fausse"));
    }
    Ok(t)
}

// ------------------------------------------------------------ Journal

/// Côté d'une tile, en direction (dx, dy) ∈ {−1, 0, 1}² (diagonale = coin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cote {
    pub dx: i8,
    pub dy: i8,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mouvement {
    /// Les `k` zombies les plus proches du côté de sortie partent.
    Depart { cote: Cote, k: u32 },
    /// `k` zombies arrivent par ce côté, placés près du bord.
    Arrivee { cote: Cote, k: u32, graine: u64 },
    /// Un zombie entré à pied depuis une tile active, à sa position exacte.
    Exacte { pos: [f32; 2], cap: f32 },
}

/// Largeur de la bande d'arrivée le long du bord (m).
pub const BANDE: f32 = 20.0;

/// Distance (m) d'un point local au côté de sortie (coin : Tchebychev).
pub fn distance_cote(p: [f32; 2], c: Cote) -> f32 {
    let axe = |v: f32, d: i8| match d {
        1 => 1000.0 - v,
        -1 => v,
        _ => 0.0,
    };
    axe(p[0], c.dx).max(axe(p[1], c.dy))
}

/// Rectangle d'arrivée par un côté : [x0, x1[ × [y0, y1[.
pub fn bande(c: Cote) -> [f32; 4] {
    let axe = |d: i8| match d {
        1 => (1000.0 - BANDE, 1000.0),
        -1 => (0.0, BANDE),
        _ => (0.0, 1000.0),
    };
    let ((x0, x1), (y0, y1)) = (axe(c.dx), axe(c.dy));
    [x0, x1, y0, y1]
}

/// Positions d'arrivée par un côté (déterministes).
pub fn arrivees(carte: &Carte, t: usize, c: Cote, k: u32, graine: u64) -> Vec<([f32; 2], f32)> {
    if k == 0 {
        return vec![];
    }
    let cases = carte.cases_libres(t, bande(c));
    (0..k as u64)
        .map(|j| {
            let g = hacher(graine, j);
            let i = cases[(g % cases.len() as u64) as usize] as usize;
            let p = [(i % 1000) as f32 + unite(hacher(g, 3)), (i / 1000) as f32 + unite(hacher(g, 4))];
            (p, unite(hacher(g, 1)) * std::f32::consts::TAU)
        })
        .collect()
}

/// Applique un événement du journal à la liste (position locale, cap).
pub fn appliquer(l: &mut Vec<([f32; 2], f32)>, m: &Mouvement, carte: &Carte, t: usize) {
    match *m {
        Mouvement::Depart { cote, k } => {
            let mut ordre: Vec<(f32, usize)> = l.iter().enumerate().map(|(i, z)| (distance_cote(z.0, cote), i)).collect();
            ordre.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let mut garder = vec![true; l.len()];
            for &(_, i) in ordre.iter().take(k as usize) {
                garder[i] = false;
            }
            let mut i = 0;
            l.retain(|_| {
                i += 1;
                garder[i - 1]
            });
        }
        Mouvement::Arrivee { cote, k, graine } => l.extend(arrivees(carte, t, cote, k, graine)),
        Mouvement::Exacte { pos, cap } => l.push((pos, cap)),
    }
}

/// Rejoue tout le journal sur la liste restaurée (ordre des événements).
#[allow(dead_code)]
pub fn rejouer(zombies: Vec<ZSauve>, journal: &[Mouvement], carte: &Carte, t: usize) -> Vec<([f32; 2], f32)> {
    let mut l: Vec<([f32; 2], f32)> = zombies.into_iter().map(|z| (z.pos(), z.cap())).collect();
    for m in journal {
        appliquer(&mut l, m, carte, t);
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alea::Alea;

    fn exemple(n: usize) -> TuileSauvee {
        let mut a = Alea::new(3);
        let mut z: Vec<ZSauve> =
            (0..n).map(|_| ZSauve::de([a.f32() * 1000.0, a.f32() * 1000.0], a.f32() * 7.0)).collect();
        z.sort_unstable();
        let mut corps = vec![0u16; COTE_CORPS * COTE_CORPS];
        for _ in 0..500 {
            let i = a.entre(0, corps.len() - 1);
            corps[i] += a.entre(1, 40) as u16;
        }
        TuileSauvee {
            x_km: 650,
            y_km: 6861,
            tick: 12345,
            zombies: z,
            corps,
            constructions: (0..300).map(|i| (i as u16, 2 * i as u16, (i * 7) as i32)).collect(),
        }
    }

    /// Tous les formats rendent exactement le contenu, et refusent une
    /// sauvegarde altérée.
    #[test]
    fn aller_retour_exact() {
        let t = exemple(20_000);
        for f in [Format::Brut, Format::BrutZstd, Format::Compact, Format::Lz4, Format::Zstd] {
            let b = encoder(&t, f);
            assert_eq!(decoder(&b).unwrap(), t, "{f:?}");
            assert_eq!(encoder(&decoder(&b).unwrap(), f), b, "{f:?} : réencodage");
            let mut c = b.clone();
            let k = c.len() - 3;
            c[k] ^= 0x10;
            assert!(decoder(&c).is_err() || decoder(&c).unwrap() != t, "{f:?} : altération non vue");
        }
    }

    /// Quantification : idempotente (une tile restaurée puis resauvegardée
    /// sans bouger donne la même sauvegarde).
    #[test]
    fn quantification_idempotente() {
        let mut a = Alea::new(9);
        for _ in 0..100_000 {
            let z = ZSauve::de([a.f32() * 1000.0, a.f32() * 1000.0], (a.f32() - 0.5) * 20.0);
            assert_eq!(ZSauve::de(z.pos(), z.cap()), z);
        }
    }
}
