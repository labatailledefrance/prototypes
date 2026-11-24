//! Format d'une tile et des paquets de tiles.
//!
//! Une tile = un en-tête, une table de sections, les sections (chacune
//! compressée seule en zstd, pour ne décoder que ce dont on a besoin : la vue
//! simplifiée n'a pas besoin du relief de 1 m), puis une empreinte BLAKE3 de
//! tout ce qui précède.
//!
//! Les tiles sont rangées dans des **paquets** de 16 × 16 tiles (un fichier
//! par bloc de 16 km), avec un index en tête : lire une tile, c'est un `pread`
//! à un décalage connu. Analogie : une image de conteneur en couches, ou un
//! fichier PMTiles ; pas 600 000 petits fichiers.
//!
//! Tout est petit-boutien, les entiers variables en LEB128.

use crate::geo::TileId;

pub const MAGIC: &[u8; 4] = b"P07T";
pub const VERSION: u8 = 1;
pub const BLOC: i32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Sec {
    Relief = 1,
    Bati = 2,
    Franch = 3,
    Zone = 4,
    Reseau = 5,
    Ponts = 6,
    Occup = 7,
    Communes = 8,
    Lieux = 9,
}

pub const SECTIONS: &[(Sec, &str)] = &[
    (Sec::Relief, "relief"),
    (Sec::Bati, "bati"),
    (Sec::Franch, "franchissable"),
    (Sec::Zone, "zone"),
    (Sec::Reseau, "reseau"),
    (Sec::Ponts, "ponts"),
    (Sec::Occup, "occupation"),
    (Sec::Communes, "communes"),
    (Sec::Lieux, "lieux"),
];

impl Sec {
    pub fn de(v: u8) -> Sec {
        SECTIONS.iter().find(|s| s.0 as u8 == v).expect("section inconnue").0
    }
    pub fn nom(self) -> &'static str {
        SECTIONS.iter().find(|s| s.0 == self).unwrap().1
    }
}

/// En-tête : ce qu'il faut sans rien décompresser (part de bâti, type,
/// habitants) : la vue simplifiée et les tiles inertes n'ont besoin que de lui.
#[derive(Clone, Debug, Default)]
pub struct Entete {
    pub ix: i32,
    pub iy: i32,
    pub zone: u8,
    pub flags: u8, // 1 = terre, 2 = mer, 4 = relief présent
    pub part_bati: u16, // sur 65 535
    pub habitants: u32,
    pub nb_bat: u32,
}

pub struct Section {
    pub sec: Sec,
    pub brut: usize,
    pub donnees: Vec<u8>, // compressées
}

pub fn compresser(sec: Sec, brut: &[u8], niveau: i32) -> Section {
    let donnees = zstd::bulk::compress(brut, niveau).unwrap();
    Section { sec, brut: brut.len(), donnees }
}

pub fn assembler(e: &Entete, sections: &[Section]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(e.zone);
    out.extend_from_slice(&e.ix.to_le_bytes());
    out.extend_from_slice(&e.iy.to_le_bytes());
    out.push(e.flags);
    out.extend_from_slice(&e.part_bati.to_le_bytes());
    out.extend_from_slice(&e.habitants.to_le_bytes());
    out.extend_from_slice(&e.nb_bat.to_le_bytes());
    out.push(sections.len() as u8);
    for s in sections {
        out.push(s.sec as u8);
        out.extend_from_slice(&(s.brut as u32).to_le_bytes());
        out.extend_from_slice(&(s.donnees.len() as u32).to_le_bytes());
    }
    for s in sections {
        out.extend_from_slice(&s.donnees);
    }
    let h = blake3::hash(&out);
    out.extend_from_slice(h.as_bytes());
    out
}

pub struct TileLue<'a> {
    pub entete: Entete,
    pub sections: Vec<(Sec, usize, &'a [u8])>,
}

/// Lit une tile ; vérifie l'empreinte si demandé.
pub fn lire(b: &[u8], verifier: bool) -> TileLue<'_> {
    assert_eq!(&b[..4], MAGIC);
    if verifier {
        let n = b.len() - 32;
        assert_eq!(blake3::hash(&b[..n]).as_bytes(), &b[n..], "empreinte de tile fausse");
    }
    let mut p = 6;
    let i32_ = |p: &mut usize| {
        let v = i32::from_le_bytes(b[*p..*p + 4].try_into().unwrap());
        *p += 4;
        v
    };
    let ix = i32_(&mut p);
    let iy = i32_(&mut p);
    let flags = b[p];
    p += 1;
    let part_bati = u16::from_le_bytes([b[p], b[p + 1]]);
    p += 2;
    let habitants = i32_(&mut p) as u32;
    let nb_bat = i32_(&mut p) as u32;
    let ns = b[p] as usize;
    p += 1;
    let mut table = Vec::new();
    for _ in 0..ns {
        let sec = Sec::de(b[p]);
        let brut = u32::from_le_bytes(b[p + 1..p + 5].try_into().unwrap()) as usize;
        let st = u32::from_le_bytes(b[p + 5..p + 9].try_into().unwrap()) as usize;
        table.push((sec, brut, st));
        p += 9;
    }
    let mut sections = Vec::new();
    for (sec, brut, st) in table {
        sections.push((sec, brut, &b[p..p + st]));
        p += st;
    }
    TileLue { entete: Entete { ix, iy, zone: b[5], flags, part_bati, habitants, nb_bat }, sections }
}

impl TileLue<'_> {
    pub fn section(&self, s: Sec) -> Option<Vec<u8>> {
        self.sections.iter().find(|x| x.0 == s).map(|(_, brut, d)| zstd::bulk::decompress(d, *brut).unwrap())
    }
}

/// Bloc (paquet) d'une tile.
pub fn bloc(t: TileId) -> (i32, i32) {
    (t.ix.div_euclid(BLOC), t.iy.div_euclid(BLOC))
}

/// Écrit un paquet : index (nombre, puis ix, iy, décalage, taille) puis les
/// tiles, dans l'ordre (iy, ix).
pub fn paquet(tiles: &[(TileId, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"P07P");
    out.extend_from_slice(&(tiles.len() as u32).to_le_bytes());
    let tete = 8 + tiles.len() * 20;
    let mut off = tete as u64;
    for (t, b) in tiles {
        out.extend_from_slice(&t.ix.to_le_bytes());
        out.extend_from_slice(&t.iy.to_le_bytes());
        out.extend_from_slice(&off.to_le_bytes());
        out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        off += b.len() as u64;
    }
    for (_, b) in tiles {
        out.extend_from_slice(b);
    }
    out
}

/// Index d'un paquet : (tile, décalage, taille).
pub fn index_paquet(b: &[u8]) -> Vec<(TileId, u64, usize)> {
    assert_eq!(&b[..4], b"P07P");
    let n = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    (0..n)
        .map(|k| {
            let p = 8 + k * 20;
            let ix = i32::from_le_bytes(b[p..p + 4].try_into().unwrap());
            let iy = i32::from_le_bytes(b[p + 4..p + 8].try_into().unwrap());
            let off = u64::from_le_bytes(b[p + 8..p + 16].try_into().unwrap());
            let t = u32::from_le_bytes(b[p + 16..p + 20].try_into().unwrap()) as usize;
            (TileId { ix, iy }, off, t)
        })
        .collect()
}

// ------------------------------------------------------------ Entiers variables

#[derive(Default)]
pub struct Ecrivain {
    pub b: Vec<u8>,
}

impl Ecrivain {
    pub fn u(&mut self, mut v: u64) {
        loop {
            let o = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                self.b.push(o);
                break;
            }
            self.b.push(o | 0x80);
        }
    }
    pub fn i(&mut self, v: i64) {
        self.u(((v << 1) ^ (v >> 63)) as u64);
    }
    pub fn o(&mut self, v: u8) {
        self.b.push(v);
    }
    pub fn s(&mut self, s: &str) {
        self.u(s.len() as u64);
        self.b.extend_from_slice(s.as_bytes());
    }
}

pub struct LecteurV<'a> {
    pub b: &'a [u8],
    pub p: usize,
}

impl<'a> LecteurV<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        LecteurV { b, p: 0 }
    }
    pub fn u(&mut self) -> u64 {
        let (mut v, mut s) = (0u64, 0);
        loop {
            let o = self.b[self.p];
            self.p += 1;
            v |= ((o & 0x7F) as u64) << s;
            if o & 0x80 == 0 {
                return v;
            }
            s += 7;
        }
    }
    pub fn i(&mut self) -> i64 {
        let z = self.u();
        ((z >> 1) as i64) ^ -((z & 1) as i64)
    }
    pub fn o(&mut self) -> u8 {
        self.p += 1;
        self.b[self.p - 1]
    }
    pub fn s(&mut self) -> String {
        let n = self.u() as usize;
        self.p += n;
        String::from_utf8_lossy(&self.b[self.p - n..self.p]).into_owned()
    }
    pub fn fini(&self) -> bool {
        self.p >= self.b.len()
    }
}

/// Quantification des coordonnées locales : 1/16 m, relatives au centre de
/// la tile (C-01 : position locale centrée).
pub const QUANT: f64 = 16.0;

pub fn q(v: f64, centre: f64) -> i64 {
    ((v - centre) * QUANT).round() as i64
}
