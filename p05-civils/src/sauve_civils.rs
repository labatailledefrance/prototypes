//! Sauvegarde des colons d'une tile, au format de P-04 : instantané
//! normalisé (trié), compact, compressé en zstd, avec somme de contrôle.
//!
//! Par lieu de la tile : nombre de colons ; par colon, trié par identifiant :
//! écart d'identifiant (varint), écart de logement et de travail au lieu
//! (varint zigzag), ancienneté de la mise à jour (varint), jauges (5 × u16),
//! expérience et compétences (varint), maladie et drapeaux (1 octet).
//! Sans perte : l'état relu est identique octet pour octet.

use crate::alea::hacher;
use crate::colon::{Colon, NUL};

fn varint(v: &mut Vec<u8>, mut x: u64) {
    while x >= 0x80 {
        v.push((x as u8) | 0x80);
        x >>= 7;
    }
    v.push(x as u8);
}

fn zigzag(x: i64) -> u64 {
    ((x << 1) ^ (x >> 63)) as u64
}

fn dezigzag(x: u64) -> i64 {
    ((x >> 1) as i64) ^ -((x & 1) as i64)
}

struct Lecteur<'a> {
    b: &'a [u8],
    i: usize,
}

impl Lecteur<'_> {
    fn varint(&mut self) -> std::io::Result<u64> {
        let mut x = 0u64;
        let mut s = 0;
        loop {
            let o = *self.b.get(self.i).ok_or_else(|| std::io::Error::other("sauvegarde tronquée"))?;
            self.i += 1;
            x |= ((o & 0x7F) as u64) << s;
            if o < 0x80 {
                return Ok(x);
            }
            s += 7;
        }
    }
    fn u16(&mut self) -> std::io::Result<u16> {
        let v = self.b.get(self.i..self.i + 2).ok_or_else(|| std::io::Error::other("sauvegarde tronquée"))?;
        self.i += 2;
        Ok(u16::from_le_bytes([v[0], v[1]]))
    }
    fn u8(&mut self) -> std::io::Result<u8> {
        let v = *self.b.get(self.i).ok_or_else(|| std::io::Error::other("sauvegarde tronquée"))?;
        self.i += 1;
        Ok(v)
    }
}

/// Charge brute (avant compression) des dossiers `(lieu, colons)` d'une
/// tile, au tick `t`.
pub fn charge(dossiers: &[(u32, Vec<Colon>)], t: u32) -> Vec<u8> {
    let mut v = Vec::new();
    varint(&mut v, t as u64);
    varint(&mut v, dossiers.len() as u64);
    for (l, d) in dossiers {
        let mut d = d.clone();
        d.sort_unstable_by_key(|c| c.id);
        varint(&mut v, *l as u64);
        varint(&mut v, d.len() as u64);
        let mut prec = 0u32;
        for c in &d {
            varint(&mut v, (c.id - prec) as u64);
            prec = c.id;
            varint(&mut v, zigzag(c.logement as i64 - *l as i64));
            let tr = if c.travail == NUL { 0 } else { zigzag(c.travail as i64 - *l as i64) + 1 };
            varint(&mut v, tr);
            varint(&mut v, t.wrapping_sub(c.maj) as u64);
            for j in c.j {
                v.extend_from_slice(&j.to_le_bytes());
            }
            varint(&mut v, c.experience as u64);
            varint(&mut v, c.competences as u64);
            v.push(c.malade << 7 | c.drapeaux);
        }
    }
    v
}

pub fn lire(b: &[u8]) -> std::io::Result<Vec<(u32, Vec<Colon>)>> {
    let mut r = Lecteur { b, i: 0 };
    let t = r.varint()? as u32;
    let n = r.varint()? as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let l = r.varint()? as u32;
        let k = r.varint()? as usize;
        let mut d = Vec::with_capacity(k);
        let mut prec = 0u32;
        for _ in 0..k {
            let id = prec + r.varint()? as u32;
            prec = id;
            let logement = (l as i64 + dezigzag(r.varint()?)) as u32;
            let tr = r.varint()?;
            let travail = if tr == 0 { NUL } else { (l as i64 + dezigzag(tr - 1)) as u32 };
            let maj = t.wrapping_sub(r.varint()? as u32);
            let mut j = [0u16; 5];
            for x in &mut j {
                *x = r.u16()?;
            }
            let experience = r.varint()? as u16;
            let competences = r.varint()? as u16;
            let o = r.u8()?;
            d.push(Colon { id, maj, logement, travail, j, experience, competences, malade: o >> 7, drapeaux: o & 0x7F });
        }
        out.push((l, d));
    }
    Ok(out)
}

pub fn somme(dossiers: &[(u32, Vec<Colon>)]) -> u64 {
    let mut h = 0u64;
    for (l, d) in dossiers {
        for c in d {
            h = h.wrapping_add(crate::colon::empreinte(c, *l));
        }
    }
    hacher(h, dossiers.len() as u64)
}

/// Fichier de tile : somme de contrôle (8 octets) + charge en zstd.
pub fn encoder(dossiers: &[(u32, Vec<Colon>)], t: u32) -> Vec<u8> {
    let c = charge(dossiers, t);
    let z = zstd::bulk::compress(&c, 3).expect("zstd");
    let mut v = somme(dossiers).to_le_bytes().to_vec();
    v.extend_from_slice(&z);
    v
}

pub fn decoder(b: &[u8]) -> std::io::Result<Vec<(u32, Vec<Colon>)>> {
    let s = u64::from_le_bytes(b[..8].try_into().unwrap());
    let c = zstd::stream::decode_all(&b[8..])?;
    let d = lire(&c)?;
    if somme(&d) != s {
        return Err(std::io::Error::other("somme de contrôle fausse"));
    }
    Ok(d)
}
