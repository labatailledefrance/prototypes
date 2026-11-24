//! Implémentation avec `bevy_ecs` seul (sans fenêtre ni rendu).

use bevy_ecs::batching::BatchingStrategy;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{MultiThreadedExecutor, SingleThreadedExecutor};
use bevy_tasks::{ComputeTaskPool, TaskPoolBuilder};

use crate::logique::{deplacer, empreinte, etat_initial, maj_jauges};
use crate::{Banc, Disposition, Mode};

#[derive(Component)]
struct Position([f64; 3]);
#[derive(Component)]
struct Vitesse([f32; 3]);

// Variante « séparées » : une jauge = un composant.
#[derive(Component)]
struct Fatigue(f32);
#[derive(Component)]
struct Moral(f32);
#[derive(Component)]
struct Faim(f32);
#[derive(Component)]
struct Soif(f32);

// Variante « groupées » : les quatre jauges dans un seul composant.
#[derive(Component)]
struct Jauges([f32; 4]);

// Marqueurs sans données, pour fragmenter le monde en archétypes. Dans un
// stockage en colonnes, une colonne qu'une requête ne lit pas ne coûte rien :
// seuls comptent le nombre de tables (archétypes) et leur taille. Ces
// marqueurs simulent donc des types d'entités variés (soldat, civil, zombie,
// véhicule, états…) sans fausser le travail des systèmes mesurés.
macro_rules! marqueurs {
    ($($m:ident),*) => { $( #[derive(Component, Default)] struct $m; )* };
}
marqueurs!(M0, M1, M2, M3, M4, M5, M6, M7, M8, M9);
/// 10 marqueurs : jusqu'à 1 024 combinaisons, donc 1 024 archétypes.
pub const ARCHETYPES_MAX: usize = 1 << 10;

/// Répartit les entités en `k` archétypes : l'entité de rang i reçoit les
/// marqueurs correspondant aux bits de `i % k`.
fn fragmenter(world: &mut World, entites: &[Entity], k: usize) {
    assert!(
        (1..=ARCHETYPES_MAX).contains(&k),
        "archétypes : 1 à {ARCHETYPES_MAX}"
    );
    macro_rules! poser {
        ($($bit:literal => $m:ident),*) => { $(
            world.insert_batch(
                entites
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| (i % k) >> $bit & 1 == 1)
                    .map(|(_, e)| (*e, $m)),
            );
        )* };
    }
    poser!(0 => M0, 1 => M1, 2 => M2, 3 => M3, 4 => M4, 5 => M5, 6 => M6, 7 => M7, 8 => M8, 9 => M9);
}

/// Stratégie de découpage des requêtes parallèles. Par défaut, Bevy fixe la
/// taille d'un lot à « plus grande table / nombre de threads » : avec beaucoup
/// de petites tables, chacune est découpée en autant de lots minuscules. Une
/// taille minimale (`--lot-min`) regroupe les petites tables dans un même lot.
#[derive(Resource, Clone)]
struct Lots(BatchingStrategy);

type QJaugesSep<'a> = (
    &'a Vitesse,
    &'a mut Fatigue,
    &'a mut Moral,
    &'a mut Faim,
    &'a mut Soif,
);

/// Accès en écriture à un composant. Avec `DETECTION`, chaque écriture met à
/// jour l'horodatage de modification du composant (comportement normal de
/// Bevy) ; sans, on le contourne pour mesurer ce que coûte ce suivi.
#[inline(always)]
fn ecrire<'a, T, const DETECTION: bool>(m: &'a mut Mut<T>) -> &'a mut T {
    if DETECTION {
        m.as_mut()
    } else {
        m.bypass_change_detection()
    }
}

fn deplacement_seq<const D: bool>(mut q: Query<(&mut Position, &Vitesse)>) {
    for (mut p, v) in &mut q {
        ecrire::<_, D>(&mut p).0 = deplacer(p.0, v.0);
    }
}

fn deplacement_par<const D: bool>(mut q: Query<(&mut Position, &Vitesse)>, l: Res<Lots>) {
    q.par_iter_mut()
        .batching_strategy(l.0.clone())
        .for_each(|(mut p, v)| {
            ecrire::<_, D>(&mut p).0 = deplacer(p.0, v.0);
        });
}

#[inline(always)]
fn jauges_sep<const D: bool>(
    (v, mut f, mut m, mut fa, mut s): (&Vitesse, Mut<Fatigue>, Mut<Moral>, Mut<Faim>, Mut<Soif>),
) {
    let [a, b, c, d] = maj_jauges(v.0, [f.0, m.0, fa.0, s.0]);
    ecrire::<_, D>(&mut f).0 = a;
    ecrire::<_, D>(&mut m).0 = b;
    ecrire::<_, D>(&mut fa).0 = c;
    ecrire::<_, D>(&mut s).0 = d;
}

fn jauges_sep_seq<const D: bool>(mut q: Query<QJaugesSep>) {
    for e in &mut q {
        jauges_sep::<D>(e);
    }
}

fn jauges_sep_par<const D: bool>(mut q: Query<QJaugesSep>, l: Res<Lots>) {
    q.par_iter_mut()
        .batching_strategy(l.0.clone())
        .for_each(jauges_sep::<D>);
}

fn jauges_grp_seq<const D: bool>(mut q: Query<(&Vitesse, &mut Jauges)>) {
    for (v, mut j) in &mut q {
        ecrire::<_, D>(&mut j).0 = maj_jauges(v.0, j.0);
    }
}

fn jauges_grp_par<const D: bool>(mut q: Query<(&Vitesse, &mut Jauges)>, l: Res<Lots>) {
    q.par_iter_mut()
        .batching_strategy(l.0.clone())
        .for_each(|(v, mut j)| {
            ecrire::<_, D>(&mut j).0 = maj_jauges(v.0, j.0);
        });
}

/// Ajoute les systèmes de la variante choisie au schedule.
fn ajouter_systemes<const D: bool>(s: &mut Schedule, d: Disposition, m: Mode) {
    match (m, d) {
        (Mode::Mono, Disposition::Separees) => {
            s.add_systems((deplacement_seq::<D>, jauges_sep_seq::<D>))
        }
        (Mode::Mono, Disposition::Groupees) => {
            s.add_systems((deplacement_seq::<D>, jauges_grp_seq::<D>))
        }
        (Mode::Par, Disposition::Separees) => {
            s.add_systems((deplacement_par::<D>, jauges_sep_par::<D>))
        }
        (Mode::Par, Disposition::Groupees) => {
            s.add_systems((deplacement_par::<D>, jauges_grp_par::<D>))
        }
    };
}

pub struct BancBevy {
    world: World,
    schedule: Schedule,
    disposition: Disposition,
}

impl BancBevy {
    pub fn new(
        n: u64,
        disposition: Disposition,
        mode: Mode,
        threads: usize,
        detection: bool,
        archetypes: usize,
        lot_min: usize,
    ) -> Self {
        let mut world = World::new();
        world.insert_resource(Lots(BatchingStrategy::new().min_batch_size(lot_min.max(1))));
        let entites: Vec<Entity> = match disposition {
            Disposition::Separees => world
                .spawn_batch((0..n).map(|i| {
                    let (p, v, j) = etat_initial(i);
                    (
                        Position(p),
                        Vitesse(v),
                        Fatigue(j[0]),
                        Moral(j[1]),
                        Faim(j[2]),
                        Soif(j[3]),
                    )
                }))
                .collect(),
            Disposition::Groupees => world
                .spawn_batch((0..n).map(|i| {
                    let (p, v, j) = etat_initial(i);
                    (Position(p), Vitesse(v), Jauges(j))
                }))
                .collect(),
        };
        if archetypes > 1 {
            fragmenter(&mut world, &entites, archetypes);
        }
        drop(entites);

        let mut schedule = Schedule::default();
        match mode {
            Mode::Mono => {
                schedule.set_executor(SingleThreadedExecutor::new());
            }
            Mode::Par => {
                // Le pool global de Bevy, dimensionné une fois par processus.
                ComputeTaskPool::get_or_init(|| {
                    TaskPoolBuilder::new().num_threads(threads).build()
                });
                // Les deux systèmes ne s'écrivent pas mutuellement : l'exécuteur
                // peut les lancer en même temps, et chacun se découpe en lots.
                schedule.set_executor(MultiThreadedExecutor::new());
            }
        }
        if detection {
            ajouter_systemes::<true>(&mut schedule, disposition, mode);
        } else {
            ajouter_systemes::<false>(&mut schedule, disposition, mode);
        }
        // Construit le graphe des systèmes hors mesure.
        schedule
            .initialize(&mut world)
            .expect("initialisation du schedule");
        BancBevy {
            world,
            schedule,
            disposition,
        }
    }
}

impl Banc for BancBevy {
    fn tick(&mut self) {
        self.schedule.run(&mut self.world);
    }

    fn somme_controle(&mut self) -> u64 {
        let mut h = 0u64;
        match self.disposition {
            Disposition::Separees => {
                let mut q = self
                    .world
                    .query::<(&Position, &Fatigue, &Moral, &Faim, &Soif)>();
                for (p, f, m, fa, s) in q.iter(&self.world) {
                    h = h.wrapping_add(empreinte(p.0, [f.0, m.0, fa.0, s.0]));
                }
            }
            Disposition::Groupees => {
                let mut q = self.world.query::<(&Position, &Jauges)>();
                for (p, j) in q.iter(&self.world) {
                    h = h.wrapping_add(empreinte(p.0, j.0));
                }
            }
        }
        h
    }
}
