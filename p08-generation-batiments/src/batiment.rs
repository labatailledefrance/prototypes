//! Modèle et maillage d'un bâtiment.
//!
//! **Modèle** (ce que l'on déduit des attributs, sans géométrie) : hauteur à
//! la gouttière et sa source (donnée, étages, altitudes du toit, repli),
//! nombre de niveaux, forme et hauteur du toit, couleurs.
//!
//! **Maillage**, pour chaque polygone de l'emprise réelle (trous compris) :
//! - un **soubassement** : du sol le plus bas (moins un enfouissement) au sol
//!   le plus haut sous l'emprise, qui absorbe la pente ;
//! - des **façades** du rez-de-chaussée au toit, avec une grille de fenêtres
//!   en coordonnées de texture (une travée, un niveau) : rien à stocker, la
//!   position de chaque fenêtre se recalcule ;
//! - un **toit** décrit comme une fonction affine par morceaux sur une
//!   partition convexe du rectangle englobant orienté (1 cellule : plat ;
//!   2 : deux pans ; 4 : quatre pans ; 5 : mansarde). L'emprise est découpée
//!   par chaque cellule (Sutherland-Hodgman), puis triangulée ; les murs
//!   montent jusqu'à la même fonction, ce qui forme les pignons tout seuls,
//!   pour n'importe quelle emprise (concave, à cours, multipolygone).
//! - un **porche** là où P-07 a percé des cases : le bas du bâtiment est
//!   l'emprise moins les cases percées (opération booléenne entière), le
//!   haut l'emprise entière, avec un plafond de passage ;
//! - la **porte**, posée par `porte.rs`.
//!
//! Analogie : un modèle de bâtiment est un gabarit (template) instancié avec
//! les attributs de la BD TOPO ; le maillage est le rendu du gabarit, aussi
//! reproductible qu'un build à partir des mêmes sources.

use crate::donnees::{Bat, nat, usa};
use crate::geom::{self, DemiPlan, Obb, P2};
use std::collections::BTreeMap;

/// Paramètres de prototype : **hypothèses de mesure**, pas des statistiques
/// de jeu.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Hauteur d'un niveau (m), pour les étages et les fenêtres.
    pub etage: f64,
    /// Largeur visée d'une travée de fenêtres (m).
    pub travee: f64,
    /// Enfouissement du soubassement sous le sol le plus bas (m).
    pub enfouissement: f64,
    /// Hauteur d'un porche (m).
    pub porche: f64,
    /// Toit plat sous cette hauteur de toit (m).
    pub toit_plat: f64,
    /// Pente par défaut d'un toit à pans (tan 40°), pente maximale (tan 60°).
    pub pente: f64,
    pub pente_max: f64,
    /// Tolérance de simplification de la vue lointaine (m).
    pub tol_loin: f64,
    /// Hauteur maximale du soubassement (m).
    pub socle_max: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params { etage: 3.0, travee: 3.0, enfouissement: 0.5, porche: 4.0, toit_plat: 0.5, pente: 0.839, pente_max: 1.732, tol_loin: 1.0, socle_max: 3.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lod {
    /// Vue isométrique : modèle complet.
    Iso,
    /// Vue lointaine : emprise simplifiée, murs d'un seul tenant, sans
    /// fenêtres, porche ni porte.
    Loin,
    /// Vue simplifiée (carte) : emprise plate.
    Carte,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Etat {
    Intact,
    Endommage,
    Effondre,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Forme {
    Plat,
    DeuxPans,
    QuatrePans,
    Mansarde,
}

impl Forme {
    pub fn nom(self) -> &'static str {
        match self {
            Forme::Plat => "plat",
            Forme::DeuxPans => "deux_pans",
            Forme::QuatrePans => "quatre_pans",
            Forme::Mansarde => "mansarde",
        }
    }
}

/// Source de la hauteur.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceH {
    Donnee,
    Etages,
    AltitudesToit,
    MedianeTile,
    Constante,
}

impl SourceH {
    pub fn nom(self) -> &'static str {
        match self {
            SourceH::Donnee => "hauteur",
            SourceH::Etages => "etages",
            SourceH::AltitudesToit => "altitudes_toit",
            SourceH::MedianeTile => "mediane_tile",
            SourceH::Constante => "constante_usage",
        }
    }
}

#[inline]
pub fn hacher(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

// ------------------------------------------------------------ Repli de la hauteur

pub fn classe_emprise(a: f64) -> u8 {
    match a {
        a if a < 20.0 => 0,
        a if a < 100.0 => 1,
        a if a < 500.0 => 2,
        a if a < 2000.0 => 3,
        _ => 4,
    }
}

/// Hauteur par défaut selon l'usage (m) : dernier recours.
pub fn constante_usage(b: &Bat) -> f64 {
    if b.leger {
        return 2.5;
    }
    match b.usage1 {
        usa::RESID => 7.0,
        usa::ANNEXE => 3.0,
        usa::COMMERCE => 7.0,
        usa::INDUS => 8.0,
        usa::SPORT => 8.0,
        usa::RELIG => 12.0,
        usa::AGRI => 6.0,
        _ => 5.0,
    }
}

/// Médianes des hauteurs connues de la tile, par (usage, classe d'emprise)
/// et par usage. Ne dépend que du contenu de la tile : même repli quel que
/// soit l'ordre de génération.
#[derive(Default, Clone)]
pub struct Repli {
    pub par_classe: BTreeMap<(u8, u8), f64>,
    pub par_usage: BTreeMap<u8, f64>,
}

fn mediane(v: &mut [u32]) -> f64 {
    v.sort_unstable();
    v[v.len() / 2] as f64 / 10.0
}

pub fn repli(bats: &[Bat], sauf: Option<u64>) -> Repli {
    let mut c: BTreeMap<(u8, u8), Vec<u32>> = BTreeMap::new();
    let mut u: BTreeMap<u8, Vec<u32>> = BTreeMap::new();
    for b in bats {
        if Some(b.id) == sauf {
            continue;
        }
        if let Some(h) = b.hauteur_dm {
            c.entry((b.usage1, classe_emprise(b.aire()))).or_default().push(h);
            u.entry(b.usage1).or_default().push(h);
        }
    }
    let mut r = Repli::default();
    for (k, mut v) in c {
        if v.len() >= 5 {
            r.par_classe.insert(k, mediane(&mut v));
        }
    }
    for (k, mut v) in u {
        if v.len() >= 5 {
            r.par_usage.insert(k, mediane(&mut v));
        }
    }
    r
}

/// Hauteur à la gouttière (m) et sa source. `avec_donnee = false` ignore la
/// hauteur de la BD TOPO (validation croisée de la règle de repli).
pub fn hauteur(b: &Bat, r: &Repli, p: &Params, avec_donnee: bool) -> (f64, SourceH) {
    if avec_donnee && let Some(h) = b.hauteur_dm.filter(|h| *h >= 10) {
        return (h as f64 / 10.0, SourceH::Donnee);
    }
    if let Some(e) = b.etages.filter(|e| *e > 0) {
        return (e as f64 * p.etage, SourceH::Etages);
    }
    if let Some(g) = b.extra.gout_dm.filter(|g| *g >= 10) {
        return (g as f64 / 10.0, SourceH::AltitudesToit);
    }
    if let Some(h) = r.par_classe.get(&(b.usage1, classe_emprise(b.aire()))).or_else(|| r.par_usage.get(&b.usage1)) {
        return (*h, SourceH::MedianeTile);
    }
    (constante_usage(b), SourceH::Constante)
}

// ------------------------------------------------------------ Modèle

#[derive(Clone, Debug)]
pub struct Modele {
    pub h: f64,
    pub source_h: SourceH,
    pub niveaux: u32,
    pub forme: Forme,
    /// Hauteur du toit (m) ; `dz_donnee` : tirée de la BD TOPO.
    pub dz: f64,
    pub dz_donnee: bool,
    pub mur: [u8; 3],
    pub toit: [u8; 3],
    pub fenetres: bool,
}

/// Couleur de façade : premier chiffre du code de matériau des fichiers
/// fonciers (1 pierre, 2 meulière, 3 béton, 4 brique, 5 aggloméré, 6 bois ;
/// à vérifier), sinon une teinte de pierre tirée de la graine.
fn couleur_mur(b: &Bat, h: u64) -> [u8; 3] {
    let pal: [[u8; 3]; 5] = [[205, 195, 175], [190, 180, 165], [215, 205, 190], [178, 172, 160], [200, 186, 160]];
    match b.extra.murs / 10 {
        1 => [196, 184, 160],
        2 => [182, 150, 112],
        3 => [168, 168, 163],
        4 => [165, 92, 66],
        5 => [190, 186, 176],
        6 => [128, 94, 64],
        _ => pal[(h % 5) as usize],
    }
}

/// Couleur de toiture : 1 tuiles, 2 ardoises, 3 zinc ou aluminium, 4 béton
/// (à vérifier) ; sinon selon la forme et la graine.
fn couleur_toit(b: &Bat, f: Forme, h: u64) -> [u8; 3] {
    match b.extra.toiture / 10 {
        1 => [158, 76, 52],
        2 => [72, 76, 86],
        3 => [126, 131, 136],
        4 => [150, 150, 145],
        _ => match f {
            Forme::Plat => [158, 152, 142],
            Forme::Mansarde => [96, 102, 110],
            _ => {
                if h >> 8 & 1 == 0 {
                    [158, 76, 52]
                } else {
                    [72, 76, 86]
                }
            }
        },
    }
}

pub fn modele(b: &Bat, r: &Repli, p: &Params) -> Modele {
    let (h, source_h) = hauteur(b, r, p, true);
    let niveaux = b.etages.filter(|e| *e > 0).unwrap_or_else(|| ((h / p.etage).round() as u32).max(1));
    let g = hacher(b.id);
    // Rectangle englobant du plus grand polygone.
    let grand = b.polys.iter().max_by(|x, y| geom::aire2_i(&x[0]).abs().cmp(&geom::aire2_i(&y[0]).abs())).unwrap();
    let o = geom::obb(&grand[0]);
    let (w, aire) = (2.0 * o.w, b.aire());
    let resid = matches!(b.usage1, usa::RESID | usa::INDIFF | usa::COMMERCE);
    let indus = b.nature == nat::INDUS || matches!(b.usage1, usa::INDUS | usa::AGRI) || aire > 1000.0;
    let toit_data = b.extra.toit_dm.map(|d| d as f64 / 10.0);
    let (forme, dz, dz_donnee) = if matches!(b.nature, nat::SILO | nat::TOUR | nat::PHARE | nat::FORT | nat::TRIBUNE | nat::ARC | nat::ARENE | nat::MONUMENT) {
        (Forme::Plat, 0.0, false)
    } else if let Some(dz) = toit_data {
        if dz < p.toit_plat {
            (Forme::Plat, 0.0, true)
        } else if matches!(b.nature, nat::EGLISE | nat::CHAPELLE) || indus {
            (Forme::DeuxPans, dz, true)
        } else if resid && h >= 10.0 && niveaux >= 3 && dz >= 2.0 && w >= 8.0 {
            (Forme::Mansarde, dz, true)
        } else if o.allongement() < 1.6 || g % 4 == 0 {
            (Forme::QuatrePans, dz, true)
        } else {
            (Forme::DeuxPans, dz, true)
        }
    } else if b.leger || (b.usage1 == usa::ANNEXE && aire < 40.0) {
        (Forme::Plat, 0.0, false)
    } else if matches!(b.nature, nat::EGLISE | nat::CHAPELLE) {
        (Forme::DeuxPans, 1.4 * p.pente * o.w, false)
    } else if b.nature == nat::SERRE {
        (Forme::DeuxPans, 0.3 * o.w, false)
    } else if indus {
        if g % 2 == 0 { (Forme::Plat, 0.0, false) } else { (Forme::DeuxPans, (0.27 * o.w).min(6.0), false) }
    } else if h >= 12.0 {
        if g % 2 == 0 { (Forme::Mansarde, 3.5f64.min(0.35 * w * 2.75), false) } else { (Forme::Plat, 0.0, false) }
    } else if o.allongement() < 1.6 && g % 3 == 0 {
        (Forme::QuatrePans, (p.pente * o.w).min(8.0), false)
    } else {
        (Forme::DeuxPans, (p.pente * o.w).min(8.0), false)
    };
    let fenetres = !(b.leger || h < 2.2 || (b.usage1 == usa::ANNEXE && aire < 20.0) || b.nature == nat::SERRE || b.nature == nat::SILO);
    let mut mur = couleur_mur(b, g);
    if b.nature == nat::SERRE {
        mur = [200, 222, 212];
    }
    Modele { h, source_h, niveaux, forme, dz, dz_donnee, mur, toit: couleur_toit(b, forme, g), fenetres }
}

// ------------------------------------------------------------ Toit

/// Toit d'un polygone : fonction z(p) affine par morceaux sur des cellules
/// convexes, et droites de pli (pour insérer des sommets dans les murs).
pub struct Toit {
    pub o: Obb,
    pub forme: Forme,
    pub z0: f64,
    pub dz: f64,
    /// Mansarde : largeur du brisis (m).
    pub e: f64,
    pub cellules: Vec<Vec<DemiPlan>>,
    pub plis: Vec<DemiPlan>,
}

fn dp(al: f64, be: f64, ga: f64) -> DemiPlan {
    DemiPlan { al, be, ga }
}

impl Toit {
    pub fn new(o: Obb, mut forme: Forme, z0: f64, dz: f64, p: &Params) -> Toit {
        let (l, w) = (o.l, o.w);
        // Pente bornée : un toit ne dépasse pas 60° sur sa demi-largeur.
        let mut dz = dz.min(p.pente_max * w).max(0.0);
        if dz < 0.05 {
            forme = Forme::Plat;
            dz = 0.0;
        }
        let mut e = 0.0;
        if forme == Forme::Mansarde {
            // Brisis à 70° (tan = 2,75), au plus 35 % de la demi-largeur.
            e = (dz / 2.75).min(0.35 * w);
            if w < 4.0 || e < 0.3 {
                forme = Forme::DeuxPans;
            }
        }
        let r = l - w;
        let cellules: Vec<Vec<DemiPlan>> = match forme {
            Forme::Plat => vec![vec![]],
            Forme::DeuxPans => vec![vec![dp(0.0, 1.0, 0.0)], vec![dp(0.0, -1.0, 0.0)]],
            Forme::QuatrePans => vec![
                vec![dp(0.0, 1.0, 0.0), dp(-1.0, 1.0, r), dp(1.0, 1.0, r)],
                vec![dp(0.0, -1.0, 0.0), dp(-1.0, -1.0, r), dp(1.0, -1.0, r)],
                vec![dp(1.0, -1.0, -r), dp(1.0, 1.0, -r)],
                vec![dp(-1.0, -1.0, -r), dp(-1.0, 1.0, -r)],
            ],
            Forme::Mansarde => vec![
                vec![dp(0.0, 1.0, -(w - e)), dp(-1.0, 1.0, r), dp(1.0, 1.0, r)],
                vec![dp(0.0, -1.0, -(w - e)), dp(-1.0, -1.0, r), dp(1.0, -1.0, r)],
                vec![dp(1.0, 0.0, -(l - e)), dp(1.0, -1.0, -r), dp(1.0, 1.0, -r)],
                vec![dp(-1.0, 0.0, -(l - e)), dp(-1.0, -1.0, -r), dp(-1.0, 1.0, -r)],
                vec![dp(0.0, -1.0, w - e), dp(0.0, 1.0, w - e), dp(-1.0, 0.0, l - e), dp(1.0, 0.0, l - e)],
            ],
        };
        // Plis : droites distinctes des cellules (orientation normalisée).
        let mut plis: Vec<DemiPlan> = Vec::new();
        for c in &cellules {
            for d in c {
                let n = if d.al < 0.0 || (d.al == 0.0 && d.be < 0.0) { dp(-d.al, -d.be, -d.ga) } else { *d };
                if !plis.contains(&n) {
                    plis.push(n);
                }
            }
        }
        Toit { o, forme, z0, dz, e, cellules, plis }
    }

    /// Altitude du toit au point p.
    #[inline]
    pub fn z(&self, p: P2) -> f64 {
        let (u, v) = self.o.uv(p);
        let (l, w) = (self.o.l, self.o.w);
        match self.forme {
            Forme::Plat => self.z0,
            Forme::DeuxPans => self.z0 + self.dz * (1.0 - v.abs() / w),
            Forme::QuatrePans => self.z0 + self.dz / w * (w - v.abs()).min(l - u.abs()),
            Forme::Mansarde => self.z0 + (self.dz / self.e * (w - v.abs()).min(l - u.abs())).min(self.dz),
        }
    }

    /// Gradient (dz/du, dz/dv) de la cellule k.
    fn gradient(&self, k: usize) -> (f64, f64) {
        let (s4, s) = (self.dz / self.o.w, if self.e > 0.0 { self.dz / self.e } else { 0.0 });
        match (self.forme, k) {
            (Forme::Plat, _) => (0.0, 0.0),
            (Forme::DeuxPans, 0) => (0.0, -self.dz / self.o.w),
            (Forme::DeuxPans, _) => (0.0, self.dz / self.o.w),
            (Forme::QuatrePans, 0) => (0.0, -s4),
            (Forme::QuatrePans, 1) => (0.0, s4),
            (Forme::QuatrePans, 2) => (-s4, 0.0),
            (Forme::QuatrePans, _) => (s4, 0.0),
            (Forme::Mansarde, 0) => (0.0, -s),
            (Forme::Mansarde, 1) => (0.0, s),
            (Forme::Mansarde, 2) => (-s, 0.0),
            (Forme::Mansarde, 3) => (s, 0.0),
            (Forme::Mansarde, _) => (0.0, 0.0),
        }
    }

    /// Normale (repère local x est, y nord, z haut) de la cellule k.
    pub fn normale(&self, k: usize) -> [f64; 3] {
        let (gu, gv) = self.gradient(k);
        let (gx, gy) = (gu * self.o.a[0] + gv * self.o.b[0], gu * self.o.a[1] + gv * self.o.b[1]);
        let n = (gx * gx + gy * gy + 1.0).sqrt();
        [-gx / n, -gy / n, 1.0 / n]
    }
}

// ------------------------------------------------------------ Maillage

/// Maillage d'un bâtiment ou d'une tile. Positions en f32 locales au centre
/// de la tile propriétaire (C-01), dans le repère de Bevy (x est, y haut,
/// z sud).
#[derive(Default, Clone)]
pub struct Maillage {
    pub pos: Vec<[f32; 3]>,
    pub nor: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    pub col: Vec<[u8; 4]>,
    pub idx: Vec<u32>,
}

/// Coordonnée de texture hors des fenêtres (mur nu, toit).
pub const UV_NU: [f32; 2] = [0.04, 0.04];

#[inline]
fn bevy(x: f64, y: f64, z: f64) -> [f32; 3] {
    [x as f32, z as f32, -y as f32]
}

impl Maillage {
    pub fn sommet(&mut self, p: [f64; 3], n: [f64; 3], uv: [f32; 2], c: [u8; 3]) -> u32 {
        self.pos.push(bevy(p[0], p[1], p[2]));
        self.nor.push(bevy(n[0], n[1], n[2]));
        self.uv.push(uv);
        self.col.push([c[0], c[1], c[2], 255]);
        (self.pos.len() - 1) as u32
    }
    /// Quadrilatère vertical (a, b en bas, sens direct vu de l'extérieur).
    #[allow(clippy::too_many_arguments)]
    pub fn mur(&mut self, a: P2, b: P2, za0: f64, zb0: f64, za1: f64, zb1: f64, n: [f64; 3], uva: [f32; 2], uvb: [f32; 2], v0: f32, v1a: f32, v1b: f32, c: [u8; 3], fen: bool) {
        if za1 - za0 < 1e-3 && zb1 - zb0 < 1e-3 {
            return;
        }
        let uv = |u: [f32; 2], v: f32| if fen { [u[0], v] } else { UV_NU };
        let i0 = self.sommet([a[0], a[1], za0], n, uv(uva, v0), c);
        let i1 = self.sommet([b[0], b[1], zb0], n, uv(uvb, v0), c);
        let i2 = self.sommet([b[0], b[1], zb1], n, uv(uvb, v1b), c);
        let i3 = self.sommet([a[0], a[1], za1], n, uv(uva, v1a), c);
        self.idx.extend_from_slice(&[i0, i1, i2, i0, i2, i3]);
    }
    pub fn ajouter(&mut self, o: &Maillage) {
        let base = self.pos.len() as u32;
        self.pos.extend_from_slice(&o.pos);
        self.nor.extend_from_slice(&o.nor);
        self.uv.extend_from_slice(&o.uv);
        self.col.extend_from_slice(&o.col);
        self.idx.extend(o.idx.iter().map(|i| i + base));
    }
    pub fn triangles(&self) -> usize {
        self.idx.len() / 3
    }
    /// Octets au format de Bevy non compressé : position, normale (3 × f32),
    /// UV (2 × f32), couleur (4 × f32), indices u32.
    pub fn octets_gpu(&self) -> usize {
        self.pos.len() * (12 + 12 + 8 + 16) + self.idx.len() * 4
    }
    /// Octets au format compressé de Bevy 0.20 (`MeshCompressionArgs::regular`) :
    /// position Snorm16 × 4, normale octaèdre Snorm16 × 2, UV Unorm16 × 2,
    /// couleur Unorm8 × 4 ; indices u16 si possible.
    pub fn octets_gpu_compresses(&self) -> usize {
        self.pos.len() * (8 + 4 + 4 + 4) + self.idx.len() * if self.pos.len() <= 65_536 { 2 } else { 4 }
    }
    /// Octets canoniques (petit-boutien) : base de l'empreinte et du stockage brut.
    pub fn octets(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(self.pos.len() * 36 + self.idx.len() * 4 + 8);
        o.extend_from_slice(&(self.pos.len() as u32).to_le_bytes());
        o.extend_from_slice(&(self.idx.len() as u32).to_le_bytes());
        for k in 0..self.pos.len() {
            for v in self.pos[k] {
                o.extend_from_slice(&v.to_le_bytes());
            }
            for v in self.nor[k] {
                o.extend_from_slice(&v.to_le_bytes());
            }
            for v in self.uv[k] {
                o.extend_from_slice(&v.to_le_bytes());
            }
            o.extend_from_slice(&self.col[k]);
        }
        for i in &self.idx {
            o.extend_from_slice(&i.to_le_bytes());
        }
        o
    }
    pub fn empreinte(&self) -> blake3::Hash {
        blake3::hash(&self.octets())
    }
}

// ------------------------------------------------------------ Génération

/// Porte choisie par `porte.rs` : point au pied du mur, normale sortante.
#[derive(Clone, Copy, Debug)]
pub struct Porte {
    pub p: P2,
    pub n: P2,
}

pub struct Entree<'a> {
    pub b: &'a Bat,
    pub m: &'a Modele,
    /// Altitude du sol (m) au point local, si le relief est connu.
    pub sol: &'a dyn Fn(f64, f64) -> Option<f64>,
    pub porte: Option<Porte>,
    /// Cases de 2 m percées (P-07) dont le centre est dans l'emprise, en
    /// cases globales depuis le coin sud-ouest de la tile propriétaire.
    pub porches: &'a [(i32, i32)],
    pub etat: Etat,
    pub lod: Lod,
    pub p: &'a Params,
}

#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub aire_emprise: f64,
    pub aire_toit: f64,
    pub echecs_toit: u32,
    pub fenetres: u32,
    pub porche: bool,
    pub cases_porche: u32,
    pub cases_gravats: u32,
    pub z_bas: f64,
    pub z_rdc: f64,
    pub z_gout: f64,
    /// Dénivelé du sol sous l'emprise (m).
    pub denivele: f64,
}

pub struct Sortie {
    pub mesh: Maillage,
    pub stats: Stats,
    /// Effondré : cases de 2 m couvertes par les gravats (cases globales).
    pub gravats: Vec<(i32, i32)>,
}

/// Anneaux d'un polygone, orientés (extérieur direct, trous indirects), en m.
pub fn anneaux_m(poly: &[Vec<[i32; 2]>], simplifier: Option<i64>) -> Vec<Vec<P2>> {
    let mut out = Vec::new();
    for (k, a) in poly.iter().enumerate() {
        let a = match simplifier {
            Some(t) => geom::simplifier(a, t),
            None => a.clone(),
        };
        if a.len() < 3 {
            if k == 0 {
                return Vec::new();
            }
            continue;
        }
        let s = geom::aire2_i(&a);
        if s == 0 {
            if k == 0 {
                return Vec::new();
            }
            continue;
        }
        let direct = s > 0;
        let mut m: Vec<P2> = a.iter().map(|p| geom::m(*p)).collect();
        if direct != (k == 0) {
            m.reverse();
        }
        out.push(m);
    }
    out
}

fn assombrir(c: [u8; 3], f: f64) -> [u8; 3] {
    [(c[0] as f64 * f) as u8, (c[1] as f64 * f) as u8, (c[2] as f64 * f) as u8]
}

/// Murs d'un ensemble d'anneaux, entre zbas(p) et zhaut(p), avec insertion
/// des points de pli du toit. `fac` : façade (fenêtres) au-dessus de z_rdc.
#[allow(clippy::too_many_arguments)]
fn murs(
    me: &mut Maillage,
    anneaux: &[Vec<P2>],
    toit: Option<&Toit>,
    zbas: f64,
    zhaut: &dyn Fn(P2) -> f64,
    z_rdc: f64,
    h_etage: f64,
    p: &Params,
    couleur: [u8; 3],
    fen: bool,
    fenetres: &mut u32,
    compter: bool,
) {
    for (ka, a) in anneaux.iter().enumerate() {
        let n = a.len();
        for i in 0..n {
            let (pa, pb) = (a[i], a[(i + 1) % n]);
            let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-4 {
                continue;
            }
            let nrm = [dy / len, -dx / len, 0.0];
            let travees = ((len / p.travee).round()).max(1.0);
            if compter && fen && ka == 0 && len >= 2.0 {
                *fenetres += travees as u32 * ((zhaut(pa).min(zhaut(pb)) - z_rdc) / h_etage).round().max(1.0) as u32;
            }
            // Points de l'arête : extrémités et plis du toit.
            let mut ts: Vec<(f64, P2)> = vec![(0.0, pa), (1.0, pb)];
            if let Some(t) = toit {
                for d in &t.plis {
                    let (fa, fb) = (d.f(&t.o, pa), d.f(&t.o, pb));
                    if (fa > 0.0 && fb < 0.0) || (fa < 0.0 && fb > 0.0) {
                        ts.push((fa / (fa - fb), geom::coupe_canon(pa, pb, d, &t.o)));
                    }
                }
                ts.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
            }
            for w in ts.windows(2) {
                let ((ta, qa), (tb, qb)) = (w[0], w[1]);
                if tb - ta < 1e-9 {
                    continue;
                }
                let (ha, hb) = (zhaut(qa), zhaut(qb));
                let ua = [(travees * ta) as f32, 0.0];
                let ub = [(travees * tb) as f32, 0.0];
                let v = |z: f64| ((z - z_rdc) / h_etage) as f32;
                // Soubassement, puis façade.
                if zbas < z_rdc {
                    me.mur(qa, qb, zbas, zbas, z_rdc.min(ha), z_rdc.min(hb), nrm, UV_NU, UV_NU, 0.0, 0.0, 0.0, assombrir(couleur, 0.72), false);
                }
                let z0 = zbas.max(z_rdc);
                me.mur(qa, qb, z0, z0, ha, hb, nrm, ua, ub, v(z0), v(ha), v(hb), couleur, fen);
            }
        }
    }
}

/// Toit d'un polygone : l'emprise réelle est triangulée une seule fois
/// (earcut), puis chaque triangle est coupé le long des plis ; chaque
/// morceau tombe dans une seule cellule, où le toit est plan.
fn toit_maillage(me: &mut Maillage, anneaux: &[Vec<P2>], t: &Toit, couleur: [u8; 3], st: &mut Stats) {
    let Some((pts, idx)) = geom::trianguler(anneaux) else {
        st.echecs_toit += 1;
        return;
    };
    let mut tris: Vec<[P2; 3]> = idx.chunks_exact(3).map(|c| [pts[c[0] as usize], pts[c[1] as usize], pts[c[2] as usize]]).collect();
    for d in &t.plis {
        let mut out = Vec::with_capacity(tris.len() + 8);
        for tr in tris {
            let f = [d.f(&t.o, tr[0]), d.f(&t.o, tr[1]), d.f(&t.o, tr[2])];
            let sg = |x: f64| if x > 0.0 { 1 } else if x < 0.0 { -1 } else { 0 };
            let s = [sg(f[0]), sg(f[1]), sg(f[2])];
            let (np, nn) = (s.iter().filter(|x| **x > 0).count(), s.iter().filter(|x| **x < 0).count());
            if np == 0 || nn == 0 {
                out.push(tr);
                continue;
            }
            let cp = |i: usize, j: usize| geom::coupe_canon(tr[i], tr[j], d, &t.o);
            if let Some(k0) = (0..3).find(|&k| s[k] == 0) {
                // La droite passe par un sommet et coupe l'arête opposée.
                let (k1, k2) = ((k0 + 1) % 3, (k0 + 2) % 3);
                let x = cp(k1, k2);
                out.push([tr[k0], tr[k1], x]);
                out.push([tr[k0], x, tr[k2]]);
            } else {
                // Sommet seul de son côté.
                let k = (0..3).find(|&k| s[k] != s[(k + 1) % 3] && s[k] != s[(k + 2) % 3]).unwrap();
                let (k1, k2) = ((k + 1) % 3, (k + 2) % 3);
                let (x1, x2) = (cp(k, k1), cp(k2, k));
                out.push([tr[k], x1, x2]);
                out.push([x1, tr[k1], tr[k2]]);
                out.push([x1, tr[k2], x2]);
            }
        }
        tris = out;
    }
    let mut vus: BTreeMap<(u64, u64, usize), u32> = BTreeMap::new();
    for tr in &tris {
        let a2 = (tr[1][0] - tr[0][0]) * (tr[2][1] - tr[0][1]) - (tr[1][1] - tr[0][1]) * (tr[2][0] - tr[0][0]);
        if a2 <= 0.0 {
            continue;
        }
        st.aire_toit += a2 / 2.0;
        let c = [(tr[0][0] + tr[1][0] + tr[2][0]) / 3.0, (tr[0][1] + tr[1][1] + tr[2][1]) / 3.0];
        let k = t.cellules.iter().position(|cel| cel.iter().all(|d| d.f(&t.o, c) >= 0.0)).unwrap_or(0);
        let n = t.normale(k);
        let mut ix = [0u32; 3];
        for (i, q) in tr.iter().enumerate() {
            let cle = (q[0].to_bits(), q[1].to_bits(), k);
            ix[i] = *vus.entry(cle).or_insert_with(|| me.sommet([q[0], q[1], t.z(*q)], n, UV_NU, couleur));
        }
        me.idx.extend_from_slice(&ix);
    }
}

/// Opération booléenne entière (i_overlay) : emprise et carrés des cases.
fn booleen(poly: &[Vec<[i32; 2]>], cases: &[(i32, i32)], regle: i_overlay::core::overlay_rule::OverlayRule) -> Vec<Vec<Vec<[i32; 2]>>> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::single::SingleIntOverlay;
    use i_overlay::i_float::int::point::IntPoint;
    let subj: Vec<Vec<IntPoint>> = poly.iter().map(|a| a.iter().map(|p| IntPoint::new(p[0], p[1])).collect()).collect();
    let clip: Vec<Vec<IntPoint>> = cases
        .iter()
        .map(|&(gi, gj)| {
            let (x0, y0) = (32 * gi - 8000, 32 * gj - 8000);
            vec![IntPoint::new(x0, y0), IntPoint::new(x0 + 32, y0), IntPoint::new(x0 + 32, y0 + 32), IntPoint::new(x0, y0 + 32)]
        })
        .collect();
    let r = subj.overlay(&clip, regle, FillRule::EvenOdd);
    r.into_iter().map(|s| s.into_iter().map(|c| c.into_iter().map(|p| [p.x, p.y]).collect()).collect()).collect()
}

/// Génère un bâtiment.
pub fn generer(e: &Entree) -> Sortie {
    let (b, m, p) = (e.b, e.m, e.p);
    let mut me = Maillage::default();
    let mut st = Stats { aire_emprise: b.aire(), ..Default::default() };
    let g = hacher(b.id);
    // Sol sous l'emprise : sommets extérieurs.
    let (mut zmin, mut zmax) = (f64::MAX, f64::MIN);
    for poly in &b.polys {
        for q in &poly[0] {
            let z = (e.sol)(q[0] as f64 / 16.0, q[1] as f64 / 16.0).unwrap_or(0.0);
            zmin = zmin.min(z);
            zmax = zmax.max(z);
        }
    }
    let z_bas = zmin - p.enfouissement;
    // Rez-de-chaussée au sol le plus haut, mais soubassement d'au plus
    // `socle_max` : sur une forte pente, la façade s'enfonce dans le terrain
    // côté amont au lieu d'un mur aveugle (Mont-Saint-Michel : 40 m).
    let z_rdc = zmin + (zmax - zmin).min(p.socle_max);
    let z_gout = (zmin + m.h).max(z_rdc + m.h.min(2.0));
    // Rangées de fenêtres : les niveaux, mais au moins une tous les 4,5 m
    // (une halle d'un seul niveau n'a pas de fenêtres de 10 m de haut).
    let rangees = (m.niveaux as f64).max(((z_gout - z_rdc) / 4.5).ceil()).max(1.0);
    let h_etage = ((z_gout - z_rdc) / rangees).max(1.0);
    (st.z_bas, st.z_rdc, st.z_gout, st.denivele) = (z_bas, z_rdc, z_gout, zmax - zmin);
    let degat = if e.etat == Etat::Endommage { 0.5 } else { 1.0 };
    let (cmur, ctoit) = (assombrir(m.mur, degat), assombrir(m.toit, degat));

    if e.etat == Etat::Effondre {
        let gravats = gravats(&mut me, b, m, e, z_rdc, g);
        st.cases_gravats = gravats.len() as u32;
        return Sortie { mesh: me, stats: st, gravats };
    }

    let tol = (p.tol_loin * 16.0) as i64;
    for poly in &b.polys {
        let simpl = if e.lod == Lod::Iso { None } else { Some(tol) };
        let an = anneaux_m(poly, simpl);
        if an.is_empty() {
            continue;
        }
        if e.lod == Lod::Carte {
            let t = Toit::new(geom::obb(&poly[0]), Forme::Plat, 0.0, 0.0, p);
            toit_maillage(&mut me, &an, &t, ctoit, &mut st);
            continue;
        }
        let base = if e.lod == Lod::Iso { poly.clone() } else { poly.iter().map(|a| geom::simplifier(a, tol)).filter(|a| a.len() >= 3).collect() };
        if base.is_empty() {
            continue;
        }
        let t = Toit::new(geom::obb(&base[0]), m.forme, z_gout, m.dz, p);
        let zt = |q: P2| t.z(q);
        // Porche : cases percées de ce polygone.
        let porche: Vec<(i32, i32)> = if e.lod == Lod::Iso {
            e.porches.iter().copied().filter(|&(gi, gj)| geom::dedans([gi as f64 * 2.0 - 499.0, gj as f64 * 2.0 - 499.0], &an)).collect()
        } else {
            Vec::new()
        };
        let fen = m.fenetres && e.lod == Lod::Iso;
        if porche.is_empty() {
            murs(&mut me, &an, Some(&t), if e.lod == Lod::Iso { z_bas } else { z_rdc.min(z_bas) }, &zt, if e.lod == Lod::Iso { z_rdc } else { z_bas }, h_etage, p, cmur, fen, &mut st.fenetres, true);
        } else {
            use i_overlay::core::overlay_rule::OverlayRule;
            st.porche = true;
            st.cases_porche += porche.len() as u32;
            let z_p = z_rdc + p.porche.min(0.6 * (z_gout - z_rdc));
            // Haut : emprise entière, du plafond du porche au toit.
            murs(&mut me, &an, Some(&t), z_p, &zt, z_rdc, h_etage, p, cmur, fen, &mut st.fenetres, true);
            // Bas : emprise moins les cases percées, jusqu'au plafond.
            for s in booleen(poly, &porche, OverlayRule::Difference) {
                let an_bas = anneaux_m(&s, None);
                if !an_bas.is_empty() {
                    murs(&mut me, &an_bas, None, z_bas, &|_| z_p, z_rdc, h_etage, p, cmur, fen, &mut st.fenetres, false);
                }
            }
            // Plafond du passage, tourné vers le bas.
            for s in booleen(poly, &porche, OverlayRule::Intersect) {
                let an_p = anneaux_m(&s, None);
                if let Some((pts, idx)) = (!an_p.is_empty()).then(|| geom::trianguler(&an_p)).flatten() {
                    let base = me.pos.len() as u32;
                    for q in &pts {
                        me.sommet([q[0], q[1], z_p], [0.0, 0.0, -1.0], UV_NU, [62, 58, 54]);
                    }
                    for tri in idx.chunks_exact(3) {
                        me.idx.extend_from_slice(&[base + tri[0], base + tri[2], base + tri[1]]);
                    }
                }
            }
        }
        toit_maillage(&mut me, &an, &t, ctoit, &mut st);
    }
    // Porte : panneau sombre de 1,2 × 2,2 m, décollé de 3 cm du mur.
    if let (Some(po), Lod::Iso) = (e.porte, e.lod) {
        let tg = [-po.n[1], po.n[0]];
        let c = [po.p[0] + po.n[0] * 0.03, po.p[1] + po.n[1] * 0.03];
        let a = [c[0] - tg[0] * 0.6, c[1] - tg[1] * 0.6];
        let bb = [c[0] + tg[0] * 0.6, c[1] + tg[1] * 0.6];
        let zs = (e.sol)(po.p[0], po.p[1]).unwrap_or(0.0).max(z_bas);
        me.mur(a, bb, zs, zs, zs + 2.2, zs + 2.2, [po.n[0], po.n[1], 0.0], UV_NU, UV_NU, 0.0, 0.0, 0.0, [70, 45, 30], false);
    }
    Sortie { mesh: me, stats: st, gravats: Vec::new() }
}

/// Gravats d'un bâtiment effondré : les cases de 2 m de l'emprise, dilatées
/// selon la hauteur, chacune un bloc de hauteur tirée de la graine. Les
/// mêmes cases deviennent infranchissables pour les humains et les véhicules.
fn gravats(me: &mut Maillage, b: &Bat, m: &Modele, e: &Entree, z_rdc: f64, g: u64) -> Vec<(i32, i32)> {
    let bb = b.bbox();
    let d = ((0.3 * m.h).min(6.0) / 2.0).ceil() as i32;
    let (gi0, gj0) = ((bb[0] + 8000).div_euclid(32) - d - 1, (bb[1] + 8000).div_euclid(32) - d - 1);
    let (gi1, gj1) = ((bb[2] + 8000).div_euclid(32) + d + 1, (bb[3] + 8000).div_euclid(32) + d + 1);
    let (nx, ny) = ((gi1 - gi0 + 1) as usize, (gj1 - gj0 + 1) as usize);
    let w = crate::raster::Fenetre { x0: gi0 as f64 * 2.0 - 500.0, y0: gj0 as f64 * 2.0 - 500.0, res: 2.0, nx, ny };
    let mut dist = vec![i32::MAX; nx * ny];
    for poly in &b.polys {
        let an: Vec<Vec<P2>> = poly.iter().map(|a| a.iter().map(|q| geom::m(*q)).collect()).collect();
        crate::raster::polygone(w, &an, |i, j| dist[j * nx + i] = 0);
    }
    // Dilatation (distance de Tchebychev, deux passes).
    for _ in 0..d {
        let prev = dist.clone();
        for j in 0..ny {
            for i in 0..nx {
                if prev[j * nx + i] != i32::MAX {
                    continue;
                }
                let mut best = i32::MAX;
                for (di, dj) in [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    let (ii, jj) = (i as i32 + di, j as i32 + dj);
                    if ii >= 0 && jj >= 0 && (ii as usize) < nx && (jj as usize) < ny {
                        best = best.min(prev[jj as usize * nx + ii as usize]);
                    }
                }
                if best != i32::MAX {
                    dist[j * nx + i] = best + 1;
                }
            }
        }
    }
    let hmax = (1.0 + 0.12 * m.h).min(5.0);
    let mut cases = Vec::new();
    let teinte = assombrir(m.mur, 0.6);
    for j in 0..ny {
        for i in 0..nx {
            let k = dist[j * nx + i];
            if k == i32::MAX {
                continue;
            }
            let (gi, gj) = (gi0 + i as i32, gj0 + j as i32);
            cases.push((gi, gj));
            let h = hacher(g ^ ((gi as u64) << 32 | gj as u32 as u64));
            let f = 1.0 - k as f64 / (d as f64 + 1.0);
            let hz = (0.3 + (h % 1000) as f64 / 1000.0 * hmax * f).max(0.2);
            let (x0, y0) = (gi as f64 * 2.0 - 500.0, gj as f64 * 2.0 - 500.0);
            let zs = (e.sol)(x0 + 1.0, y0 + 1.0).unwrap_or(z_rdc) - 0.2;
            let zt = zs + 0.2 + hz;
            let c = [[x0, y0], [x0 + 2.0, y0], [x0 + 2.0, y0 + 2.0], [x0, y0 + 2.0]];
            for s in 0..4 {
                let (a, q) = (c[s], c[(s + 1) % 4]);
                let (dx, dy) = ((q[0] - a[0]) / 2.0, (q[1] - a[1]) / 2.0);
                me.mur(a, q, zs, zs, zt, zt, [dy, -dx, 0.0], UV_NU, UV_NU, 0.0, 0.0, 0.0, teinte, false);
            }
            let base = me.pos.len() as u32;
            for q in &c {
                me.sommet([q[0], q[1], zt], [0.0, 0.0, 1.0], UV_NU, teinte);
            }
            me.idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    cases
}
