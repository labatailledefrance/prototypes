//! Test 3 : un état fréquent (« en feu ») porté par un composant marqueur
//! ou par un champ dans un composant.
//!
//! - Marqueur : l'entité en feu porte `EnFeu`. Changer d'état déplace
//!   l'entité d'une table (archétype) à l'autre, via des commandes différées
//!   appliquées à la fin du tick. En échange, le système qui traite les
//!   entités en feu ne parcourt qu'elles.
//! - Champ : un composant `Etat { en_feu }` sur toutes les entités. Changer
//!   d'état est une simple écriture ; mais le système de traitement parcourt
//!   toutes les entités pour trouver celles qui sont en feu.

use bevy_ecs::batching::BatchingStrategy;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::MultiThreadedExecutor;
use bevy_tasks::{ComputeTaskPool, TaskPoolBuilder};
use clap::ValueEnum;

use crate::Banc;
use crate::logique::{empreinte, etat_initial};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Representation {
    Marqueur,
    Champ,
}

/// Part des entités en feu, maintenue stable pendant le test.
const PART_EN_FEU: f64 = 0.05;
/// Taille de lot minimale (décision F-02).
const LOT_MIN: usize = 4096;

#[derive(Component)]
struct Position([f64; 3]);
/// Non lue par les systèmes du test : présente pour que la taille des lignes
/// soit réaliste (le déplacement d'archétype copie toute la ligne).
#[derive(Component)]
#[allow(dead_code)]
struct Vitesse([f32; 3]);
#[derive(Component)]
struct Jauges([f32; 4]);
/// Identifiant stable, indépendant de l'ordre de création : la décision de
/// changer d'état est la même dans les deux représentations.
#[derive(Component)]
struct Id(u64);

#[derive(Component)]
struct EnFeu;
#[derive(Component)]
struct Etat {
    en_feu: bool,
}

#[derive(Resource)]
struct Params {
    tick: u64,
    /// Seuils sur un hachage 64 bits : probabilité d'allumage et d'extinction.
    seuil_allumage: u64,
    seuil_extinction: u64,
}

fn hacher(a: u64, b: u64) -> u64 {
    let mut z = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Décision déterministe : l'entité change-t-elle d'état à ce tick ?
#[inline(always)]
fn bascule(id: &Id, en_feu: bool, p: &Params) -> bool {
    let seuil = if en_feu {
        p.seuil_extinction
    } else {
        p.seuil_allumage
    };
    hacher(id.0, p.tick) < seuil
}

#[inline(always)]
fn bruler(j: &mut Jauges) {
    j.0[0] = (j.0[0] + 0.001).min(1.0); // fatigue
    j.0[1] = (j.0[1] - 0.001).max(0.0); // moral
}

fn lots() -> BatchingStrategy {
    BatchingStrategy::new().min_batch_size(LOT_MIN)
}

fn basculer_marqueur(q: Query<(Entity, &Id, Has<EnFeu>)>, pc: ParallelCommands, p: Res<Params>) {
    q.par_iter()
        .batching_strategy(lots())
        .for_each(|(e, id, en_feu)| {
            if bascule(id, en_feu, &p) {
                pc.command_scope(|mut c| {
                    if en_feu {
                        c.entity(e).remove::<EnFeu>();
                    } else {
                        c.entity(e).insert(EnFeu);
                    }
                });
            }
        });
}

fn bruler_marqueur(mut q: Query<&mut Jauges, With<EnFeu>>) {
    q.par_iter_mut()
        .batching_strategy(lots())
        .for_each(|mut j| bruler(&mut j));
}

fn basculer_champ(mut q: Query<(&Id, &mut Etat)>, p: Res<Params>) {
    q.par_iter_mut()
        .batching_strategy(lots())
        .for_each(|(id, mut etat)| {
            // N'écrit (et n'horodate) que si l'état change.
            if bascule(id, etat.en_feu, &p) {
                etat.en_feu = !etat.en_feu;
            }
        });
}

fn bruler_champ(mut q: Query<(&Etat, &mut Jauges)>) {
    q.par_iter_mut()
        .batching_strategy(lots())
        .for_each(|(etat, mut j)| {
            if etat.en_feu {
                bruler(&mut j);
            }
        });
}

fn avancer(mut p: ResMut<Params>) {
    p.tick += 1;
}

pub struct BancEtats {
    world: World,
    schedule: Schedule,
    repr: Representation,
}

impl BancEtats {
    /// `changements` : part des entités qui changent d'état à chaque tick.
    pub fn new(n: u64, repr: Representation, threads: usize, changements: f64) -> Self {
        // Régime stable : autant d'allumages que d'extinctions, et 5 % en feu.
        let allumage = (changements / (2.0 * (1.0 - PART_EN_FEU))).min(1.0);
        let extinction = (changements / (2.0 * PART_EN_FEU)).min(1.0);
        let seuil = |prob: f64| (prob * u64::MAX as f64) as u64;

        let mut world = World::new();
        world.insert_resource(Params {
            tick: 0,
            seuil_allumage: seuil(allumage),
            seuil_extinction: seuil(extinction),
        });
        let en_feu_initial = |i: u64| hacher(i, 0xFE0) < seuil(PART_EN_FEU);
        match repr {
            Representation::Marqueur => {
                // Créées directement dans leur archétype final.
                world.spawn_batch((0..n).filter(|i| !en_feu_initial(*i)).map(|i| {
                    let (p, v, j) = etat_initial(i);
                    (Id(i), Position(p), Vitesse(v), Jauges(j))
                }));
                world.spawn_batch((0..n).filter(|i| en_feu_initial(*i)).map(|i| {
                    let (p, v, j) = etat_initial(i);
                    (Id(i), Position(p), Vitesse(v), Jauges(j), EnFeu)
                }));
            }
            Representation::Champ => {
                world.spawn_batch((0..n).map(|i| {
                    let (p, v, j) = etat_initial(i);
                    let etat = Etat {
                        en_feu: en_feu_initial(i),
                    };
                    (Id(i), Position(p), Vitesse(v), Jauges(j), etat)
                }));
            }
        }

        ComputeTaskPool::get_or_init(|| TaskPoolBuilder::new().num_threads(threads).build());
        let mut schedule = Schedule::default();
        schedule.set_executor(MultiThreadedExecutor::new());
        match repr {
            Representation::Marqueur => {
                schedule.add_systems((bruler_marqueur, basculer_marqueur, avancer).chain())
            }
            Representation::Champ => {
                schedule.add_systems((bruler_champ, basculer_champ, avancer).chain())
            }
        };
        schedule
            .initialize(&mut world)
            .expect("initialisation du schedule");
        BancEtats {
            world,
            schedule,
            repr,
        }
    }
}

impl Banc for BancEtats {
    fn tick(&mut self) {
        // Les commandes (insertion, retrait de `EnFeu`) sont appliquées à la
        // fin de `run` : leur coût est compris dans la mesure.
        self.schedule.run(&mut self.world);
    }

    fn somme_controle(&mut self) -> u64 {
        let mut h = 0u64;
        let mut en_feu = 0u64;
        match self.repr {
            Representation::Marqueur => {
                let mut q = self.world.query::<(&Position, &Jauges, Has<EnFeu>)>();
                for (p, j, f) in q.iter(&self.world) {
                    h = h.wrapping_add(empreinte(p.0, j.0));
                    en_feu += f as u64;
                }
            }
            Representation::Champ => {
                let mut q = self.world.query::<(&Position, &Jauges, &Etat)>();
                for (p, j, e) in q.iter(&self.world) {
                    h = h.wrapping_add(empreinte(p.0, j.0));
                    en_feu += e.en_feu as u64;
                }
            }
        }
        h.wrapping_add(en_feu.wrapping_mul(0x9E37_79B9_7F4A_7C15))
    }
}
