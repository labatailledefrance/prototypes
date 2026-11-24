//! Rendu par Bevy 0.20.1 sur la RTX 3060, **hors écran** (caméra rendue dans
//! une image, sans winit : sous Wayland, une fenêtre est cadencée par le
//! compositeur, P-06).
//!
//! - Positions : chaque tile est une entité ancrée sur son centre en
//!   Lambert-93 (f64) ; son `Transform` est relatif à une **origine de rendu**
//!   recalée quand la caméra s'en éloigne de plus de 5 km (C-01, P-06). Les
//!   sommets sont locaux à la tile (f32, ±500 m et un peu plus).
//! - Organisations comparées : `tuile` (un maillage fusionné par tile),
//!   `morceaux` (fusion par carré de 250 m), `batiment` (une entité par
//!   bâtiment).
//! - Modes : `cadence` (temps de frame et mémoire GPU, 1, 9 ou 49 tiles),
//!   `survol` (streaming, génération en tâche de fond), `rendu-destruction`
//!   (mise à jour d'un bâtiment), `capture` (images de vérification).

use crate::Args;
use crate::batiment::{Etat, Lod, Maillage, Params};
use crate::donnees::{Cache, Contexte, Quoi};
use crate::generation::{self, Gen, Ordre};
use crate::geo::TileId;
use crate::stats::quantile;
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::CascadeShadowConfigBuilder;
use bevy::mesh::{Indices, MeshCompressionArgs, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::tasks::{AsyncComputeTaskPool, Task};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const LARGEUR: u32 = 1920;
pub const HAUTEUR: u32 = 1080;
pub const SEUIL_RECALAGE: f64 = 5000.0;

// ------------------------------------------------------------ Génération d'une tile pour le rendu

/// Ce que produit la génération d'une tile : maillages à envoyer au GPU.
pub struct Produit {
    pub t: TileId,
    pub morceaux: Vec<Maillage>,
    pub sol: Maillage,
    pub ms: f64,
    pub batiments: usize,
}

/// Sol d'une tile : relief décimé à 8 m (ou plan à Paris, sans relief).
pub fn sol(ctx: &Contexte) -> Maillage {
    let mut m = Maillage::default();
    let pas = 8usize;
    let n = 1000 / pas;
    let rel = ctx.tile().relief.as_ref();
    let c = [118u8, 128, 98];
    if rel.is_none() {
        let i0 = m.sommet([-500.0, -500.0, -0.05], [0.0, 0.0, 1.0], crate::batiment::UV_NU, [92, 96, 100]);
        m.sommet([500.0, -500.0, -0.05], [0.0, 0.0, 1.0], crate::batiment::UV_NU, [92, 96, 100]);
        m.sommet([500.0, 500.0, -0.05], [0.0, 0.0, 1.0], crate::batiment::UV_NU, [92, 96, 100]);
        m.sommet([-500.0, 500.0, -0.05], [0.0, 0.0, 1.0], crate::batiment::UV_NU, [92, 96, 100]);
        m.idx.extend_from_slice(&[i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3]);
        return m;
    }
    let r = rel.unwrap();
    let z = |i: usize, j: usize| {
        let v = r.get(i * pas / 2, j * pas / 2);
        if v == crate::relief::SANS { 0.0 } else { v as f64 / 100.0 }
    };
    for j in 0..=n {
        for i in 0..=n {
            let (zx0, zx1) = (z(i.saturating_sub(1), j), z((i + 1).min(n), j));
            let (zy0, zy1) = (z(i, j.saturating_sub(1)), z(i, (j + 1).min(n)));
            let (gx, gy) = ((zx1 - zx0) / (2.0 * pas as f64), (zy1 - zy0) / (2.0 * pas as f64));
            let l = (gx * gx + gy * gy + 1.0).sqrt();
            let mer = r.get(i * pas / 2, j * pas / 2) == crate::relief::SANS;
            m.sommet([i as f64 * pas as f64 - 500.0, j as f64 * pas as f64 - 500.0, z(i, j) - 0.1], [-gx / l, -gy / l, 1.0 / l], crate::batiment::UV_NU, if mer { [60, 90, 130] } else { c });
        }
    }
    let w = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let k = j * w + i;
            m.idx.extend_from_slice(&[k, k + 1, k + w + 1, k, k + w + 1, k + w]);
        }
    }
    m
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Organisation {
    Tuile,
    Morceaux,
    Batiment,
}

impl Organisation {
    fn lire(s: &str) -> Self {
        match s {
            "tuile" => Organisation::Tuile,
            "morceaux" => Organisation::Morceaux,
            "batiment" => Organisation::Batiment,
            x => panic!("organisation inconnue {x}"),
        }
    }
}

pub fn produire(cache: &Cache, t: TileId, lod: Lod, org: Organisation, etats: &HashMap<u64, Etat>) -> Option<Produit> {
    let t0 = Instant::now();
    let ctx = cache.contexte(t)?;
    let p = Params::default();
    // Pas de portes ici : elles seraient précalculées par la chaîne (voir
    // le README) ; leur coût est mesuré à part.
    let prep = generation::preparer(&ctx, &p, false);
    let g = Gen { ctx: &ctx, prep: &prep, p: &p, lod, etats };
    let (s, _) = generation::tout(&g, false, Ordre::Direct);
    let morceaux = match org {
        Organisation::Tuile => vec![generation::fusionner(&s).0],
        Organisation::Batiment => s.into_iter().map(|x| x.mesh).collect(),
        Organisation::Morceaux => {
            let mut par: BTreeMap<(i32, i32), Maillage> = BTreeMap::new();
            for (k, x) in s.iter().enumerate() {
                par.entry(generation::morceau_de(&ctx.tile().bats[k], 250.0)).or_default().ajouter(&x.mesh);
            }
            par.into_values().collect()
        }
    };
    let sol = sol(&ctx);
    Some(Produit { t, morceaux, sol, ms: t0.elapsed().as_secs_f64() * 1e3, batiments: ctx.tile().bats.len() })
}

fn srgb_lin(c: u8) -> f32 {
    (c as f32 / 255.0).powf(2.2)
}

pub fn mesh_bevy(m: &Maillage, compresse: bool) -> Mesh {
    let col: Vec<[f32; 4]> = m.col.iter().map(|c| [srgb_lin(c[0]), srgb_lin(c[1]), srgb_lin(c[2]), 1.0]).collect();
    let me = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, m.pos.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, m.nor.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, m.uv.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
        .with_inserted_indices(Indices::U32(m.idx.clone()));
    if compresse {
        match me.compressed_mesh(&MeshCompressionArgs::regular()) {
            Ok(x) => x,
            Err((x, _)) => x,
        }
    } else {
        me
    }
}

// ------------------------------------------------------------ Ressources et composants

#[derive(Resource)]
struct Monde {
    cache: Arc<Cache>,
    lod: Lod,
    org: Organisation,
    compresse: bool,
    pregen: Option<Arc<Mutex<HashMap<TileId, Arc<Produit>>>>>,
    etats: HashMap<u64, Etat>,
}

/// Origine du rendu (Lambert-93, m).
#[derive(Resource, Default)]
struct Origine {
    x: f64,
    y: f64,
    recalages: u32,
    ms_dernier: f64,
}

/// Caméra (Lambert-93, m) et point visé.
#[derive(Resource, Default, Clone, Copy)]
struct Cam {
    pos: [f64; 3],
    cible: [f64; 3],
}

#[derive(Component)]
struct Ancre {
    t: TileId,
}

#[derive(Component)]
struct CameraJeu;

/// Rang de l'entité dans sa tile (bâtiment, morceau ; usize::MAX pour le sol).
#[derive(Component)]
struct Part(usize);

/// Tile de la destruction : bâtiments générés gardés en mémoire.
#[derive(Resource)]
struct Victime {
    ctx: Contexte,
    prep: generation::Prep,
    sorties: Vec<crate::batiment::Sortie>,
    k: usize,
    morceaux: Vec<(i32, i32)>,
}

#[derive(Resource)]
struct Visuel {
    mat: Handle<StandardMaterial>,
    cible: Handle<Image>,
}

#[derive(Resource, Default)]
struct Chargement {
    taches: HashMap<TileId, Task<Option<(Arc<Produit>, Vec<Mesh>)>>>,
    affichees: HashMap<TileId, Vec<Entity>>,
    ms_gen: Vec<f64>,
    ms_envoi: Vec<f64>,
    /// File d'envoi étalé : (tile, rang, maillage, octets).
    file: std::collections::VecDeque<(TileId, usize, Mesh, usize)>,
    recues: usize,
}

#[derive(Resource, Clone)]
struct Config {
    mode: String,
    etiquette: String,
    frames: u32,
    res: std::path::PathBuf,
    entete: bool,
    cote: i32,
    tiles: Vec<TileId>,
    vitesse: f64,
    altitude: f64,
    rayon: f64,
    trajet: [f64; 4],
    vue: String,
    organisation: String,
    lod: String,
    ombres: bool,
    compresse: bool,
    pregen: bool,
    octets_par_frame: usize,
    dalles: String,
}

#[derive(Resource, Default)]
struct Mesure {
    etape: u64,
    debut: Option<Instant>,
    prec: Option<Instant>,
    frames: Vec<(f64, f64, usize, usize, usize, bool)>,
    fini: bool,
    capture: Arc<Mutex<Option<Image>>>,
    en_vol: bool,
    phase: u32,
    gpu_mo: f64,
    ms_maj: f64,
    apres_maj: Vec<f64>,
    ecritures: Vec<std::thread::JoinHandle<()>>,
}

// ------------------------------------------------------------ Lancement

pub fn lancer(a: &Args) {
    let tiles = crate::tiles_de(&a.tiles);
    let trajet: Vec<f64> = a.trajet.split(',').filter(|x| !x.is_empty()).map(|x| x.parse::<f64>().unwrap() * 1000.0).collect();
    let cfg = Config {
        mode: a.mode.clone(),
        etiquette: a.etiquette.clone(),
        frames: a.frames,
        res: a.res.clone(),
        entete: a.entete,
        cote: a.cote,
        tiles: tiles.clone(),
        vitesse: a.vitesse,
        altitude: a.altitude,
        rayon: a.rayon,
        trajet: if trajet.len() == 4 { [trajet[0], trajet[1], trajet[2], trajet[3]] } else { [0.0; 4] },
        vue: a.vue.clone(),
        organisation: a.organisation.clone(),
        lod: a.lod.clone(),
        ombres: a.ombres,
        compresse: a.compresse,
        pregen: a.pregenere,
        octets_par_frame: a.octets_par_frame,
        dalles: a.dalles.clone(),
    };
    let lod = match a.lod.as_str() {
        "iso" => Lod::Iso,
        "loin" => Lod::Loin,
        _ => Lod::Carte,
    };
    let org = Organisation::lire(&a.organisation);
    let cache = Arc::new(Cache::new(&a.sortie, a.gpkg.as_deref(), Quoi { relief: true, reseau: false }));
    let mut etats = HashMap::new();
    // Tiles à afficher dès le départ (cadence, capture, destruction).
    let mut initiales: Vec<Arc<Produit>> = Vec::new();
    if a.mode != "survol" && a.cote > 0 {
        let c = tiles[0];
        let h = a.cote / 2;
        let bloc: Vec<TileId> = (-h..=h).flat_map(|dy| (-h..=h).map(move |dx| c.voisine(dx, dy))).collect();
        use rayon::prelude::*;
        initiales = bloc.par_iter().filter_map(|t| produire(&cache, *t, lod, org, &etats).map(Arc::new)).collect();
        let tri: usize = initiales.iter().flat_map(|p| p.morceaux.iter()).map(|m| m.triangles()).sum();
        let oct: usize = initiales.iter().flat_map(|p| p.morceaux.iter()).map(|m| if a.compresse { m.octets_gpu_compresses() } else { m.octets_gpu() }).sum();
        let ent: usize = initiales.iter().map(|p| p.morceaux.len()).sum();
        eprintln!("{} tiles, {} entités, {} triangles, {:.0} Mo de maillages", initiales.len(), ent, tri, oct as f64 / 1e6);
    }
    let pregen = if a.mode == "survol" && a.pregenere {
        // Témoin : toutes les tiles du trajet générées avant le vol.
        let mut v = HashSet::new();
        let [x0, y0, x1, y1] = cfg.trajet;
        let l = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        let mut s = 0.0;
        while s <= l {
            let (x, y) = (x0 + (x1 - x0) * s / l, y0 + (y1 - y0) * s / l);
            let r = (a.rayon / 1000.0).ceil() as i32 + 1;
            let c = TileId { ix: (x / 1000.0).floor() as i32, iy: (y / 1000.0).floor() as i32 };
            for dy in -r..=r {
                for dx in -r..=r {
                    v.insert(c.voisine(dx, dy));
                }
            }
            s += 500.0;
        }
        use rayon::prelude::*;
        let v: Vec<TileId> = v.into_iter().collect();
        let m: HashMap<TileId, Arc<Produit>> = v.par_iter().filter_map(|t| produire(&cache, *t, lod, org, &etats).map(|p| (*t, Arc::new(p)))).collect();
        eprintln!("prégénéré : {} tiles", m.len());
        Some(Arc::new(Mutex::new(m)))
    } else {
        None
    };
    let victime = (a.mode == "rendu-destruction").then(|| {
        let ctx = cache.contexte(tiles[0]).unwrap();
        let p = Params::default();
        let prep = generation::preparer(&ctx, &p, false);
        let id = choisir_victime(&ctx.tile().bats);
        let k = ctx.tile().bats.iter().position(|b| b.id == id).unwrap();
        let sorties = generation::tout(&Gen { ctx: &ctx, prep: &prep, p: &p, lod, etats: &etats }, false, Ordre::Direct).0;
        let morceaux = ctx.tile().bats.iter().map(|b| generation::morceau_de(b, 250.0)).collect();
        Victime { ctx, prep, sorties, k, morceaux }
    });
    etats.clear();
    let vise = {
        let c = tiles.first().copied().unwrap_or(TileId { ix: 0, iy: 0 });
        let (cx, cy) = c.centre();
        match cache.contexte(c) {
            Some(ctx) if !ctx.tile().bats.is_empty() => {
                let (mut sx, mut sy, mut sa) = (0.0, 0.0, 0.0);
                for b in &ctx.tile().bats {
                    let p = crate::porte::centroide(b);
                    let a = b.aire();
                    sx += p[0] * a;
                    sy += p[1] * a;
                    sa += a;
                }
                let (x, y) = (sx / sa, sy / sa);
                Vise([cx + x, cy + y, ctx.sol(x, y).unwrap_or(0.0)])
            }
            _ => Vise([cx, cy, 0.0]),
        }
    };
    let fenetres = WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() };
    let base = DefaultPlugins.set(fenetres).set(TaskPoolPlugin { task_pool_options: TaskPoolOptions::with_num_threads(a.threads) }).build().disable::<bevy::winit::WinitPlugin>();
    let mut app = App::new();
    app.add_plugins(base).add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(std::time::Duration::ZERO));
    app.insert_resource(Monde { cache, lod, org, compresse: a.compresse, pregen, etats })
        .insert_resource(Origine::default())
        .insert_resource(Cam::default())
        .insert_resource(Chargement::default())
        .insert_resource(Mesure::default())
        .insert_resource(cfg)
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
        .insert_resource(Initiales(initiales))
        .insert_resource(vise)
        .add_systems(Startup, preparer)
        .add_systems(Update, (piloter, charger, recaler, poser_camera).chain())
        .add_systems(Last, mesurer);
    if let Some(v) = victime {
        app.insert_resource(v);
    }
    if a.dalles == "separees" {
        // Chaque maillage de plus de 1 Mio dans son propre tampon : les
        // dalles générales ne grossissent plus par recopie (P-08).
        use bevy::render::mesh::allocator::MeshAllocatorSettings;
        let mut r = MeshAllocatorSettings::default();
        r.slab_allocator_settings.large_threshold = 1024 * 1024;
        app.sub_app_mut(bevy::render::RenderApp).insert_resource(r);
    }
    app.run();
}

#[derive(Resource)]
struct Initiales(Vec<Arc<Produit>>);

/// Point visé par les modes fixes : barycentre des bâtiments de la tile
/// (pondéré par l'aire), à l'altitude du sol.
#[derive(Resource, Clone, Copy)]
struct Vise([f64; 3]);

fn texture_fenetres() -> Image {
    // Une cellule = une travée × un niveau : mur blanc (la couleur du sommet
    // le teinte), fenêtre sombre au milieu.
    let n = 32u32;
    let mut d = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let f = (9..23).contains(&x) && (8..24).contains(&y);
            let c = if f { [48, 54, 66, 255] } else { [255, 255, 255, 255] };
            d.extend_from_slice(&c);
        }
    }
    let mut img = Image::new(Extent3d { width: n, height: n, depth_or_array_layers: 1 }, TextureDimension::D2, d, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor { address_mode_u: ImageAddressMode::Repeat, address_mode_v: ImageAddressMode::Repeat, ..default() });
    img
}

fn preparer(
    mut commands: Commands,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    cfg: Res<Config>,
    mut cam: ResMut<Cam>,
    mut o: ResMut<Origine>,
) {
    let tex = images.add(texture_fenetres());
    let mat = mats.add(StandardMaterial { base_color: Color::WHITE, base_color_texture: Some(tex), perceptual_roughness: 0.9, ..default() });
    let mut img = Image::new_target_texture(LARGEUR, HAUTEUR, TextureFormat::Rgba8UnormSrgb, None);
    img.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let cible = images.add(img);
    commands.spawn((
        Camera3d::default(),
        bevy::camera::RenderTarget::Image(cible.clone().into()),
        Projection::Perspective(PerspectiveProjection { fov: 40f32.to_radians(), near: 0.5, far: 200_000.0, ..default() }),
        Msaa::Sample4,
        Transform::default(),
        CameraJeu,
    ));
    let loin = cfg.vue == "loin" || cfg.lod != "iso" || cfg.altitude > 1500.0;
    commands.spawn((
        DirectionalLight { shadow_maps_enabled: cfg.ombres, illuminance: 11_000.0, ..default() },
        CascadeShadowConfigBuilder { maximum_distance: if loin { 6000.0 } else { 800.0 }, first_cascade_far_bound: if loin { 400.0 } else { 60.0 }, ..default() }.build(),
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 2.3, -0.75, 0.0)),
    ));
    commands.insert_resource(GlobalAmbientLight { brightness: 500.0, ..default() });
    commands.insert_resource(Visuel { mat, cible });
    // Position initiale de la caméra et de l'origine.
    let (x, y) = if cfg.mode == "survol" {
        (cfg.trajet[0], cfg.trajet[1])
    } else {
        let c = cfg.tiles[0].centre();
        (c.0, c.1)
    };
    o.x = x;
    o.y = y;
    cam.cible = [x, y, 0.0];
    cam.pos = [x, y, 300.0];
}

/// Maillages de Bevy d'une tile produite : (rang, maillage), sol compris.
pub fn meshes_bevy(p: &Produit, compresse: bool) -> Vec<Mesh> {
    p.morceaux.iter().chain(std::iter::once(&p.sol)).map(|me| mesh_bevy(me, compresse)).collect()
}

/// Ajoute les entités d'une tile produite.
fn afficher(commands: &mut Commands, meshes: &mut Assets<Mesh>, v: &Visuel, o: &Origine, p: &Produit, me: Vec<Mesh>) -> Vec<Entity> {
    let (cx, cy) = p.t.centre();
    let tr = Transform::from_xyz((cx - o.x) as f32, 0.0, -((cy - o.y) as f32));
    let mut out = Vec::new();
    let rangs = (0..p.morceaux.len()).chain(std::iter::once(usize::MAX));
    let vides = p.morceaux.iter().chain(std::iter::once(&p.sol)).map(|x| x.idx.is_empty());
    for ((k, mb), vide) in rangs.zip(me).zip(vides) {
        if vide {
            continue;
        }
        let h = meshes.add(mb);
        out.push(commands.spawn((Mesh3d(h), MeshMaterial3d(v.mat.clone()), tr, Ancre { t: p.t }, Part(k))).id());
    }
    out
}

/// Pilote la caméra selon le mode.
fn piloter(cfg: Res<Config>, mut cam: ResMut<Cam>, mes: Res<Mesure>, reel: Res<Time<Real>>, vise: Res<Vise>) {
    let k = mes.etape as f64;
    match cfg.mode.as_str() {
        "survol" => {
            // Vol à vitesse constante en temps réel, après la compilation des shaders.
            let t = (reel.elapsed_secs_f64() - 6.0).max(0.0);
            let [x0, y0, x1, y1] = cfg.trajet;
            let l = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
            let s = (cfg.vitesse * t).min(l);
            let (dx, dy) = ((x1 - x0) / l, (y1 - y0) / l);
            let (cx, cy) = (x0 + dx * s, y0 + dy * s);
            let recul = cfg.altitude / 0.84;
            cam.cible = [cx, cy, 0.0];
            cam.pos = [cx - dx * recul, cy - dy * recul, cfg.altitude];
        }
        _ => {
            // Orbite lente autour du centre du bloc.
            let c = (vise.0[0], vise.0[1]);
            let z0 = vise.0[2];
            let (d, el): (f64, f64) = match (cfg.mode.as_str(), cfg.vue.as_str(), cfg.cote) {
                (_, "loin", _) => (4000.0, 0.85),
                ("cadence", _, 1) => (900.0, 0.7),
                ("cadence", _, 3) => (2200.0, 0.7),
                ("cadence", _, _) => (5200.0, 0.7),
                _ => (380.0, 0.62),
            };
            let az = 2.3 + k * 0.0008;
            cam.cible = [c.0, c.1, z0];
            cam.pos = [c.0 + d * el.cos() * az.cos(), c.1 + d * el.cos() * az.sin(), z0 + d * el.sin()];
        }
    }
}

/// Streaming : demande et reçoit les tiles autour du point visé.
#[allow(clippy::too_many_arguments)]
fn charger(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    v: Res<Visuel>,
    m: Res<Monde>,
    o: Res<Origine>,
    cfg: Res<Config>,
    cam: Res<Cam>,
    mut ch: ResMut<Chargement>,
    mut ini: ResMut<Initiales>,
) {
    for p in ini.0.drain(..) {
        let me = meshes_bevy(&p, m.compresse);
        let e = afficher(&mut commands, &mut meshes, &v, &o, &p, me);
        ch.affichees.insert(p.t, e);
    }
    if cfg.mode != "survol" {
        return;
    }
    let c = TileId { ix: (cam.cible[0] / 1000.0).floor() as i32, iy: (cam.cible[1] / 1000.0).floor() as i32 };
    let r = (cfg.rayon / 1000.0).ceil() as i32;
    let dist = |t: TileId| {
        let (x, y) = t.centre();
        ((x - cam.cible[0]).powi(2) + (y - cam.cible[1]).powi(2)).sqrt()
    };
    // Tiles voulues, les plus proches d'abord.
    let mut voulues: Vec<TileId> = (-r..=r).flat_map(|dy| (-r..=r).map(move |dx| c.voisine(dx, dy))).filter(|t| dist(*t) <= cfg.rayon).collect();
    voulues.sort_by(|a, b| dist(*a).partial_cmp(&dist(*b)).unwrap());
    let pool = AsyncComputeTaskPool::get();
    for t in voulues {
        if ch.affichees.contains_key(&t) || ch.taches.contains_key(&t) || ch.taches.len() >= 4 {
            continue;
        }
        let (cache, lod, org, pregen, compresse) = (m.cache.clone(), m.lod, m.org, m.pregen.clone(), m.compresse);
        let etats = m.etats.clone();
        ch.taches.insert(
            t,
            pool.spawn(async move {
                let p = match pregen {
                    Some(pg) => pg.lock().unwrap().get(&t).cloned(),
                    None => produire(&cache, t, lod, org, &etats).map(Arc::new),
                }?;
                // Maillages de Bevy construits ici, hors du thread principal.
                let me = meshes_bevy(&p, compresse);
                Some((p, me))
            }),
        );
    }
    // Réception.
    let recus: Vec<(TileId, Option<(Arc<Produit>, Vec<Mesh>)>)> = ch.taches.iter_mut().filter_map(|(t, k)| bevy::tasks::futures::check_ready(k).map(|r| (*t, r))).collect();
    let mut prets = Vec::new();
    for (t, r) in recus {
        if let Some((p, me)) = r {
            ch.recues += 1;
            if m.pregen.is_none() {
                ch.ms_gen.push(p.ms);
            }
            if cfg.octets_par_frame > 0 {
                // Envoi étalé : les morceaux passent par la file.
                let rangs = (0..p.morceaux.len()).chain(std::iter::once(usize::MAX));
                let tailles: Vec<(bool, usize)> = p.morceaux.iter().chain(std::iter::once(&p.sol)).map(|x| (x.idx.is_empty(), x.octets_gpu())).collect();
                for ((k, mb), (vide, oct)) in rangs.zip(me).zip(tailles) {
                    if !vide {
                        ch.file.push_back((t, k, mb, oct));
                    }
                }
                ch.affichees.insert(t, Vec::new());
            } else {
                let t0 = Instant::now();
                let e = afficher(&mut commands, &mut meshes, &v, &o, &p, me);
                ch.ms_envoi.push(t0.elapsed().as_secs_f64() * 1e3);
                ch.affichees.insert(t, e);
            }
        } else {
            ch.affichees.insert(t, Vec::new());
        }
        prets.push(t);
    }
    for t in prets {
        ch.taches.remove(&t);
    }
    // File d'envoi : au plus `octets_par_frame` par frame (au moins un morceau).
    let mut budget = cfg.octets_par_frame as i64;
    while budget > 0 {
        let Some((t, k, mb, oct)) = ch.file.pop_front() else { break };
        let Some(liste) = ch.affichees.get_mut(&t) else { continue };
        let (cx, cy) = t.centre();
        let tr = Transform::from_xyz((cx - o.x) as f32, 0.0, -((cy - o.y) as f32));
        let h = meshes.add(mb);
        liste.push(commands.spawn((Mesh3d(h), MeshMaterial3d(v.mat.clone()), tr, Ancre { t }, Part(k))).id());
        budget -= oct as i64;
    }
    // Déchargement au-delà de 1,3 rayon (hystérésis).
    let loin: Vec<TileId> = ch.affichees.keys().copied().filter(|t| dist(*t) > cfg.rayon * 1.3 + 800.0).collect();
    for t in loin {
        for e in ch.affichees.remove(&t).unwrap() {
            commands.entity(e).despawn();
        }
    }
    if mesure_etape_mod(&ch) {
        let garder: HashSet<TileId> = ch.affichees.keys().chain(ch.taches.keys()).copied().collect();
        m.cache.oublier(|t| garder.iter().any(|g| (g.ix - t.ix).abs() <= 1 && (g.iy - t.iy).abs() <= 1));
    }
}

fn mesure_etape_mod(ch: &Chargement) -> bool {
    ch.affichees.len() % 16 == 0
}

/// Recalage de l'origine au-delà de 5 km (C-01).
fn recaler(cam: Res<Cam>, mut o: ResMut<Origine>, mut q: Query<(&Ancre, &mut Transform)>) {
    let d = ((cam.pos[0] - o.x).powi(2) + (cam.pos[1] - o.y).powi(2)).sqrt();
    if d <= SEUIL_RECALAGE {
        o.ms_dernier = 0.0;
        return;
    }
    let t0 = Instant::now();
    o.x = cam.pos[0];
    o.y = cam.pos[1];
    for (a, mut t) in &mut q {
        let (cx, cy) = a.t.centre();
        t.translation = Vec3::new((cx - o.x) as f32, 0.0, -((cy - o.y) as f32));
    }
    o.recalages += 1;
    o.ms_dernier = t0.elapsed().as_secs_f64() * 1e3;
}

fn poser_camera(cam: Res<Cam>, o: Res<Origine>, mut q: Query<&mut Transform, With<CameraJeu>>) {
    let rel = |p: [f64; 3]| Vec3::new((p[0] - o.x) as f32, p[2] as f32, -((p[1] - o.y) as f32));
    for mut t in &mut q {
        *t = Transform::from_translation(rel(cam.pos)).looking_at(rel(cam.cible), Vec3::Y);
    }
}

fn memoire_gpu() -> f64 {
    let s = std::process::Command::new("nvidia-smi").args(["--query-gpu=name,memory.used", "--format=csv,noheader,nounits"]).output();
    let Ok(s) = s else { return f64::NAN };
    String::from_utf8_lossy(&s.stdout).lines().find(|l| l.contains("3060")).and_then(|l| l.rsplit(',').next()).and_then(|x| x.trim().parse().ok()).unwrap_or(f64::NAN)
}

fn png(chemin: &std::path::Path, img: &Image) {
    let (w, h) = (img.width(), img.height());
    let d = img.data.as_ref().unwrap();
    // L'image peut être en BGRA selon la surface : la cible est en RGBA ici.
    let f = std::fs::File::create(chemin).unwrap();
    let mut e = ::png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(::png::ColorType::Rgba);
    e.set_depth(::png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(d).unwrap();
}

/// Mesures et fin de chaque mode.
#[allow(clippy::too_many_arguments)]
fn mesurer(
    mut commands: Commands,
    mut mes: ResMut<Mesure>,
    cfg: Res<Config>,
    reel: Res<Time<Real>>,
    ch: Res<Chargement>,
    o: Res<Origine>,
    v: Res<Visuel>,
    mut m: ResMut<Monde>,
    mut meshes: ResMut<Assets<Mesh>>,
    q: Query<(Entity, &Ancre)>,
    mut qp: Query<(&Ancre, &Part, &mut Mesh3d)>,
    vic: Option<ResMut<Victime>>,
    cam: Res<Cam>,
    mut exit: MessageWriter<AppExit>,
) {
    mes.etape += 1;
    let now = Instant::now();
    let pret = reel.elapsed_secs_f64() > 6.0;
    let dt = mes.prec.map(|p| (now - p).as_secs_f64() * 1e3).unwrap_or(0.0);
    mes.prec = Some(now);
    if !pret || mes.fini {
        return;
    }
    if mes.debut.is_none() {
        mes.debut = Some(now);
        mes.etape = 0;
        return;
    }
    let t = (now - mes.debut.unwrap()).as_secs_f64();
    match cfg.mode.as_str() {
        "cadence" => {
            if mes.etape > 120 {
                mes.frames.push((t, dt, 0, 0, 0, false));
            }
            if mes.etape >= 120 + cfg.frames as u64 {
                // Mémoire GPU lue à la fin (nvidia-smi bloque la frame qui l'appelle).
                mes.gpu_mo = memoire_gpu();
                let mut f: Vec<f64> = mes.frames.iter().map(|x| x.1).collect();
                let n_ent = q.iter().count();
                if cfg.entete {
                    println!("etiquette,tiles,organisation,lod,ombres,compresse,dalles,entites,frame_med_ms,frame_p99_ms,frame_max_ms,gpu_mo");
                }
                let (med, p99) = (quantile(&mut f, 0.5), quantile(&mut f, 0.99));
                println!("{},{},{},{},{},{},{},{},{:.2},{:.2},{:.2},{:.0}", cfg.etiquette, cfg.cote * cfg.cote, cfg.organisation, cfg.lod, cfg.ombres, cfg.compresse, cfg.dalles, n_ent, med, p99, f.last().unwrap(), mes.gpu_mo);
                mes.fini = true;
                exit.write(AppExit::Success);
            }
        }
        "survol" => {
            let voulues_proches = {
                let c = TileId { ix: (cam.cible[0] / 1000.0).floor() as i32, iy: (cam.cible[1] / 1000.0).floor() as i32 };
                let r = (cfg.rayon / 2000.0).ceil() as i32;
                (-r..=r).flat_map(|dy| (-r..=r).map(move |dx| c.voisine(dx, dy))).filter(|t| {
                    let (x, y) = t.centre();
                    ((x - cam.cible[0]).powi(2) + (y - cam.cible[1]).powi(2)).sqrt() <= cfg.rayon / 2.0
                }).filter(|t| !ch.affichees.contains_key(t) || ch.file.iter().any(|f| f.0 == *t)).count()
            };
            mes.frames.push((t, dt, ch.taches.len(), ch.affichees.len(), voulues_proches, o.ms_dernier > 0.0));
            let [x0, y0, x1, y1] = cfg.trajet;
            let l = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
            if (t - 0.5) * cfg.vitesse >= l + cfg.vitesse * 2.0 {
                let nom = cfg.res.join(format!("survol-{}.csv", cfg.etiquette));
                let mut s = String::from("t_s,frame_ms,taches,tiles_affichees,proches_manquantes,recalage\n");
                for x in &mes.frames {
                    s.push_str(&format!("{:.3},{:.3},{},{},{},{}\n", x.0, x.1, x.2, x.3, x.4, x.5 as u8));
                }
                std::fs::write(&nom, s).unwrap();
                let mut f: Vec<f64> = mes.frames.iter().map(|x| x.1).collect();
                let mut sans: Vec<f64> = mes.frames.iter().filter(|x| x.2 == 0).map(|x| x.1).collect();
                let mut avec: Vec<f64> = mes.frames.iter().filter(|x| x.2 > 0).map(|x| x.1).collect();
                let rec: Vec<f64> = mes.frames.iter().filter(|x| x.5).map(|x| x.1).collect();
                let manq = mes.frames.iter().filter(|x| x.4 > 0).count();
                let mut g = ch.ms_gen.clone();
                let mut env = ch.ms_envoi.clone();
                if cfg.entete {
                    println!(
                        "etiquette,organisation,lod,pregenere,octets_par_frame,dalles,vitesse_ms,altitude_m,rayon_m,frames,frame_med_ms,frame_p99_ms,frame_max_ms,sans_gen_med,sans_gen_p99,avec_gen_med,avec_gen_p99,frames_avec_gen,frames_au_dessus_16_6,recalages,frame_recalage_max_ms,frames_proches_manquantes,tiles_generees,gen_tile_med_ms,gen_tile_max_ms,envoi_med_ms,envoi_max_ms"
                    );
                }
                let n = f.len();
                let au_dessus = f.iter().filter(|x| **x > 16.6).count();
                println!(
                    "{},{},{},{},{},{},{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{},{},{},{:.2},{},{},{:.1},{:.1},{:.2},{:.2}",
                    cfg.etiquette,
                    cfg.organisation,
                    cfg.lod,
                    cfg.pregen,
                    cfg.octets_par_frame,
                    cfg.dalles,
                    cfg.vitesse,
                    cfg.altitude,
                    cfg.rayon,
                    n,
                    quantile(&mut f, 0.5),
                    quantile(&mut f, 0.99),
                    f.last().unwrap(),
                    quantile(&mut sans, 0.5),
                    quantile(&mut sans, 0.99),
                    quantile(&mut avec, 0.5),
                    quantile(&mut avec, 0.99),
                    avec.len(),
                    au_dessus,
                    o.recalages,
                    rec.iter().copied().fold(0.0, f64::max),
                    manq,
                    ch.recues,
                    quantile(&mut g, 0.5),
                    g.last().copied().unwrap_or(f64::NAN),
                    quantile(&mut env, 0.5),
                    env.last().copied().unwrap_or(f64::NAN)
                );
                mes.fini = true;
                exit.write(AppExit::Success);
            }
        }
        "capture" | "rendu-destruction" => {
            // Phases : attente, capture « avant », (destruction : endommager,
            // capture, effondrer, capture), fin.
            if mes.en_vol {
                let img = mes.capture.lock().unwrap().take();
                if let Some(img) = img {
                    mes.en_vol = false;
                    let nom = match (cfg.mode.as_str(), mes.phase) {
                        ("capture", _) => format!("{}-{}.png", cfg.etiquette, cfg.vue),
                        (_, 0) => format!("{}-intact.png", cfg.etiquette),
                        (_, 1) => format!("{}-endommage.png", cfg.etiquette),
                        _ => format!("{}-effondre.png", cfg.etiquette),
                    };
                    // PNG écrit dans un thread à part : l'encodage ne compte pas dans les frames.
                    let chemin = cfg.res.join(nom);
                    mes.ecritures.push(std::thread::spawn(move || png(&chemin, &img)));
                    mes.phase += 1;
                    mes.etape = 0;
                    if cfg.mode == "capture" || mes.phase >= 3 {
                        if cfg.mode == "rendu-destruction" {
                            let mut f: Vec<f64> = mes.frames.iter().map(|x| x.1).collect();
                            let mx = f.iter().copied().fold(0.0, f64::max);
                            let ma = mes.apres_maj.iter().copied().fold(0.0, f64::max);
                            if cfg.entete {
                                println!("etiquette,organisation,frames,frame_med_ms,frame_max_ms,frames_apres_maj_max_ms,ms_maj_cpu");
                            }
                            println!("{},{},{},{:.2},{:.2},{:.2},{:.2}", cfg.etiquette, cfg.organisation, f.len(), quantile(&mut f, 0.5), mx, ma, mes.ms_maj);
                        }
                        // Attendre l'écriture des images avant de quitter.
                        for h in mes.ecritures.drain(..) {
                            let _ = h.join();
                        }
                        mes.fini = true;
                        exit.write(AppExit::Success);
                    }
                }
                return;
            }
            // Frames mesurées loin des captures (lecture de l'image) ; les six
            // frames qui suivent une mise à jour sont gardées à part.
            if cfg.mode == "rendu-destruction" && (10..110).contains(&mes.etape) {
                if mes.phase > 0 && (61..=66).contains(&mes.etape) {
                    mes.apres_maj.push(dt);
                } else {
                    mes.frames.push((t, dt, 0, 0, 0, false));
                }
            }
            if mes.etape == 60 {
                // Destruction : état du bâtiment choisi, puis régénération de
                // la tile (ou du morceau, ou de l'entité) qui le contient.
                if cfg.mode == "rendu-destruction" && mes.phase > 0 {
                    let mut vic = vic.unwrap();
                    let vk = vic.k;
                    let id = vic.ctx.tile().bats[vk].id;
                    m.etats.insert(id, if mes.phase == 1 { Etat::Endommage } else { Etat::Effondre });
                    let t0 = Instant::now();
                    let p = Params::default();
                    let s = {
                        let g = Gen { ctx: &vic.ctx, prep: &vic.prep, p: &p, lod: m.lod, etats: &m.etats };
                        g.bat(vk)
                    };
                    vic.sorties[vk] = s;
                    let (part, me) = match m.org {
                        Organisation::Tuile => (0, generation::fusionner(&vic.sorties).0),
                        Organisation::Batiment => (vk, vic.sorties[vk].mesh.clone()),
                        Organisation::Morceaux => {
                            let cle = vic.morceaux[vk];
                            let rang = vic.morceaux.iter().collect::<std::collections::BTreeSet<_>>().iter().position(|c| **c == cle).unwrap();
                            let mut me = Maillage::default();
                            for (j, x) in vic.sorties.iter().enumerate() {
                                if vic.morceaux[j] == cle {
                                    me.ajouter(&x.mesh);
                                }
                            }
                            (rang, me)
                        }
                    };
                    let h = meshes.add(mesh_bevy(&me, m.compresse));
                    for (a, pa, mut mh) in &mut qp {
                        if a.t == cfg.tiles[0] && pa.0 == part {
                            mh.0 = h.clone();
                        }
                    }
                    let _ = &q;
                    mes.ms_maj = mes.ms_maj.max(t0.elapsed().as_secs_f64() * 1e3);
                }
            }
            if mes.etape == 120 {
                let slot = mes.capture.clone();
                commands.spawn(Screenshot::image(v.cible.clone())).observe(move |ev: On<ScreenshotCaptured>| {
                    *slot.lock().unwrap() = Some(ev.image.clone());
                });
                mes.en_vol = true;
            }
        }
        _ => {}
    }
}

/// Bâtiment détruit : le plus grand à moins de 150 m du centre de la tile.
pub fn choisir_victime(bats: &[crate::donnees::Bat]) -> u64 {
    bats.iter()
        .filter(|b| {
            let c = crate::porte::centroide(b);
            c[0].abs() < 150.0 && c[1].abs() < 150.0
        })
        .max_by(|a, b| a.aire().partial_cmp(&b.aire()).unwrap())
        .map(|b| b.id)
        .unwrap_or(bats[0].id)
}
