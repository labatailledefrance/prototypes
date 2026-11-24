//! Logique de tick commune aux deux implémentations.
//!
//! Bevy et la version « Rust brut » appellent exactement ces fonctions :
//! elles font donc le même calcul, et seules la disposition mémoire et
//! l'ordonnancement diffèrent.

/// Pas de temps d'un tick : 20 ticks par seconde.
pub const DT: f32 = 0.05;

const K_FAIM: f32 = 1.0 / (3600.0 * 24.0);
const K_SOIF: f32 = 1.0 / (3600.0 * 12.0);
const K_EFFORT: f32 = 1.0 / (3600.0 * 8.0);
const K_REPOS: f32 = 1.0 / (3600.0 * 4.0);
const K_MORAL: f32 = 1.0 / 3600.0;

/// Déplacement : position += vitesse × dt (positions en 64 bits, cf. C-01).
#[inline(always)]
pub fn deplacer(p: [f64; 3], v: [f32; 3]) -> [f64; 3] {
    let dt = DT as f64;
    [
        p[0] + v[0] as f64 * dt,
        p[1] + v[1] as f64 * dt,
        p[2] + v[2] as f64 * dt,
    ]
}

/// Jauges dans l'ordre : fatigue, moral, faim, soif. Toutes bornées à 0..1.
#[inline(always)]
pub fn maj_jauges(v: [f32; 3], j: [f32; 4]) -> [f32; 4] {
    let [fatigue, moral, faim, soif] = j;
    let v2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    let faim = (faim + K_FAIM * DT).min(1.0);
    let soif = (soif + K_SOIF * DT).min(1.0);
    let fatigue = if v2 > 0.0 {
        fatigue + K_EFFORT * v2.sqrt() * DT
    } else {
        fatigue - K_REPOS * DT
    }
    .clamp(0.0, 1.0);
    let detresse = (faim + soif + fatigue) / 3.0;
    let moral = (moral + (0.5 - detresse) * K_MORAL * DT).clamp(0.0, 1.0);
    [fatigue, moral, faim, soif]
}

/// État initial déterministe de l'entité `i` (splitmix64, sans dépendance).
pub fn etat_initial(i: u64) -> ([f64; 3], [f32; 3], [f32; 4]) {
    let mut s = i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    let mut alea = || {
        s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64 // 0..1
    };
    // Positions sur un carré de 1 000 km (ordre de grandeur de la métropole).
    let p = [alea() * 1.0e6, alea() * 1.0e6, alea() * 100.0];
    // Un quart des entités immobiles, pour que la branche repos existe.
    let immobile = alea() < 0.25;
    let mut v = [0.0f32; 3];
    if !immobile {
        v = [
            (alea() * 3.0 - 1.5) as f32,
            (alea() * 3.0 - 1.5) as f32,
            0.0,
        ];
    }
    let j = [alea() as f32, alea() as f32, alea() as f32, alea() as f32];
    (p, v, j)
}

/// Contribution d'une entité à la somme de contrôle. Addition modulaire des
/// bits : indépendante de l'ordre de parcours, donc comparable entre
/// implémentations et entre exécutions parallèles.
#[inline(always)]
pub fn empreinte(p: [f64; 3], j: [f32; 4]) -> u64 {
    let mut h = p[0].to_bits() ^ p[1].to_bits().rotate_left(21) ^ p[2].to_bits().rotate_left(42);
    for (k, x) in j.iter().enumerate() {
        h ^= (x.to_bits() as u64) << (k * 16 % 64);
    }
    h
}
