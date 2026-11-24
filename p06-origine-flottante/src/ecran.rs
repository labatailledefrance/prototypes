//! Modes fenêtrés : rendu réel par Bevy sur la RTX 3060.
//!
//! - `voir` : vérification à l'œil, une scène à l'écran (touches ci-dessous) ;
//! - `capture` : validation de la relecture par le rendu réel (centroïde d'un
//!   marqueur magenta dans les captures, comparé à la projection f64) ;
//! - `profondeur` : z-fighting de deux plaques parallèles, distance × écart ;
//! - `ombres` : image rendue selon la distance caméra–origine, comparée à
//!   l'image à distance nulle, avec et sans ombres ;
//! - `cadence` : temps de frame avec n modèles affichés.
//!
//! Touches du mode `voir` : N stratégie suivante (naïves, chaque frame,
//! seuil), T horloge des shaders (f32 absolue ou phase), flèches haut/bas
//! zoom, +/− accélération, espace pause, Échap quitter.

use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::CascadeShadowConfigBuilder;
use bevy::math::{Affine2, DQuat, DVec2, DVec3};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::{PresentMode, WindowResolution};
use big_space::prelude::*;

use crate::geo;
use crate::monde::{self, Ancre, CamRig, CameraJeu, Horloge, ModeFenetre, Options, Origine, Reglage, Releve, Scenario, Strategie, Visuel, Zoom};
use crate::stats::{self, f};
use crate::Args;

#[derive(Resource, Clone)]
struct Config {
    mode: String,
    ombres: bool,
    etiquette: String,
    entete: bool,
    sortie: String,
    frames: u32,
    n: usize,
}

#[derive(Component)]
struct Marqueur;

#[derive(Component)]
struct Eau;

#[derive(Component)]
struct Lumiere(DQuat);

#[derive(Component)]
struct Texte;

#[derive(Resource)]
struct Visuels {
    soldat: Handle<Mesh>,
    tete: Handle<Mesh>,
    sol: Handle<Mesh>,
    coque: Handle<Mesh>,
    cube: Handle<Mesh>,
    m_soldat: Handle<StandardMaterial>,
    m_tete: Handle<StandardMaterial>,
    m_sol: Handle<StandardMaterial>,
    m_bloc: Handle<StandardMaterial>,
    m_coque: Handle<StandardMaterial>,
    m_globe: Handle<StandardMaterial>,
    m_eau: Handle<StandardMaterial>,
}

#[derive(Resource)]
struct HorlogeShader {
    phase: bool,
}

pub fn lancer(a: &Args) {
    let strat = Strategie::lire(&a.strategie);
    let temps_reel = a.mode == "voir" || a.mode == "cadence";
    let titre = format!("P-06 — {} — {} — {}", a.mode, a.lieu, strat.nom());
    let fenetre = Window {
        title: titre,
        resolution: WindowResolution::new(monde::LARGEUR as u32, monde::HAUTEUR as u32).with_scale_factor_override(1.0),
        present_mode: if std::env::var("P06_PRESENT").as_deref() == Ok("mailbox") { PresentMode::Mailbox } else { PresentMode::Immediate },
        resizable: false,
        ..default()
    };
    // Cadence : rendu hors écran dans une image, sans winit. Sous Wayland, winit
    // cale les frames sur le compositeur (16,6 ms), quel que soit le mode de
    // présentation.
    let hors_ecran = a.mode == "cadence";
    let fenetres = if hors_ecran {
        WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() }
    } else {
        WindowPlugin { primary_window: Some(fenetre), ..default() }
    };
    let mut base = DefaultPlugins.set(fenetres).set(TaskPoolPlugin { task_pool_options: TaskPoolOptions::with_num_threads(a.threads) }).build();
    if hors_ecran {
        base = base.disable::<bevy::winit::WinitPlugin>();
    }
    let mut app = App::new();
    if strat.big_space() {
        app.add_plugins((base.disable::<TransformPlugin>(), BigSpaceDefaultPlugins));
    } else {
        app.add_plugins(base);
    }
    if hors_ecran {
        app.add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(std::time::Duration::ZERO));
    }
    let scenario = match a.mode.as_str() {
        "profondeur" => Scenario::Vide,
        "cadence" => Scenario::Cout { n: a.n, mobiles: a.mobiles, vitesse: a.vitesse },
        _ => crate::scenario_de(a),
    };
    let acceleration = if a.mode == "voir" && a.scenario == "navire" { a.acceleration.max(200.0) } else { a.acceleration };
    monde::installer(
        &mut app,
        &Options { strat, scenario, t0: a.jour * 86400.0, acceleration, lot: a.lot, fenetre: true },
    );
    app.insert_resource(ModeFenetre { temps_reel })
        .insert_resource(ClearColor(if a.mode == "capture" || a.mode == "profondeur" { Color::srgb(0.0, 0.0, 0.3) } else { Color::srgb(0.55, 0.7, 0.9) }))
        .insert_resource(HorlogeShader { phase: true })
        .insert_resource(Config {
            mode: a.mode.clone(),
            ombres: a.ombres,
            etiquette: a.etiquette.clone(),
            entete: a.entete,
            sortie: a.sortie.clone(),
            frames: a.frames,
            n: a.n,
        })
        .add_systems(Startup, (preparer_visuels, globe.after(monde::installer_scene)))
        .add_systems(PostStartup, (camera_et_lumiere, texte))
        .add_systems(Update, (habiller, orienter_lumiere.after(monde::maj_origine), animer_eau, afficher))
        .add_systems(Last, diagnostic);
    match a.mode.as_str() {
        "voir" => {
            app.add_systems(Update, clavier.before(monde::avancer));
            app.world_mut().resource_mut::<Releve>().actif = true;
        }
        "capture" => {
            app.insert_resource(Capture::default()).add_systems(Update, capture.after(monde::poser_scene));
            app.world_mut().resource_mut::<Releve>().actif = true;
        }
        "profondeur" => {
            app.insert_resource(Profondeur::default()).add_systems(Update, profondeur.before(monde::avancer));
        }
        "ombres" => {
            app.insert_resource(Ombres::default()).add_systems(Update, ombres.after(monde::poser_scene));
        }
        "cadence" => {
            app.insert_resource(Cadence::default()).add_systems(Update, cadence);
        }
        m => panic!("mode inconnu : {m}"),
    }
    app.run();
}

// ------------------------------------------------------------------ Visuels

fn preparer_visuels(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    cfg: Res<Config>,
) {
    let plat = cfg.mode == "capture" || cfg.mode == "profondeur";
    let mat = |mats: &mut Assets<StandardMaterial>, c: Color| {
        mats.add(StandardMaterial { base_color: c, unlit: plat, perceptual_roughness: 0.9, cull_mode: None, double_sided: true, ..default() })
    };
    // Texture rayée pour l'eau (défilement piloté par l'horloge des shaders).
    let mut data = vec![];
    for y in 0..8u32 {
        for x in 0..64u32 {
            let v = if (x / 8) % 2 == 0 { 40 } else { 160 };
            let _ = y;
            data.extend_from_slice(&[v / 2, v, 255, 255]);
        }
    }
    let mut img = Image::new(
        Extent3d { width: 64, height: 8, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..default()
    });
    let tex = images.add(img);
    let v = Visuels {
        soldat: meshes.add(Mesh::from(Capsule3d::new(0.3, 1.2)).translated_by(Vec3::Y * 0.9)),
        tete: meshes.add(Sphere::new(0.2).mesh().uv(32, 18)),
        sol: meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(500.0)).mesh().subdivisions(49)),
        coque: meshes.add(Mesh::from(Cuboid::new(20.0, 12.0, 120.0)).translated_by(Vec3::Y * 2.0)),
        cube: meshes.add(Cuboid::new(2.0, 2.0, 2.0)),
        m_soldat: mat(&mut mats, Color::srgb(0.25, 0.3, 0.2)),
        m_tete: mats.add(StandardMaterial { base_color: Color::srgb(1.0, 0.0, 1.0), unlit: true, ..default() }),
        m_sol: mat(&mut mats, Color::srgb(0.4, 0.4, 0.38)),
        m_bloc: mat(&mut mats, Color::srgb(0.75, 0.72, 0.68)),
        m_coque: mat(&mut mats, Color::srgb(0.3, 0.32, 0.36)),
        m_globe: mat(&mut mats, Color::srgb(0.1, 0.25, 0.45)),
        m_eau: mats.add(StandardMaterial { base_color_texture: Some(tex), unlit: plat, ..default() }),
    };
    commands.insert_resource(v);
}

/// Globe découpé en morceaux à origine locale (cube projeté sur l'ellipsoïde).
fn globe(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    cfg: Res<Config>,
    reglage: Res<Reglage>,
    racine: Option<Res<monde::RacineBig>>,
) {
    if cfg.mode == "capture" || cfg.mode == "profondeur" || cfg.mode == "cadence" {
        return;
    }
    let _ = reglage;
    for face in 0..6 {
        for i in 0..monde::GLOBE_N {
            for j in 0..monde::GLOBE_N {
                let (centre, sommets, idx) = monde::morceau_globe(face, i, j);
                let normales: Vec<Vec3> = sommets.iter().map(|s| (s.as_dvec3() + centre).normalize().as_vec3()).collect();
                let m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
                    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, sommets)
                    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normales)
                    .with_inserted_indices(Indices::U32(idx));
                let h = meshes.add(m);
                let mut e = commands.spawn((Ancre::Global { ecef: centre, rot: DQuat::IDENTITY }, Transform::default(), Visuel::Globe(h)));
                if let Some(r) = &racine {
                    e.insert((CellCoord::default(), ChildOf(r.0)));
                }
            }
        }
    }
}

fn habiller(mut commands: Commands, v: Res<Visuels>, q: Query<(Entity, &Visuel), Added<Visuel>>, mut meshes: ResMut<Assets<Mesh>>) {
    for (e, vis) in &q {
        let mut ec = commands.entity(e);
        match vis {
            Visuel::Soldat => {
                ec.insert((Mesh3d(v.soldat.clone()), MeshMaterial3d(v.m_soldat.clone())));
                ec.with_child((Mesh3d(v.tete.clone()), MeshMaterial3d(v.m_tete.clone()), Transform::from_xyz(0.0, 2.1, 0.0), Marqueur));
            }
            Visuel::Sol => {
                ec.insert((Mesh3d(v.sol.clone()), MeshMaterial3d(v.m_sol.clone())));
            }
            Visuel::Bloc(t) => {
                let m = meshes.add(Mesh::from(Cuboid::new(t.x, t.y, t.z)).translated_by(Vec3::Y * t.y / 2.0));
                ec.insert((Mesh3d(m), MeshMaterial3d(v.m_bloc.clone())));
            }
            Visuel::Coque => {
                ec.insert((Mesh3d(v.coque.clone()), MeshMaterial3d(v.m_coque.clone())));
            }
            Visuel::Cube => {
                ec.insert((Mesh3d(v.cube.clone()), MeshMaterial3d(v.m_bloc.clone())));
            }
            Visuel::Globe(h) => {
                ec.insert((Mesh3d(h.clone()), MeshMaterial3d(v.m_globe.clone())));
            }
            Visuel::Plaque(..) => {}
        }
    }
}

fn camera_et_lumiere(
    mut commands: Commands,
    cfg: Res<Config>,
    q: Query<Entity, With<CamRig>>,
    scene: Res<monde::Scene>,
    mut tiles: ResMut<monde::Tiles>,
    v: Res<Visuels>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut parents: ResMut<monde::ParentsTiles>,
    reglage: Res<Reglage>,
    racine: Option<Res<monde::RacineBig>>,
    mut images: ResMut<Assets<Image>>,
) {
    let plat = cfg.mode == "capture" || cfg.mode == "profondeur";
    let cible = (cfg.mode == "cadence").then(|| {
        let mut img = Image::new_target_texture(monde::LARGEUR as u32, monde::HAUTEUR as u32, TextureFormat::Rgba8UnormSrgb, None);
        img.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::COPY_SRC;
        bevy::camera::RenderTarget::Image(images.add(img).into())
    });
    for e in &q {
        if let Some(c) = &cible {
            commands.entity(e).insert(c.clone());
        }
        commands.entity(e).insert((
            Camera3d::default(),
            Projection::Perspective(monde::projection32(monde::FOV_DEG.to_radians())),
            Msaa::Off,
            if plat { Tonemapping::None } else { Tonemapping::default() },
        ));
    }
    if !plat {
        // Soleil : azimut 135°, élévation 40°, dans le repère local de la scène.
        let lieu = match &scene.0 {
            Scenario::Soldat { lieu, .. } | Scenario::Zoom { lieu, .. } => geo::lieu(lieu).g,
            _ => geo::lieu("Brest").g,
        };
        let (pos, _) = monde::rig_iso(geo::vers_ecef(lieu), 1.0, 135.0, 40.0);
        let dir = geo::vers_ecef(lieu) - pos; // du soleil vers le sol
        let rot = monde::regarder(DVec3::ZERO, dir, geo::verticale(lieu));
        let ombres = cfg.mode != "ombres" || cfg.ombres;
        commands.spawn((
            DirectionalLight { shadow_maps_enabled: ombres, illuminance: 12_000.0, ..default() },
            CascadeShadowConfigBuilder { maximum_distance: 400.0, first_cascade_far_bound: 20.0, ..default() }.build(),
            Transform::default(),
            Lumiere(rot),
        ));
        commands.insert_resource(GlobalAmbientLight { brightness: 400.0, ..default() });
    }
    // Bassin d'eau près de la rue (horloge des shaders).
    if let (Scenario::Soldat { lieu, .. }, "voir") = (&scene.0, cfg.mode.as_str()) {
        let l = geo::lieu(lieu);
        let z = geo::zone_par_epsg(l.epsg);
        let t0 = geo::TileId::de_grille(l.epsg, z.proj.avant(l.g));
        tiles.assurer(t0);
        let tex = mats.get(&v.m_eau).unwrap().base_color_texture.clone();
        let m = mats.add(StandardMaterial { base_color_texture: tex, ..default() });
        let mesh = v.sol.clone();
        let e = commands
            .spawn((
                Ancre::Tile { id: t0, local: Vec3::new(1000.0 - 30.0, 0.05, -500.0 + 40.0), rot: Quat::IDENTITY },
                Transform::default().with_scale(Vec3::new(0.02, 1.0, 0.02)),
                Mesh3d(mesh),
                MeshMaterial3d(m),
                Eau,
            ))
            .id();
        if let Some(r) = &racine {
            commands.entity(e).insert((CellCoord::default(), ChildOf(r.0)));
        } else if reglage.strat.hierarchie() {
            let p = monde::parent_de(&mut commands, &mut parents, t0);
            commands.entity(e).insert(ChildOf(p));
        }
    }
}

fn orienter_lumiere(o: Res<Origine>, reglage: Res<Reglage>, mut q: Query<(&Lumiere, &mut Transform)>) {
    let qr = if reglage.strat.big_space() { DQuat::from_mat3(&geo::bevy_depuis_ecef_fixe()) } else { o.qrot };
    for (l, mut t) in &mut q {
        t.rotation = (qr * l.0).normalize().as_quat();
    }
}

fn animer_eau(h: Res<Horloge>, hs: Res<HorlogeShader>, q: Query<&MeshMaterial3d<StandardMaterial>, With<Eau>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    for m in &q {
        if let Some(mut mat) = mats.get_mut(&m.0) {
            // Défilement d'une texture de 10 s de période, calculé comme le ferait un shader.
            let phase = if hs.phase {
                (((h.tick % 200) as f64 + h.alpha) / 200.0) as f32
            } else {
                (h.temps() as f32 * 0.1).fract()
            };
            mat.uv_transform = Affine2::from_scale(Vec2::new(50.0, 50.0)) * Affine2::from_translation(Vec2::new(phase, 0.0));
        }
    }
}

fn texte(mut commands: Commands, cfg: Res<Config>) {
    if cfg.mode != "voir" {
        return;
    }
    commands.spawn((
        Text::new(""),
        TextFont { font_size: bevy::text::FontSize::Px(20.0), ..default() },
        TextColor(Color::WHITE),
        Node { position_type: PositionType::Absolute, top: Val::Px(10.0), left: Val::Px(10.0), ..default() },
        Texte,
    ));
}

#[allow(clippy::too_many_arguments)]
fn afficher(
    reglage: Res<Reglage>,
    rel: Res<Releve>,
    h: Res<Horloge>,
    hs: Res<HorlogeShader>,
    cam: Res<CameraJeu>,
    time: Res<Time<Real>>,
    scene: Res<monde::Scene>,
    mut q: Query<&mut Text, With<Texte>>,
) {
    let Ok(mut t) = q.single_mut() else { return };
    let g = geo::depuis_ecef(cam.pos);
    let d_paris = geo::distance_gc(geo::lieu("Paris").g, g) / 1000.0;
    let lieu = match &scene.0 {
        Scenario::Soldat { lieu, .. } | Scenario::Zoom { lieu, .. } => lieu.clone(),
        Scenario::Navire => "navire".into(),
        _ => String::new(),
    };
    **t = format!(
        "{lieu} — stratégie {} (N)\nCaméra à {:.0} km de Paris, altitude {:.0} m\nÉcart max de la frame (relecture) : {:.3} px\nTemps de jeu : jour {:.2}, ×{} {}\nHorloge des shaders (T) : {}\n{:.0} images/s",
        reglage.strat.nom(),
        d_paris,
        g.h,
        rel.derniere_max_px,
        h.temps() / 86400.0,
        h.acceleration,
        if h.pause { "(pause)" } else { "" },
        if hs.phase { "phase calculée sur les ticks" } else { "f32 absolue (témoin)" },
        1.0 / time.delta_secs().max(1e-6)
    );
}

fn clavier(
    keys: Res<ButtonInput<KeyCode>>,
    mut reglage: ResMut<Reglage>,
    mut o: ResMut<Origine>,
    mut h: ResMut<Horloge>,
    mut hs: ResMut<HorlogeShader>,
    zoom: Option<ResMut<Zoom>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
    if keys.just_pressed(KeyCode::KeyN) && !reglage.strat.big_space() && !reglage.strat.hierarchie() {
        let liste = [Strategie::NaifParis, Strategie::NaifEcef, Strategie::ChaqueFrame, Strategie::Seuil(1000.0)];
        let i = liste.iter().position(|s| *s == reglage.strat).map(|i| (i + 1) % liste.len()).unwrap_or(0);
        reglage.strat = liste[i];
        o.init = false;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        hs.phase = !hs.phase;
    }
    if keys.just_pressed(KeyCode::Space) {
        h.pause = !h.pause;
    }
    if keys.just_pressed(KeyCode::NumpadAdd) || keys.just_pressed(KeyCode::Equal) {
        h.acceleration *= 2.0;
    }
    if keys.just_pressed(KeyCode::NumpadSubtract) || keys.just_pressed(KeyCode::Minus) {
        h.acceleration = (h.acceleration / 2.0).max(0.125);
    }
    let up = keys.pressed(KeyCode::ArrowUp);
    let down = keys.pressed(KeyCode::ArrowDown);
    if up || down {
        let k = if up { 1.0 / 1.04 } else { 1.04 };
        match zoom {
            Some(mut z) => z.d = (z.d * k).clamp(5.0, 4.0e7),
            None => commands.insert_resource(Zoom { d: 25.0 * k, auto: false }),
        }
    }
}

// ------------------------------------------------------------------ Captures

/// Les pipelines de shaders se compilent en tâche de fond : rien n'est dessiné
/// pendant les premières secondes.
fn pret(reel: &Time<Real>) -> bool {
    reel.elapsed_secs() > 6.0
}

/// Centroïde des pixels magenta (le marqueur) : (x, y, nombre).
fn centroide(img: &Image) -> Option<(f64, f64, usize)> {
    let w = img.width() as usize;
    let data = img.data.as_ref()?;
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0usize);
    for (i, p) in data.chunks_exact(4).enumerate() {
        // Magenta : symétrique en R et B, donc indépendant de l'ordre RGBA / BGRA.
        if p[0] > 200 && p[2] > 200 && p[1] < 60 {
            sx += (i % w) as f64 + 0.5;
            sy += (i / w) as f64 + 0.5;
            n += 1;
        }
    }
    (n > 0).then(|| (sx / n as f64, sy / n as f64, n))
}

/// Projection de référence f64 d'un point ECEF, en pixels (y vers le bas).
fn px_ref(cam: &CameraJeu, p: DVec3) -> Option<DVec2> {
    let v = cam.rot.inverse() * (p - cam.pos);
    if v.z > -0.1 {
        return None;
    }
    let f = 1.0 / (cam.fov / 2.0).tan();
    let aspect = (monde::LARGEUR / monde::HAUTEUR) as f64;
    let ndc = DVec2::new(f / aspect * v.x / -v.z, f * v.y / -v.z);
    Some(DVec2::new((ndc.x + 1.0) / 2.0 * monde::LARGEUR as f64, (1.0 - ndc.y) / 2.0 * monde::HAUTEUR as f64))
}

/// Projection f32 relue sur les `GlobalTransform`, en pixels.
fn px_emul(cgt: &GlobalTransform, cam: &CameraJeu, w32: Vec3) -> Vec2 {
    use bevy::camera::CameraProjection;
    let clip = monde::projection32(cam.fov).get_clip_from_view() * cgt.to_matrix().inverse() * w32.extend(1.0);
    Vec2::new((clip.x / clip.w + 1.0) / 2.0 * monde::LARGEUR, (1.0 - clip.y / clip.w) / 2.0 * monde::HAUTEUR)
}

#[derive(Resource, Default)]
struct Capture {
    etape: u32,
    attente: u32,
    pose: u32,
    en_vol: bool,
    resultat: std::sync::Arc<std::sync::Mutex<Option<Image>>>,
    reference: Option<DVec2>,
    emule: Option<Vec2>,
    gpu_ref: Vec<f64>,
    emul_ref: Vec<f64>,
    gpu_emul: Vec<f64>,
    pixels: Vec<f64>,
}

#[allow(clippy::too_many_arguments)]
fn capture(
    mut commands: Commands,
    mut c: ResMut<Capture>,
    mut h: ResMut<Horloge>,
    cfg: Res<Config>,
    cam: Res<CameraJeu>,
    reglage: Res<Reglage>,
    scene: Res<monde::Scene>,
    q_cam: Query<&GlobalTransform, With<CamRig>>,
    q_m: Query<&GlobalTransform, With<Marqueur>>,
    q_s: Query<&Ancre, With<monde::Mobile>>,
    tiles: Res<monde::Tiles>,
    reel: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
) {
    c.etape += 1;
    if !pret(&reel) {
        return; // compilation asynchrone des pipelines de shaders
    }
    let c = &mut *c;
    if !c.en_vol {
        if !h.pause {
            // Avance de quelques frames entre deux poses.
            c.attente += 1;
            if c.attente < 7 {
                return;
            }
            h.pause = true;
            c.attente = 0;
            return;
        }
        // Pose figée : attendre que le rendu ait rattrapé l'état.
        c.attente += 1;
        if c.attente < 4 {
            return;
        }
        c.attente = 0;
        // Référence et relecture pour la pose figée.
        let soldat = q_s.iter().find(|a| matches!(a, Ancre::Tile { .. }));
        let (Some(a), Ok(cgt), Ok(mgt)) = (soldat, q_cam.single(), q_m.single()) else { return };
        let (p, r) = monde::pose(a, &tiles);
        let p_tete = p + r * DVec3::new(0.0, 2.1, 0.0);
        c.reference = px_ref(&cam, p_tete);
        c.emule = Some(px_emul(cgt, &cam, mgt.translation()));
        let slot = c.resultat.clone();
        commands.spawn(Screenshot::primary_window()).observe(move |ev: On<ScreenshotCaptured>| {
            *slot.lock().unwrap() = Some(ev.image.clone());
        });
        c.en_vol = true;
        return;
    }
    let img = c.resultat.lock().unwrap().take();
    let Some(img) = img else { return };
    c.en_vol = false;
    h.pause = false;
    if c.pose == 0 && !cfg.sortie.is_empty() {
        let _ = std::fs::create_dir_all(&cfg.sortie);
        let nom = format!("{}/capture-{}.png", cfg.sortie, reglage.strat.nom().replace(':', "-"));
        let _ = img.clone().try_into_dynamic().map(|d| d.to_rgb8().save(nom));
        eprintln!("capture : format {:?}, {}×{}, référence {:?}, relecture {:?}", img.texture_descriptor.format, img.width(), img.height(), c.reference, c.emule);
    }
    if let (Some((x, y, n)), Some(r), Some(e)) = (centroide(&img), c.reference, c.emule) {
        let g = DVec2::new(x, y);
        c.gpu_ref.push((g - r).length());
        c.emul_ref.push((e.as_dvec2() - r).length());
        c.gpu_emul.push((g - e.as_dvec2()).length());
        c.pixels.push(n as f64);
    }
    c.pose += 1;
    if c.pose >= cfg.frames {
        let lieu = match &scene.0 {
            Scenario::Soldat { lieu, .. } => lieu.clone(),
            _ => String::new(),
        };
        let (a, b, d, p) = (stats::resumer(&c.gpu_ref), stats::resumer(&c.emul_ref), stats::resumer(&c.gpu_emul), stats::resumer(&c.pixels));
        if cfg.entete {
            println!("etiquette,lieu,strategie,captures,pixels_marqueur,gpu_ref_med,gpu_ref_p99,gpu_ref_max,relecture_ref_med,relecture_ref_max,gpu_relecture_med,gpu_relecture_p99,gpu_relecture_max");
        }
        println!(
            "{},{},{},{},{:.0},{},{},{},{},{},{},{},{}",
            cfg.etiquette,
            lieu,
            reglage.strat.nom(),
            a.n,
            p.mediane,
            f(a.mediane),
            f(a.p99),
            f(a.max),
            f(b.mediane),
            f(b.max),
            f(d.mediane),
            f(d.p99),
            f(d.max)
        );
        exit.write(AppExit::Success);
    }
}

// ------------------------------------------------------------------ Profondeur

#[derive(Component)]
struct Plaque(bool);

#[derive(Resource, Default)]
struct Profondeur {
    etape: u32,
    config: usize,
    attente: u32,
    en_vol: bool,
    resultat: std::sync::Arc<std::sync::Mutex<Option<Image>>>,
    plaques: Option<(Entity, Entity)>,
    lignes: Vec<String>,
}

const DISTANCES: [f64; 7] = [30.0, 300.0, 3_000.0, 30_000.0, 300_000.0, 3_000_000.0, 20_000_000.0];
const ECARTS: [f64; 9] = [0.0001, 0.001, 0.01, 0.1, 1.0, 10.0, 100.0, 1_000.0, 10_000.0];

#[allow(clippy::too_many_arguments)]
fn profondeur(
    mut commands: Commands,
    mut p: ResMut<Profondeur>,
    mut cam: ResMut<CameraJeu>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    reglage: Res<Reglage>,
    cfg: Res<Config>,
    mut q: Query<(&Plaque, &mut Ancre, &mut Mesh3d)>,
    racine: Option<Res<monde::RacineBig>>,
    reel: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
) {
    p.etape += 1;
    let lieu = geo::lieu("Papeete").g;
    let centre = geo::vers_ecef(lieu);
    let rot_loc = DQuat::from_mat3(&geo::bevy_depuis_ecef(lieu).transpose());
    if p.plaques.is_none() {
        let m = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(1.0)).mesh());
        let mut mk = |haut: bool, c: Color| {
            let mat = mats.add(StandardMaterial { base_color: c, unlit: true, cull_mode: None, ..default() });
            let mut e = commands.spawn((
                Ancre::Global { ecef: centre, rot: rot_loc },
                Transform::default(),
                Mesh3d(m.clone()),
                MeshMaterial3d(mat),
                Plaque(haut),
                monde::Mobile,
            ));
            if let Some(r) = &racine {
                e.insert((CellCoord::default(), ChildOf(r.0)));
            }
            e.id()
        };
        let a = mk(false, Color::srgb(0.0, 1.0, 0.0));
        let b = mk(true, Color::srgb(1.0, 0.0, 0.0));
        p.plaques = Some((a, b));
        return;
    }
    if !pret(&reel) {
        return;
    }
    let (i_d, i_e) = (p.config / ECARTS.len(), p.config % ECARTS.len());
    if i_d >= DISTANCES.len() {
        let mut s = String::new();
        if cfg.entete {
            s.push_str("etiquette,strategie,distance_m,ecart_m,pct_dessous_visible,resolution_profondeur_m,ulp_sommets_m,valide\n");
        }
        for l in &p.lignes {
            s.push_str(l);
            s.push('\n');
        }
        print!("{s}");
        exit.write(AppExit::Success);
        return;
    }
    let (d, e) = (DISTANCES[i_d], ECARTS[i_e]);
    // Plaques de côté 1,5 d (maillage unitaire mis à l'échelle), caméra iso à d.
    let taille = 1.5 * d;
    let mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(taille as f32 / 2.0)).mesh());
    for (pl, mut a, mut m) in &mut q {
        let h = if pl.0 { e } else { 0.0 };
        *a = Ancre::Global { ecef: centre + geo::verticale(lieu) * h, rot: rot_loc };
        m.0 = mesh.clone();
    }
    let (pos, rot) = monde::rig_iso(centre, d, 45.0, 35.264);
    cam.pos = pos;
    cam.rot = rot;
    if !p.en_vol {
        p.attente += 1;
        if p.attente < 5 {
            return;
        }
        p.attente = 0;
        let slot = p.resultat.clone();
        commands.spawn(Screenshot::primary_window()).observe(move |ev: On<ScreenshotCaptured>| {
            *slot.lock().unwrap() = Some(ev.image.clone());
        });
        p.en_vol = true;
        return;
    }
    let img = p.resultat.lock().unwrap().take();
    let Some(img) = img else { return };
    p.en_vol = false;
    // Zone centrale (40 % de l'écran) : part des pixels où la plaque du dessous gagne.
    let (w, hgt) = (img.width() as usize, img.height() as usize);
    let data = img.data.as_ref().unwrap();
    let (mut vert, mut total) = (0usize, 0usize);
    for y in (hgt * 3 / 10)..(hgt * 7 / 10) {
        for x in (w * 3 / 10)..(w * 7 / 10) {
            let px = &data[(y * w + x) * 4..(y * w + x) * 4 + 4];
            // Vert : G élevé ; rouge : R ou B élevé selon l'ordre des canaux.
            let g = px[1] > 128;
            let r = px[0] > 128 || px[2] > 128;
            if g || r {
                total += 1;
            }
            if g {
                vert += 1;
            }
        }
    }
    let d32 = monde::PRES / d as f32;
    let dn = f32::from_bits(d32.to_bits() - 1);
    let res = monde::PRES as f64 / dn as f64 - monde::PRES as f64 / d32 as f64;
    let l = format!(
        "{},{},{},{},{:.3},{},{},{}",
        cfg.etiquette,
        reglage.strat.nom(),
        d,
        e,
        100.0 * vert as f64 / total.max(1) as f64,
        f(res),
        f(geo::ulp32(d) / 2.0),
        // Au-delà, la plaque du haut sort du champ (caméra à 0,58 d de hauteur).
        (e < 0.1 * d) as u8
    );
    p.lignes.push(l);
    p.config += 1;
}

// ------------------------------------------------------------------ Ombres

#[derive(Resource, Default)]
struct Ombres {
    etape: u32,
    config: usize,
    attente: u32,
    en_vol: bool,
    resultat: std::sync::Arc<std::sync::Mutex<Option<Image>>>,
    reference: Option<Vec<u8>>,
    lignes: Vec<String>,
}

fn strategies_ombres() -> Vec<Strategie> {
    let mut v: Vec<Strategie> = [0.0, 10.0, 100.0, 1_000.0, 10_000.0, 100_000.0, 1_000_000.0].iter().map(|s| Strategie::Decalage(*s)).collect();
    v.push(Strategie::NaifParis);
    v.push(Strategie::NaifEcef);
    v
}

#[allow(clippy::too_many_arguments)]
fn ombres(
    mut commands: Commands,
    mut o: ResMut<Ombres>,
    mut h: ResMut<Horloge>,
    mut reglage: ResMut<Reglage>,
    mut origine: ResMut<Origine>,
    cfg: Res<Config>,
    scene: Res<monde::Scene>,
    reel: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
) {
    o.etape += 1;
    h.pause = true;
    if !pret(&reel) {
        return;
    }
    let liste = strategies_ombres();
    if o.config >= liste.len() {
        if cfg.entete {
            println!("etiquette,lieu,ombres,strategie,decalage_m,pct_pixels_differents,ecart_moyen,ecart_max");
        }
        for l in &o.lignes {
            println!("{l}");
        }
        exit.write(AppExit::Success);
        return;
    }
    if !o.en_vol {
        if o.attente == 0 {
            reglage.strat = liste[o.config];
            origine.init = false;
        }
        o.attente += 1;
        if o.attente < 8 {
            return;
        }
        o.attente = 0;
        let slot = o.resultat.clone();
        commands.spawn(Screenshot::primary_window()).observe(move |ev: On<ScreenshotCaptured>| {
            *slot.lock().unwrap() = Some(ev.image.clone());
        });
        o.en_vol = true;
        return;
    }
    let img = o.resultat.lock().unwrap().take();
    let Some(img) = img else { return };
    o.en_vol = false;
    let data = img.data.clone().unwrap();
    let s = liste[o.config];
    if o.reference.is_none() {
        o.reference = Some(data.clone());
    }
    let r = o.reference.as_ref().unwrap();
    let (mut diff, mut somme, mut max) = (0usize, 0u64, 0u8);
    let n = data.len() / 4;
    for (a, b) in data.chunks_exact(4).zip(r.chunks_exact(4)) {
        let m = (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap();
        if m > 8 {
            diff += 1;
        }
        somme += m as u64;
        max = max.max(m);
    }
    let lieu = match &scene.0 {
        Scenario::Soldat { lieu, .. } => lieu.clone(),
        _ => String::new(),
    };
    let dec = match s {
        Strategie::Decalage(x) => x,
        _ => f64::NAN,
    };
    if !cfg.sortie.is_empty() {
        let _ = std::fs::create_dir_all(&cfg.sortie);
        let _ = img.clone().try_into_dynamic().map(|d| d.to_rgb8().save(format!("{}/ombres-{}-{}.png", cfg.sortie, cfg.ombres as u8, s.nom().replace(':', "-"))));
    }
    o.lignes.push(format!(
        "{},{},{},{},{},{:.4},{:.4},{}",
        cfg.etiquette,
        lieu,
        cfg.ombres as u8,
        s.nom(),
        dec,
        100.0 * diff as f64 / n as f64,
        somme as f64 / n as f64,
        max
    ));
    o.config += 1;
}

// ------------------------------------------------------------------ Cadence

#[derive(Resource, Default)]
struct Cadence {
    etape: u32,
    debut: Option<Instant>,
    frames: Vec<f64>,
    precedent: Option<Instant>,
}

fn cadence(mut c: ResMut<Cadence>, cfg: Res<Config>, reglage: Res<Reglage>, mut chronos: ResMut<monde::Chronos>, mut exit: MessageWriter<AppExit>) {
    c.etape += 1;
    let now = Instant::now();
    if c.etape == 120 {
        c.debut = Some(now);
        chronos.actif = true;
    }
    if c.etape > 120
        && let Some(p) = c.precedent
    {
        c.frames.push((now - p).as_secs_f64() * 1000.0);
    }
    c.precedent = Some(now);
    if c.etape >= 120 + cfg.frames {
        let fr = stats::resumer(&c.frames);
        let tot: Vec<f64> = chronos.recalage.iter().zip(&chronos.propagation).map(|(a, b)| a + b).collect();
        let t = stats::resumer(&tot);
        if cfg.entete {
            println!("etiquette,strategie,n,frame_med_ms,frame_p99_ms,frame_max_ms,recalage_propagation_med_ms,recalage_propagation_p99_ms,recalage_propagation_max_ms");
        }
        println!(
            "{},{},{},{},{},{},{},{},{}",
            cfg.etiquette,
            reglage.strat.nom(),
            cfg.n,
            f(fr.mediane),
            f(fr.p99),
            f(fr.max),
            f(t.mediane),
            f(t.p99),
            f(t.max)
        );
        exit.write(AppExit::Success);
    }
}

fn diagnostic(mut n: Local<u32>, q: Query<(&GlobalTransform, &ViewVisibility, Option<&bevy::camera::primitives::Aabb>), With<Mesh3d>>, qc: Query<(&GlobalTransform, &Camera, &Projection)>) {
    *n += 1;
    if *n != 40 || std::env::var("P06_DIAG").is_err() {
        return;
    }
    let vis = q.iter().filter(|x| x.1.get()).count();
    eprintln!("diag : {} maillages, {} visibles, aabb {}", q.iter().count(), vis, q.iter().filter(|x| x.2.is_some()).count());
    for (gt, cam, p) in &qc {
        eprintln!("caméra : {:?} actif {} {:?}", gt.translation(), cam.is_active, p);
    }
    for (gt, v, a) in q.iter().take(3) {
        eprintln!("  {:?} {:?} {:?}", gt.translation(), v.get(), a);
    }
}
