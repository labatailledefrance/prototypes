//! Compteurs de travail par thread, sans contention.
//!
//! Chaque thread écrit dans ses propres compteurs (une écriture simple, pas
//! d'instruction atomique verrouillée) ; la lecture fait la somme de tous les
//! threads entre deux ticks. Analogie : des compteurs par CPU du noyau
//! (`percpu`), agrégés seulement à la lecture.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

pub const REQUETES: usize = 0;
pub const REJETS: usize = 1;
pub const CASES: usize = 2;
pub const CANDIDATS: usize = 3;
pub const LIGNES: usize = 4;
const N: usize = 5;

#[derive(Default)]
struct Local([AtomicU64; N]);

static REGISTRE: Mutex<Vec<&'static Local>> = Mutex::new(Vec::new());

thread_local! {
    static LOCAL: &'static Local = {
        let l: &'static Local = Box::leak(Box::default());
        REGISTRE.lock().unwrap().push(l);
        l
    };
}

/// Ajoute des valeurs aux compteurs du thread courant.
#[inline(always)]
pub fn ajouter(v: [u64; N]) {
    LOCAL.with(|l| {
        for (a, x) in l.0.iter().zip(v) {
            // Seul ce thread écrit : lecture puis écriture, sans verrou.
            a.store(a.load(Relaxed) + x, Relaxed);
        }
    });
}

/// Somme de tous les threads (à lire entre deux ticks).
pub fn total() -> [u64; N] {
    let mut t = [0; N];
    for l in REGISTRE.lock().unwrap().iter() {
        for (s, a) in t.iter_mut().zip(&l.0) {
            *s += a.load(Relaxed);
        }
    }
    t
}
