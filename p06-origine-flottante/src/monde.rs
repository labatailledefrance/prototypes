//! Le monde du prototype : ancres des modèles, stratégies de rendu relatif à la
//! caméra, horloge, scènes et relecture de la projection.
//!
//! Chaque modèle a une **ancre** : une tile et une position locale en f32
//! (solution C de la simulation), ou une position ECEF en f64 (navire en mer,
//! globe). La caméra a une pose ECEF en f64. Une **stratégie** transforme ces
//! poses en `Transform` f32 pour Bevy ; Bevy propage ; la relecture rejoue la
//! projection du GPU en f32 sur les `GlobalTransform` réellement calculés et la
//! compare à une projection de référence en f64.

use std::collections::HashMap;
use std::time::Instant;

use bevy::camera::{CameraProjection, PerspectiveProjection};
use bevy::ecs::batching::BatchingStrategy;
use bevy::math::{DMat3, DQuat, DVec2, DVec3};
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use big_space::prelude::*;

use crate::geo::{self, Geo, RepereTile, TileId, Zone};
use crate::stats::{self, Resume};

pub const LARGEUR: f32 = 1920.0;
pub const HAUTEUR: f32 = 1080.0;
pub const FOV_DEG: f64 = 30.0;
pub const PRES: f32 = 0.1;
pub const TICK_HZ: f64 = 20.0;

// ------------------------------------------------------------------ Stratégies

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Strategie {
    /// Témoin : monde f32 dont l'origine est fixée à Paris (axes locaux).
    NaifParis,
    /// Témoin : monde f32 dont l'origine est le centre de la Terre.
    NaifEcef,
    /// Origine recalée sur la caméra à chaque frame.
    ChaqueFrame,
    /// Origine recalée quand la caméra s'en éloigne de plus du seuil (m).
    Seuil(f64),
    /// Origine fixée à une distance donnée de la caméra (pire cas du seuil).
    Decalage(f64),
    /// Une entité parente par tile ; recalage des seuls parents, à chaque frame.
    Tiles,
    /// Parents de tiles recalés au-delà du seuil.
    TilesSeuil(f64),
    /// Crate `big_space`, cellules de l'arête donnée (m).
    BigSpace(f64),
}

impl Strategie {
    pub fn lire(s: &str) -> Strategie {
        let (n, v) = match s.split_once(':') {
            Some((n, v)) => (n, v.parse::<f64>().expect("valeur de stratégie")),
            None => (s, 1000.0),
        };
        match n {
            "naif-paris" => Strategie::NaifParis,
            "naif-ecef" => Strategie::NaifEcef,
            "chaque-frame" => Strategie::ChaqueFrame,
            "seuil" => Strategie::Seuil(v),
            "decalage" => Strategie::Decalage(v),
            "tiles" => Strategie::Tiles,
            "tiles-seuil" => Strategie::TilesSeuil(v),
            "big-space" => Strategie::BigSpace(v),
            _ => panic!("stratégie inconnue : {s}"),
        }
    }
    pub fn nom(&self) -> String {
        match self {
            Strategie::NaifParis => "naif-paris".into(),
            Strategie::NaifEcef => "naif-ecef".into(),
            Strategie::ChaqueFrame => "chaque-frame".into(),
            Strategie::Seuil(s) => format!("seuil:{s}"),
            Strategie::Decalage(s) => format!("decalage:{s}"),
            Strategie::Tiles => "tiles".into(),
            Strategie::TilesSeuil(s) => format!("tiles-seuil:{s}"),
            Strategie::BigSpace(s) => format!("big-space:{s}"),
        }
    }
    pub fn hierarchie(&self) -> bool {
        matches!(self, Strategie::Tiles | Strategie::TilesSeuil(_))
    }
    pub fn big_space(&self) -> bool {
        matches!(self, Strategie::BigSpace(_))
    }
}

#[derive(Resource, Clone)]
pub struct Reglage {
    pub strat: Strategie,
    pub lot: usize,
}

// ------------------------------------------------------------------ Tiles et ancres

#[derive(Resource, Default)]
pub struct Tiles {
    pub reperes: HashMap<TileId, RepereTile>,
    pub zones: HashMap<u32, Zone>,
}

impl Tiles {
    pub fn assurer(&mut self, id: TileId) -> RepereTile {
        if let Some(r) = self.reperes.get(&id) {
            return *r;
        }
        let z = *self.zones.entry(id.epsg).or_insert_with(|| geo::zone_par_epsg(id.epsg));
        let r = geo::repere_tile(&z, id);
        self.reperes.insert(id, r);
        r
    }
    pub fn lire(&self, id: &TileId) -> &RepereTile {
        self.reperes.get(id).expect("tile non enregistrée")
    }
}

/// Où se trouve un modèle.
#[derive(Component, Clone, Copy, Debug)]
pub enum Ancre {
    /// Dans une tile : position locale f32 (x = est, y = haut, z = −nord, depuis
    /// le coin sud-ouest), rotation locale.
    Tile { id: TileId, local: Vec3, rot: Quat },
    /// Hors des tiles (navire en mer, globe, caméra) : ECEF f64.
    Global { ecef: DVec3, rot: DQuat },
}

/// Pose ECEF f64 de référence (position, rotation modèle → ECEF).
pub fn pose(a: &Ancre, tiles: &Tiles) -> (DVec3, DQuat) {
    match a {
        Ancre::Tile { id, local, rot } => {
            let r = tiles.lire(id);
            (r.vers_ecef(local.as_dvec3()), r.rotation * rot.as_dquat())
        }
        Ancre::Global { ecef, rot } => (*ecef, *rot),
    }
}

/// Parent d'une tile (stratégies hiérarchiques).
#[derive(Component)]
pub struct ParentTile(pub TileId);

#[derive(Resource, Default)]
pub struct ParentsTiles(pub HashMap<TileId, Entity>);

/// Points mesurés d'un modèle, en coordonnées du modèle.
#[derive(Component)]
pub struct Sonde {
    pub nom: &'static str,
    pub points: Vec<Vec3>,
}

/// Le modèle bouge à chaque frame (soldat, navire).
#[derive(Component)]
pub struct Mobile;

#[derive(Component)]
pub struct CamRig;

/// Ce qu'il faut dessiner (rempli par le mode fenêtré).
#[derive(Component, Clone, Debug)]
pub enum Visuel {
    Soldat,
    Sol,
    Bloc(Vec3),
    Coque,
    Cube,
    Globe(Handle<Mesh>),
    Plaque(Vec2, Color),
}

// ------------------------------------------------------------------ Origine et caméra

#[derive(Resource)]
pub struct Origine {
    pub ecef: DVec3,
    /// Rotation « axes Bevy depuis ECEF ».
    pub rot: DMat3,
    pub qrot: DQuat,
    pub change: bool,
    pub init: bool,
    pub recalages: u64,
}

impl Default for Origine {
    fn default() -> Self {
        let rot = geo::bevy_depuis_ecef_fixe();
        Origine { ecef: DVec3::ZERO, rot, qrot: DQuat::from_mat3(&rot), change: true, init: false, recalages: 0 }
    }
}

impl Origine {
    fn poser(&mut self, ecef: DVec3, rot: DMat3) {
        self.ecef = ecef;
        self.rot = rot;
        self.qrot = DQuat::from_mat3(&rot);
        self.change = true;
        self.init = true;
        self.recalages += 1;
    }
    pub fn vers_rendu(&self, p: DVec3) -> DVec3 {
        self.rot * (p - self.ecef)
    }
}

/// Pose f64 de la caméra pour cette frame.
#[derive(Resource, Clone, Copy)]
pub struct CameraJeu {
    pub pos: DVec3,
    pub rot: DQuat,
    pub fov: f64,
}

impl Default for CameraJeu {
    fn default() -> Self {
        CameraJeu { pos: DVec3::new(geo::A, 0.0, 0.0), rot: DQuat::IDENTITY, fov: FOV_DEG.to_radians() }
    }
}

pub fn projection32(fov: f64) -> PerspectiveProjection {
    PerspectiveProjection { fov: fov as f32, aspect_ratio: LARGEUR / HAUTEUR, near: PRES, ..default() }
}

/// Caméra « isométrique » : distance d, azimut et élévation dans le repère
/// local de la cible.
pub fn rig_iso(cible: DVec3, d: f64, az_deg: f64, el_deg: f64) -> (DVec3, DQuat) {
    let g = geo::depuis_ecef(cible);
    let up = geo::verticale(g);
    let b = geo::bevy_depuis_ecef(g).transpose(); // local → ECEF (colonnes est, haut, sud)
    let (sa, ca) = az_deg.to_radians().sin_cos();
    let (se, ce) = el_deg.to_radians().sin_cos();
    // Azimut depuis le nord, vers l'est ; la caméra est « derrière » la cible.
    let est = b.col(0);
    let nord = -b.col(2);
    let horiz = -(nord * ca + est * sa);
    let pos = cible + d * (horiz * ce + up * se);
    (pos, regarder(pos, cible, up))
}

/// Rotation caméra → ECEF qui regarde `cible` (conventions Bevy : −Z devant).
pub fn regarder(pos: DVec3, cible: DVec3, up: DVec3) -> DQuat {
    let f = (cible - pos).normalize();
    let r = f.cross(up).normalize();
    let u = r.cross(f);
    DQuat::from_mat3(&DMat3::from_cols(r, u, -f))
}

// ------------------------------------------------------------------ Horloge

/// Horloge de la simulation : ticks entiers à 20 Hz, fraction pour
/// l'interpolation du rendu. Aucune horloge flottante cumulée.
#[derive(Resource, Clone, Copy)]
pub struct Horloge {
    pub t0: f64,
    pub tick: u64,
    pub alpha: f64,
    pub acceleration: f64,
    /// Durée réelle d'une frame (s) en mode relecture.
    pub dt_frame: f64,
    pub pause: bool,
}

impl Horloge {
    pub fn temps(&self) -> f64 {
        self.t0 + (self.tick as f64 + self.alpha) / TICK_HZ
    }
}

// ------------------------------------------------------------------ Scènes

#[derive(Clone, Debug)]
pub enum Scenario {
    /// Soldat qui marche, caméra iso à `distance`.
    Soldat { lieu: String, distance: f64 },
    /// Navire de Brest à Tahiti par Panama, puis vers Nouméa.
    Navire,
    /// Zoom continu du soldat au globe et retour.
    Zoom { lieu: String, frames: u32 },
    /// Coût : n modèles statiques, une part de mobiles, caméra qui avance.
    Cout { n: usize, mobiles: f64, vitesse: f64 },
    /// Caméra seule, pilotée de l'extérieur (`CameraJeu`).
    Vide,
}

#[derive(Resource)]
pub struct Scene(pub Scenario);

/// Simulation du soldat : solution C (tile + position locale f32), 20 Hz.
#[derive(Resource, Clone, Copy)]
pub struct Soldat {
    pub tile: TileId,
    pub pos: Vec2,
    pub prec: Vec2,
    pub prec_tile: TileId,
    pub vitesse: f32,
}

impl Soldat {
    pub fn pas(&mut self) {
        self.prec = self.pos;
        self.prec_tile = self.tile;
        self.pos.x += self.vitesse / TICK_HZ as f32;
        // Passage de tile : soustraction exacte (lemme de Sterbenz).
        if self.pos.x >= geo::TILE as f32 {
            self.pos.x -= geo::TILE as f32;
            self.tile = self.tile.voisine(1, 0);
        }
    }
    /// Position interpolée, locale à la tile courante.
    pub fn interpole(&self, alpha: f32) -> Vec2 {
        let mut p = self.prec;
        if self.prec_tile != self.tile {
            p.x -= geo::TILE as f32 * (self.tile.ix - self.prec_tile.ix) as f32;
        }
        p + (self.pos - p) * alpha
    }
}

/// Itinéraire du navire : polyligne dense sur la sphère des normales.
#[derive(Resource)]
pub struct Navire {
    pub points: Vec<DVec3>,
    pub cumul: Vec<f64>,
    pub s: f64,
    pub vitesse: f64,
    /// Ports : (normale, zone, rayon où le navire est dans les tiles).
    pub ports: Vec<(DVec3, u32)>,
    pub rayon_port: f64,
    pub en_tile: bool,
    pub sauts: Vec<SautNavire>,
}

#[derive(Clone, Copy, Debug)]
pub struct SautNavire {
    pub temps: f64,
    pub vers_tile: bool,
    pub saut_m: f64,
    pub saut_px: f64,
}

pub fn route_navire() -> Vec<Geo> {
    use geo::geo as g;
    vec![
        g(48.3800, -4.4900),  // Brest
        g(48.3000, -5.2000),  // Iroise
        g(46.0000, -10.000),
        g(25.0000, -50.000),
        g(16.0000, -61.200),  // passage de la Dominique
        g(12.0000, -75.000),
        g(9.3800, -79.9200),  // Colón
        g(9.1500, -79.8500),  // lac Gatún (canal non simulé)
        g(8.8800, -79.5200),  // Balboa
        g(-5.0000, -110.00),
        g(-17.4500, -149.55), // devant Papeete
        g(-17.5350, -149.5696), // Papeete
        g(-17.4000, -149.70),
        g(-19.0000, -170.00),
        g(-21.0000, 178.000), // antiméridien
        g(-22.3500, 166.300),
        g(-22.2758, 166.4580), // Nouméa
    ]
}

impl Navire {
    pub fn nouveau(vitesse: f64) -> Navire {
        let route = route_navire();
        let mut points = vec![];
        for w in route.windows(2) {
            let (a, b) = (geo::verticale(w[0]), geo::verticale(w[1]));
            let ang = a.angle_between(b);
            let n = ((ang * 6_371_000.0 / 5_000.0).ceil() as usize).max(1);
            for i in 0..n {
                points.push(slerp(a, b, i as f64 / n as f64));
            }
        }
        points.push(geo::verticale(*route.last().unwrap()));
        let mut cumul = vec![0.0];
        for w in points.windows(2) {
            let d = (geo::vers_ecef(normale_geo(w[1])) - geo::vers_ecef(normale_geo(w[0]))).length();
            cumul.push(cumul.last().unwrap() + d);
        }
        let ports = vec![
            (geo::verticale(geo::lieu("Brest").g), 2154),
            (geo::verticale(geo::lieu("Papeete").g), 3297),
            (geo::verticale(geo::lieu("Nouméa").g), 3163),
        ];
        Navire { points, cumul, s: 0.0, vitesse, ports, rayon_port: 30_000.0, en_tile: true, sauts: vec![] }
    }
    pub fn longueur(&self) -> f64 {
        *self.cumul.last().unwrap()
    }
    /// Normale et cap (unitaires) à l'abscisse s.
    pub fn etat(&self, s: f64) -> (DVec3, DVec3) {
        let s = s.clamp(0.0, self.longueur() - 1.0);
        let i = match self.cumul.binary_search_by(|c| c.partial_cmp(&s).unwrap()) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
        .min(self.points.len() - 2);
        let t = (s - self.cumul[i]) / (self.cumul[i + 1] - self.cumul[i]);
        let n = slerp(self.points[i], self.points[i + 1], t);
        let cap = (self.points[i + 1] - self.points[i]).reject_from(n).normalize();
        (n, cap)
    }
}

pub fn slerp(a: DVec3, b: DVec3, t: f64) -> DVec3 {
    let ang = a.angle_between(b);
    if ang < 1e-12 {
        return a;
    }
    ((a * ((1.0 - t) * ang).sin() + b * (t * ang).sin()) / ang.sin()).normalize()
}

pub fn normale_geo(n: DVec3) -> Geo {
    Geo { lat: n.z.clamp(-1.0, 1.0).asin().to_degrees(), lon: n.y.atan2(n.x).to_degrees(), h: 0.0 }
}

// ------------------------------------------------------------------ Globe

/// Découpage du globe en morceaux (cube projeté sur l'ellipsoïde) : chaque
/// morceau a son origine locale, ses sommets sont relatifs à son centre.
pub const GLOBE_N: usize = 16;
pub const GLOBE_M: usize = 16;
pub const GLOBE_H: f64 = -2.0;

pub fn face_dir(face: usize, u: f64, v: f64) -> DVec3 {
    let d = match face {
        0 => DVec3::new(1.0, u, v),
        1 => DVec3::new(-1.0, -u, v),
        2 => DVec3::new(-u, 1.0, v),
        3 => DVec3::new(u, -1.0, v),
        4 => DVec3::new(-v, u, 1.0),
        _ => DVec3::new(v, u, -1.0),
    };
    d.normalize()
}

pub fn dir_face(d: DVec3) -> (usize, f64, f64) {
    let a = d.abs();
    if a.x >= a.y && a.x >= a.z {
        if d.x > 0.0 { (0, d.y / a.x, d.z / a.x) } else { (1, -d.y / a.x, d.z / a.x) }
    } else if a.y >= a.z {
        if d.y > 0.0 { (2, -d.x / a.y, d.z / a.y) } else { (3, d.x / a.y, d.z / a.y) }
    } else if d.z > 0.0 {
        (4, d.y / a.z, -d.x / a.z)
    } else {
        (5, d.y / a.z, d.x / a.z)
    }
}

pub fn point_globe(face: usize, u: f64, v: f64) -> DVec3 {
    let mut g = normale_geo(face_dir(face, u, v));
    g.h = GLOBE_H;
    geo::vers_ecef(g)
}

/// Centre ECEF d'un morceau et sommets relatifs (f32), avec indices.
pub fn morceau_globe(face: usize, i: usize, j: usize) -> (DVec3, Vec<Vec3>, Vec<u32>) {
    let pas = 2.0 / GLOBE_N as f64;
    let (u0, v0) = (-1.0 + i as f64 * pas, -1.0 + j as f64 * pas);
    let centre = point_globe(face, u0 + pas / 2.0, v0 + pas / 2.0);
    let mut sommets = vec![];
    for b in 0..=GLOBE_M {
        for a in 0..=GLOBE_M {
            let p = point_globe(face, u0 + pas * a as f64 / GLOBE_M as f64, v0 + pas * b as f64 / GLOBE_M as f64);
            sommets.push((p - centre).as_vec3());
        }
    }
    let mut idx = vec![];
    let w = (GLOBE_M + 1) as u32;
    for b in 0..GLOBE_M as u32 {
        for a in 0..GLOBE_M as u32 {
            let k = b * w + a;
            idx.extend_from_slice(&[k, k + w, k + 1, k + 1, k + w, k + w + 1]);
        }
    }
    (centre, sommets, idx)
}

// ------------------------------------------------------------------ Relevé

#[derive(Default, Clone)]
pub struct SerieSonde {
    pub px: Vec<f64>,
    pub mm: Vec<f64>,
    pub trem: Vec<f64>,
    pub prec: Vec<Option<Vec2>>,
}

#[derive(Resource, Default)]
pub struct Releve {
    pub actif: bool,
    pub series: HashMap<&'static str, SerieSonde>,
    /// Chronologie optionnelle (une ligne par frame).
    pub chrono: Vec<String>,
    pub frame: u64,
    /// Erreur max de la frame courante (affichage).
    pub derniere_max_px: f64,
}

#[derive(Resource, Default)]
pub struct Chronos {
    pub t0: Option<Instant>,
    pub t1: Option<Instant>,
    pub recalage: Vec<f64>,
    pub propagation: Vec<f64>,
    pub frames_recalees: Vec<bool>,
    pub actif: bool,
}

// ------------------------------------------------------------------ Construction

pub struct Options {
    pub strat: Strategie,
    pub scenario: Scenario,
    pub t0: f64,
    pub acceleration: f64,
    pub lot: usize,
    pub fenetre: bool,
}

/// Ajoute au `App` (déjà muni de ses plugins de base) les ressources et les
/// systèmes du prototype.
pub fn installer(app: &mut App, o: &Options) {
    let strat = o.strat;
    app.insert_resource(Reglage { strat: o.strat, lot: o.lot })
        .insert_resource(Scene(o.scenario.clone()))
        .insert_resource(Horloge {
            t0: o.t0,
            tick: 0,
            alpha: 0.0,
            acceleration: o.acceleration,
            dt_frame: 1.0 / 60.0,
            pause: false,
        })
        .init_resource::<Tiles>()
        .init_resource::<ParentsTiles>()
        .init_resource::<Origine>()
        .init_resource::<CameraJeu>()
        .init_resource::<Releve>()
        .init_resource::<Chronos>()
        .add_systems(Startup, installer_scene)
        .add_systems(PostStartup, marquer_immobiles.run_if(move || strat.big_space()))
        .add_systems(Update, (avancer, poser_scene, maj_origine).chain());

    let post = (
        chrono_debut,
        appliquer_plat.run_if(move || !strat.big_space()),
        appliquer_tiles.run_if(move || strat.hierarchie()),
        appliquer_big.run_if(move || strat.big_space()),
        chrono_fin_recalage,
    )
        .chain()
        .before(TransformSystems::Propagate);
    app.add_systems(PostUpdate, post);
    app.add_systems(PostUpdate, (chrono_fin_propagation, mesurer).chain().after(TransformSystems::Propagate));
}

// ------------------------------------------------------------------ Mise en place

pub fn installer_scene(
    mut commands: Commands,
    scene: Res<Scene>,
    reglage: Res<Reglage>,
    mut tiles: ResMut<Tiles>,
    mut parents: ResMut<ParentsTiles>,
) {
    let strat = reglage.strat;
    // Racine big_space.
    let mut racine = None;
    if let Strategie::BigSpace(arete) = strat {
        commands.spawn_big_space(Grid::new(arete as f32, 0.0), |r| {
            racine = Some(r.id());
        });
        commands.insert_resource(RacineBig(racine.unwrap()));
    }
    let mut ctx = Fabrique { commands: &mut commands, tiles: &mut tiles, parents: &mut parents, strat, racine };

    // Caméra.
    let mut ec = ctx.creer(Ancre::Global { ecef: DVec3::new(geo::A, 0.0, 0.0), rot: DQuat::IDENTITY }, (CamRig, Mobile));
    if strat.big_space() {
        ec.insert(FloatingOrigin);
    }

    match &scene.0 {
        Scenario::Soldat { lieu, .. } | Scenario::Zoom { lieu, .. } => {
            let l = geo::lieu(lieu);
            let z = geo::zone_par_epsg(l.epsg);
            let g = z.proj.avant(l.g);
            let t0 = TileId::de_grille(l.epsg, g);
            // Le soldat part à 20 m du bord est de sa tile, au milieu.
            let s = Soldat { tile: t0, pos: Vec2::new(980.0, 500.0), prec: Vec2::new(980.0, 500.0), prec_tile: t0, vitesse: 1.4 };
            ctx.commands.insert_resource(s);
            ctx.creer(
                Ancre::Tile { id: t0, local: Vec3::new(980.0, 0.0, -500.0), rot: Quat::IDENTITY },
                (
                    Mobile,
                    Visuel::Soldat,
                    Sonde {
                        nom: "soldat",
                        points: vec![
                            Vec3::new(0.0, 2.1, 0.0),
                            Vec3::new(0.0, 1.8, 0.0),
                            Vec3::ZERO,
                            Vec3::new(0.3, 0.9, 0.0),
                            Vec3::new(-0.3, 0.9, 0.0),
                            Vec3::new(0.0, 0.9, 0.3),
                            Vec3::new(0.0, 0.9, -0.3),
                        ],
                    },
                ),
            );
            // Sol : 9 × 9 tiles ; sondes sur les sommets le long du chemin.
            for dx in -4..=4 {
                for dy in -4..=4 {
                    let id = t0.voisine(dx, dy);
                    let mut e = ctx.creer(
                        Ancre::Tile { id, local: Vec3::new(500.0, 0.0, -500.0), rot: Quat::IDENTITY },
                        (Visuel::Sol,),
                    );
                    if (dx, dy) == (0, 0) {
                        e.insert(Sonde { nom: "sol", points: (0..5).map(|k| Vec3::new(460.0 + 10.0 * k as f32, 0.0, 0.0)).collect() });
                    } else if (dx, dy) == (1, 0) {
                        e.insert(Sonde { nom: "sol-voisin", points: (0..5).map(|k| Vec3::new(-500.0 + 10.0 * k as f32, 0.0, 0.0)).collect() });
                    }
                }
            }
            // Bâtiments le long de la rue, des deux côtés, sur les deux tiles.
            let mut k = 0u64;
            for (id, x0) in [(t0, 700.0f32), (t0.voisine(1, 0), 0.0)] {
                for i in 0..12 {
                    for cote in [-1.0f32, 1.0] {
                        k += 1;
                        let r = stats::melange(k);
                        let taille = Vec3::new(8.0 + (r % 9) as f32, 6.0 + ((r >> 8) % 25) as f32, 8.0 + ((r >> 16) % 7) as f32);
                        let local = Vec3::new(x0 + 6.0 + 24.0 * i as f32, 0.0, -500.0 + cote * (6.0 + taille.z / 2.0));
                        if local.x > 1000.0 {
                            continue;
                        }
                        let mut e = ctx.creer(Ancre::Tile { id, local, rot: Quat::IDENTITY }, (Visuel::Bloc(taille),));
                        if id == t0 && i == 11 && cote > 0.0 {
                            let (hx, hy, hz) = (taille.x / 2.0, taille.y, taille.z / 2.0);
                            e.insert(Sonde {
                                nom: "batiment",
                                points: vec![
                                    Vec3::new(-hx, 0.0, -hz),
                                    Vec3::new(hx, 0.0, -hz),
                                    Vec3::new(-hx, hy, -hz),
                                    Vec3::new(hx, hy, -hz),
                                    Vec3::new(-hx, hy, hz),
                                ],
                            });
                        }
                    }
                }
            }
            if let Scenario::Zoom { .. } = scene.0 {
                // Morceau de globe sous le soldat, et globe naïf (sommets
                // relatifs au centre de la Terre).
                let r = ctx.tiles.lire(&t0);
                let sol = r.vers_ecef(DVec3::new(980.0, 0.0, -500.0));
                let (face, u, v) = dir_face(sol.normalize());
                let pas = 2.0 / GLOBE_N as f64;
                let (i, j) = (((u + 1.0) / pas).floor() as usize, ((v + 1.0) / pas).floor() as usize);
                let (centre, _, _) = morceau_globe(face, i.min(GLOBE_N - 1), j.min(GLOBE_N - 1));
                // Sommet du globe au pied du soldat, exprimé par rapport au
                // centre du morceau (globe découpé) ou au centre de la Terre (naïf).
                let sol = r.vers_ecef(DVec3::new(980.0, 0.0, -500.0));
                let proche = (sol - centre).as_vec3();
                ctx.creer(Ancre::Global { ecef: centre, rot: DQuat::IDENTITY }, (Sonde { nom: "globe-morceau", points: vec![proche] },));
                let p_naif = sol.as_vec3();
                ctx.creer(Ancre::Global { ecef: DVec3::ZERO, rot: DQuat::IDENTITY }, (Sonde { nom: "globe-naif", points: vec![p_naif] },));
            }
        }
        Scenario::Navire => {
            let nav = Navire::nouveau(8.0);
            let (n, _) = nav.etat(0.0);
            let _ = n;
            ctx.commands.insert_resource(nav);
            // Ancre provisoire (corrigée à la première frame).
            ctx.creer(
                Ancre::Global { ecef: DVec3::new(geo::A, 0.0, 0.0), rot: DQuat::IDENTITY },
                (
                    Mobile,
                    Visuel::Coque,
                    Sonde {
                        nom: "navire",
                        points: vec![Vec3::new(0.0, 6.0, -60.0), Vec3::new(0.0, 6.0, 60.0), Vec3::new(0.0, 30.0, 0.0), Vec3::new(10.0, 12.0, 0.0)],
                    },
                ),
            );
        }
        Scenario::Vide => {}
        Scenario::Cout { n, mobiles, .. } => {
            let l = geo::lieu("Brest");
            let z = geo::zone_par_epsg(l.epsg);
            let t0 = TileId::de_grille(l.epsg, z.proj.avant(l.g));
            let nm = (*n as f64 * mobiles) as usize;
            for k in 0..*n {
                let r = stats::melange(k as u64);
                let id = t0.voisine((r % 10) as i32 - 5, ((r >> 8) % 10) as i32 - 5);
                let local = Vec3::new(((r >> 16) % 1000) as f32 + 0.5, 0.0, -(((r >> 32) % 1000) as f32 + 0.5));
                let mut e = ctx.creer(Ancre::Tile { id, local, rot: Quat::IDENTITY }, (Visuel::Cube,));
                if k < nm {
                    e.insert(Mobile);
                }
            }
            ctx.commands.insert_resource(DepartCout(t0));
        }
    }
}

#[derive(Resource)]
pub struct RacineBig(pub Entity);

#[derive(Resource)]
pub struct DepartCout(pub TileId);

/// Crée les entités selon la disposition qu'impose la stratégie.
pub struct Fabrique<'a, 'w, 's> {
    pub commands: &'a mut Commands<'w, 's>,
    pub tiles: &'a mut Tiles,
    pub parents: &'a mut ParentsTiles,
    pub strat: Strategie,
    pub racine: Option<Entity>,
}

impl<'a, 'w, 's> Fabrique<'a, 'w, 's> {
    pub fn creer<B: Bundle>(&mut self, a: Ancre, b: B) -> EntityCommands<'_> {
        if let Ancre::Tile { id, .. } = a {
            self.tiles.assurer(id);
        }
        let e = self.commands.spawn((a, Transform::default(), b)).id();
        if let Some(r) = self.racine {
            self.commands.entity(e).insert((CellCoord::default(), ChildOf(r)));
        } else if let (true, Ancre::Tile { id, .. }) = (self.strat.hierarchie(), a) {
            let p = parent_de(self.commands, self.parents, id);
            self.commands.entity(e).insert(ChildOf(p));
        }
        self.commands.entity(e)
    }
}

pub fn parent_de(commands: &mut Commands, parents: &mut ParentsTiles, id: TileId) -> Entity {
    *parents.0.entry(id).or_insert_with(|| commands.spawn((ParentTile(id), Transform::default(), Visibility::default())).id())
}

// ------------------------------------------------------------------ Systèmes de frame

pub fn avancer(mut h: ResMut<Horloge>, soldat: Option<ResMut<Soldat>>, navire: Option<ResMut<Navire>>, fenetre: Option<Res<Time<Real>>>, mode: Option<Res<ModeFenetre>>) {
    if h.pause {
        return;
    }
    let dt_reel = match (mode, fenetre) {
        (Some(m), Some(t)) if m.temps_reel => t.delta_secs_f64().min(0.1),
        _ => h.dt_frame,
    };
    let dt = dt_reel * h.acceleration;
    let mut a = h.alpha + dt * TICK_HZ;
    let ticks = a.floor() as u64;
    a -= ticks as f64;
    h.alpha = a;
    h.tick += ticks;
    if let Some(mut s) = soldat {
        for _ in 0..ticks {
            s.pas();
        }
    }
    if let Some(mut n) = navire {
        n.s += dt * n.vitesse;
    }
}

/// Mode fenêtré : temps réel ou frames de durée fixe.
#[derive(Resource)]
pub struct ModeFenetre {
    pub temps_reel: bool,
}

/// Distance de la caméra (zoom) ; peut être pilotée au clavier.
#[derive(Resource)]
pub struct Zoom {
    pub d: f64,
    pub auto: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn poser_scene(
    h: Res<Horloge>,
    scene: Res<Scene>,
    soldat: Option<Res<Soldat>>,
    navire: Option<ResMut<Navire>>,
    zoom: Option<Res<Zoom>>,
    depart: Option<Res<DepartCout>>,
    mut tiles: ResMut<Tiles>,
    mut cam: ResMut<CameraJeu>,
    mut rel: ResMut<Releve>,
    mut q: Query<(Entity, &mut Ancre, Has<CamRig>), With<Mobile>>,
    mut commands: Commands,
    mut parents: ResMut<ParentsTiles>,
    reglage: Res<Reglage>,
) {
    let alpha = h.alpha as f32;
    match &scene.0 {
        Scenario::Soldat { .. } | Scenario::Zoom { .. } => {
            let s = soldat.unwrap();
            tiles.assurer(s.tile);
            let p = s.interpole(alpha);
            let local = Vec3::new(p.x, 0.0, -p.y);
            let r = *tiles.lire(&s.tile);
            let cible = r.ecef_depuis_local.transform_point3((local + Vec3::new(0.0, 1.0, 0.0)).as_dvec3());
            let d = match (&scene.0, zoom) {
                (_, Some(z)) => z.d,
                (Scenario::Soldat { distance, .. }, None) => *distance,
                (Scenario::Zoom { frames, .. }, None) => {
                    let u = (rel.frame as f64 / *frames as f64).min(2.0);
                    let u = if u > 1.0 { 2.0 - u } else { u };
                    25.0 * (3.0e7f64 / 25.0).powf(u)
                }
                _ => 25.0,
            };
            let (pos, rot) = rig_iso(cible, d, 90.0, 35.264);
            cam.pos = pos;
            cam.rot = rot;
            for (e, mut a, est_cam) in &mut q {
                if est_cam {
                    *a = Ancre::Global { ecef: pos, rot };
                    continue;
                }
                // Reparenter le soldat s'il a changé de tile (stratégies hiérarchiques).
                if let Ancre::Tile { id, .. } = *a
                    && id != s.tile
                    && reglage.strat.hierarchie()
                {
                    let par = parent_de(&mut commands, &mut parents, s.tile);
                    commands.entity(e).insert(ChildOf(par));
                }
                *a = Ancre::Tile { id: s.tile, local, rot: Quat::IDENTITY };
            }
        }
        Scenario::Navire => {
            let mut nav = navire.unwrap();
            let s = nav.s;
            let (n, cap) = nav.etat(s);
            let up = n;
            let g = normale_geo(n);
            let p = geo::vers_ecef(g);
            let rot_ecef = DQuat::from_mat3(&DMat3::from_cols(cap.cross(up).normalize(), up, -cap));
            // Dans un port : ancre de tile (zone du port).
            let port = nav.ports.iter().find(|(pn, _)| pn.angle_between(n) * 6_371_000.0 < nav.rayon_port).map(|x| x.1);
            let ancre = match port {
                Some(epsg) => {
                    let z = geo::zone_par_epsg(epsg);
                    let gp = z.proj.avant(g);
                    let id = TileId::de_grille(epsg, gp);
                    let r = tiles.assurer(id);
                    let loc = gp - id.coin();
                    let local = Vec3::new(loc.x as f32, 0.0, -loc.y as f32);
                    let rot_local = (r.rotation.inverse() * rot_ecef).as_quat();
                    Ancre::Tile { id, local, rot: rot_local }
                }
                None => Ancre::Global { ecef: p, rot: rot_ecef },
            };
            let en_tile = port.is_some();
            let (pos_ref, _) = pose(&ancre, &tiles);
            let (cpos, crot) = rig_iso(pos_ref + up * 10.0, 400.0, 45.0, 35.264);
            if en_tile != nav.en_tile {
                // Saut de représentation : même instant, l'autre ancre.
                let autre = if en_tile { p } else {
                    // Position que donnait l'ancre de tile précédente.
                    let z = geo::zone_par_epsg(nav.ports.iter().min_by(|a, b| a.0.angle_between(n).partial_cmp(&b.0.angle_between(n)).unwrap()).unwrap().1);
                    let gp = z.proj.avant(g);
                    let id = TileId::de_grille(z.epsg, gp);
                    let r = tiles.assurer(id);
                    let loc = gp - id.coin();
                    r.vers_ecef(DVec3::new(loc.x as f32 as f64, 0.0, -(loc.y as f32) as f64))
                };
                let saut = (autre - pos_ref).length();
                let px_par_m = HAUTEUR as f64 / (2.0 * 400.0 * (FOV_DEG.to_radians() / 2.0).tan());
                nav.sauts.push(SautNavire { temps: h.temps(), vers_tile: en_tile, saut_m: saut, saut_px: saut * px_par_m });
                nav.en_tile = en_tile;
            }
            cam.pos = cpos;
            cam.rot = crot;
            for (_, mut a, est_cam) in &mut q {
                *a = if est_cam { Ancre::Global { ecef: cpos, rot: crot } } else { ancre };
            }
            if rel.actif && rel.frame % 30 == 0 {
                let ecart_tile = if let Ancre::Tile { id, local, .. } = ancre {
                    let z = geo::zone_par_epsg(id.epsg);
                    (geo::local_vers_ecef_exact(&z, id, local.as_dvec3()) - pos_ref).length()
                } else {
                    0.0
                };
                let fr = rel.frame;
                rel.chrono.push(format!("{},{:.1},{:.4},{:.4},{},{:.6}", fr, s / 1000.0, g.lat, g.lon, en_tile as u8, ecart_tile));
            }
        }
        Scenario::Vide => {
            for (_, mut a, est_cam) in &mut q {
                if est_cam {
                    *a = Ancre::Global { ecef: cam.pos, rot: cam.rot };
                }
            }
        }
        Scenario::Cout { vitesse, .. } => {
            let t0 = depart.unwrap().0;
            let r = tiles.assurer(t0);
            let x = (h.tick as f64 + h.alpha) / TICK_HZ * vitesse;
            let cible = r.ecef_depuis_local.transform_point3(DVec3::new(500.0 + x, 0.0, -500.0));
            let (pos, rot) = rig_iso(cible, 300.0, 45.0, 35.264);
            cam.pos = pos;
            cam.rot = rot;
            let f = rel.frame;
            for (_, mut a, est_cam) in &mut q {
                if est_cam {
                    *a = Ancre::Global { ecef: pos, rot };
                } else if let Ancre::Tile { local, .. } = a.as_mut() {
                    // Les mobiles piétinent : petit pas déterministe.
                    local.x += if f % 2 == 0 { 0.01 } else { -0.01 };
                }
            }
        }
    }
}

pub fn maj_origine(reglage: Res<Reglage>, cam: Res<CameraJeu>, mut o: ResMut<Origine>) {
    o.change = false;
    let local_cam = || geo::bevy_depuis_ecef(geo::depuis_ecef(cam.pos));
    match reglage.strat {
        Strategie::NaifParis => {
            if !o.init {
                let p = geo::lieu("Paris").g;
                o.poser(geo::vers_ecef(p), geo::bevy_depuis_ecef(p));
            }
        }
        Strategie::NaifEcef | Strategie::BigSpace(_) => {
            if !o.init {
                o.poser(DVec3::ZERO, geo::bevy_depuis_ecef_fixe());
            }
        }
        Strategie::ChaqueFrame | Strategie::Tiles => {
            let r = local_cam();
            o.poser(cam.pos, r);
        }
        Strategie::Seuil(s) | Strategie::TilesSeuil(s) => {
            if !o.init || (cam.pos - o.ecef).length() > s {
                let r = local_cam();
                o.poser(cam.pos, r);
            }
        }
        Strategie::Decalage(s) => {
            let r = local_cam();
            // Origine à s mètres à l'est de la caméra (axes locaux de la caméra).
            let est = r.transpose().col(0);
            o.poser(cam.pos + est * s, r);
        }
    }
}

fn chrono_debut(mut c: ResMut<Chronos>) {
    c.t0 = Some(Instant::now());
}

fn chrono_fin_recalage(mut c: ResMut<Chronos>, o: Res<Origine>) {
    let t = Instant::now();
    if c.actif {
        let d = (t - c.t0.unwrap()).as_secs_f64() * 1000.0;
        c.recalage.push(d);
        c.frames_recalees.push(o.change);
    }
    c.t1 = Some(t);
}

fn chrono_fin_propagation(mut c: ResMut<Chronos>) {
    if c.actif {
        let d = (Instant::now() - c.t1.unwrap()).as_secs_f64() * 1000.0;
        c.propagation.push(d);
    }
}

fn rendu_de(o: &Origine, p: DVec3, r: DQuat) -> (Vec3, Quat) {
    (o.vers_rendu(p).as_vec3(), (o.qrot * r).normalize().as_quat())
}

/// Stratégies « plates » : chaque modèle racine reçoit sa position relative à
/// l'origine. Tout est recalculé quand l'origine change, sinon seuls les
/// modèles dont l'ancre a changé.
fn appliquer_plat(
    reglage: Res<Reglage>,
    o: Res<Origine>,
    tiles: Res<Tiles>,
    mut q: Query<(Ref<Ancre>, &mut Transform), Without<ChildOf>>,
) {
    let tout = o.change;
    q.par_iter_mut().batching_strategy(BatchingStrategy::new().min_batch_size(reglage.lot)).for_each(|(a, mut t)| {
        if tout || a.is_changed() {
            let (p, r) = pose(&a, &tiles);
            let (tr, ro) = rendu_de(&o, p, r);
            t.translation = tr;
            t.rotation = ro;
        }
    });
}

/// Stratégies hiérarchiques : les parents de tiles suivent l'origine, les
/// enfants gardent leur position métrique locale.
fn appliquer_tiles(
    reglage: Res<Reglage>,
    o: Res<Origine>,
    tiles: Res<Tiles>,
    mut parents: Query<(&ParentTile, &mut Transform), Without<Ancre>>,
    mut enfants: Query<(Ref<Ancre>, &mut Transform), (With<ChildOf>, Without<ParentTile>)>,
) {
    if o.change {
        for (pt, mut t) in &mut parents {
            let r = tiles.lire(&pt.0);
            let (tr, ro) = rendu_de(&o, r.ecef_depuis_local.translation, r.rotation);
            t.translation = tr;
            t.rotation = ro;
        }
    } else {
        // Parents créés en cours de route.
        for (pt, mut t) in &mut parents {
            if t.translation == Vec3::ZERO {
                let r = tiles.lire(&pt.0);
                let (tr, ro) = rendu_de(&o, r.ecef_depuis_local.translation, r.rotation);
                t.translation = tr;
                t.rotation = ro;
            }
        }
    }
    enfants.par_iter_mut().batching_strategy(BatchingStrategy::new().min_batch_size(reglage.lot)).for_each(|(a, mut t)| {
        if a.is_changed()
            && let Ancre::Tile { id, local, rot } = *a
        {
            let r = tiles.lire(&id);
            // Décalage métrique depuis le coin de la tile, dans le repère orthonormé de la tile.
            let m = r.rotation.inverse() * (r.vers_ecef(local.as_dvec3()) - r.ecef_depuis_local.translation);
            t.translation = m.as_vec3();
            t.rotation = rot;
        }
    });
}

fn appliquer_big(
    reglage: Res<Reglage>,
    tiles: Res<Tiles>,
    mut o: ResMut<Origine>,
    grilles: Query<&Grid, With<BigSpace>>,
    mut qs: ParamSet<(Query<(&Ancre, &CellCoord), With<CamRig>>, Query<(Ref<Ancre>, &mut Transform, &mut CellCoord)>)>,
) {
    let Ok(grille) = grilles.single() else { return };
    let m = geo::bevy_depuis_ecef_fixe();
    let qm = DQuat::from_mat3(&m);
    // Pour les statistiques : l'origine flottante change-t-elle de cellule ?
    if let Ok((a, c)) = qs.p0().single() {
        let (p, _) = pose(a, &tiles);
        o.change = grille.translation_to_grid(m * p).0 != *c;
    }
    qs.p1().par_iter_mut().batching_strategy(BatchingStrategy::new().min_batch_size(reglage.lot)).for_each(|(a, mut t, mut c)| {
        if a.is_changed() {
            let (p, r) = pose(&a, &tiles);
            let (cell, loc) = grille.translation_to_grid(m * p);
            if *c != cell {
                *c = cell;
            }
            t.translation = loc;
            t.rotation = (qm * r).normalize().as_quat();
        }
    });
}

/// big_space : les modèles immobiles sont marqués `Stationary`.
fn marquer_immobiles(mut commands: Commands, q: Query<Entity, (With<CellCoord>, With<Ancre>, Without<Mobile>)>) {
    for e in &q {
        commands.entity(e).insert(Stationary);
    }
}

/// Relecture de la projection : rejoue en f32 ce que fait le GPU (matrice
/// modèle issue du `GlobalTransform`, `clip_from_world` = projection × inverse
/// de la matrice de la caméra, comme `bevy_render`), et compare à la projection
/// de référence en f64.
#[allow(clippy::type_complexity)]
fn mesurer(
    cam: Res<CameraJeu>,
    tiles: Res<Tiles>,
    mut rel: ResMut<Releve>,
    q_cam: Query<&GlobalTransform, With<CamRig>>,
    q: Query<(&Sonde, &Ancre, &GlobalTransform)>,
    scene: Res<Scene>,
) {
    rel.frame += 1;
    if !rel.actif {
        return;
    }
    let Ok(cgt) = q_cam.single() else { return };
    let view_from_world = cgt.to_matrix().inverse();
    let proj = projection32(cam.fov);
    let clip_from_world = proj.get_clip_from_view() * view_from_world;
    let cam_inv = cam.rot.inverse();
    let f = 1.0 / (cam.fov / 2.0).tan();
    let aspect = (LARGEUR / HAUTEUR) as f64;
    let mut max_frame = 0.0f64;
    let rel = &mut *rel;
    let zoom = matches!(scene.0, Scenario::Zoom { .. });
    let mut par_sonde: Vec<(&'static str, f64)> = vec![];
    let mut dist = 0.0;
    for (s, a, gt) in &q {
        let (p, r) = pose(a, &tiles);
        if s.nom == "soldat" {
            dist = (p - cam.pos).length();
        }
        let mut max_sonde = f64::NAN;
        let aff = gt.affine();
        let serie = rel.series.entry(s.nom).or_default();
        if serie.prec.len() < s.points.len() {
            serie.prec.resize(s.points.len(), None);
        }
        for (k, v) in s.points.iter().enumerate() {
            // GPU (f32).
            let w32 = aff.transform_point3(*v);
            let c = clip_from_world * w32.extend(1.0);
            let v32 = view_from_world.transform_point3(w32);
            // Référence (f64).
            let p64 = p + r * v.as_dvec3();
            let v64 = cam_inv * (p64 - cam.pos);
            if v64.z > -PRES as f64 || c.w <= 0.0 {
                serie.prec[k] = None;
                continue;
            }
            let ndc64 = DVec2::new(f / aspect * v64.x / -v64.z, f * v64.y / -v64.z);
            let ndc32 = Vec2::new(c.x / c.w, c.y / c.w);
            if ndc64.x.abs() > 1.2 || ndc64.y.abs() > 1.2 {
                serie.prec[k] = None;
                continue;
            }
            let e = Vec2::new(
                ((ndc32.x as f64 - ndc64.x) * 0.5 * LARGEUR as f64) as f32,
                ((ndc32.y as f64 - ndc64.y) * 0.5 * HAUTEUR as f64) as f32,
            );
            let px = e.length() as f64;
            serie.px.push(px);
            serie.mm.push((v32.as_dvec3() - v64).length() * 1000.0);
            if let Some(pe) = serie.prec[k] {
                serie.trem.push((e - pe).length() as f64);
            }
            serie.prec[k] = Some(e);
            max_frame = max_frame.max(px);
            max_sonde = if max_sonde.is_nan() { px } else { max_sonde.max(px) };
        }
        par_sonde.push((s.nom, max_sonde));
    }
    rel.derniere_max_px = max_frame;
    if zoom {
        // Résolution de la profondeur inversée (f32) à cette distance.
        let d32 = PRES / dist as f32;
        let dn = f32::from_bits(d32.to_bits() - 1);
        let dz = PRES as f64 / dn as f64 - PRES as f64 / d32 as f64;
        par_sonde.sort_by_key(|x| x.0);
        let mut l = format!("{},{:.3},{:.4e}", rel.frame, dist, dz);
        for (_, v) in &par_sonde {
            l.push_str(&format!(",{:.4e}", v));
        }
        if rel.chrono.is_empty() {
            let mut e = "frame,distance_m,resolution_profondeur_m".to_string();
            for (n, _) in &par_sonde {
                e.push_str(&format!(",{n}_px"));
            }
            rel.chrono.push(e);
        }
        rel.chrono.push(l);
    }
}

/// Résumé des séries : (sonde, écart px, écart mm, tremblement px).
pub fn resumes(rel: &Releve) -> Vec<(&'static str, Resume, Resume, Resume)> {
    let mut noms: Vec<_> = rel.series.keys().copied().collect();
    noms.sort();
    noms.iter()
        .map(|n| {
            let s = &rel.series[n];
            (*n, stats::resumer(&s.px), stats::resumer(&s.mm), stats::resumer(&s.trem))
        })
        .collect()
}
