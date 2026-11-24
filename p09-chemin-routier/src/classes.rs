//! Classes de véhicules, charge des ponts (B-03) et métriques.
//!
//! Toutes les valeurs sont des **hypothèses de mesure** (masses et gabarits
//! de l'arsenal réel à vérifier), pas des statistiques de jeu.

use crate::graphe::{Graphe, Ouvrage};
use crate::tuiles::{Attr, Pont};

pub const INF: u32 = cch::INF_WEIGHT;

#[derive(Clone, Debug)]
pub struct Classe {
    pub nom: &'static str,
    pub masse_t: u16,
    pub largeur_dm: u16,
    pub hauteur_dm: u16,
    /// Vitesse maximale par nature (km/h), 0 = interdit.
    pub vmax: [u16; 7],
    /// Franchit les tronçons « physiquement impossibles » pour un VL
    /// (chenillé, sur chemins et routes empierrées seulement).
    pub chenille: bool,
}

/// Natures 0..=6 : autoroutier, 2 chaussées, 1 chaussée, bretelle,
/// rond-point, empierrée, chemin.
pub fn classes() -> Vec<Classe> {
    vec![
        // Voiture particulière ou VL militaire léger.
        Classe { nom: "voiture", masse_t: 2, largeur_dm: 18, hauteur_dm: 16, vmax: [130, 110, 90, 70, 50, 40, 0], chenille: false },
        // Porteur logistique chargé (PPLD, PPT : masse à vérifier).
        Classe { nom: "camion", masse_t: 32, largeur_dm: 26, hauteur_dm: 36, vmax: [90, 90, 80, 60, 40, 30, 20], chenille: false },
        // Tracteur porte-char chargé d'un Leclerc (ensemble à vérifier).
        Classe { nom: "porte-char", masse_t: 110, largeur_dm: 37, hauteur_dm: 42, vmax: [70, 70, 60, 40, 20, 0, 0], chenille: false },
        // Leclerc sur route (environ 60 t, fiche du char).
        Classe { nom: "leclerc", masse_t: 60, largeur_dm: 37, hauteur_dm: 25, vmax: [50, 50, 50, 40, 30, 30, 25], chenille: true },
    ]
}

/// Vitesse par défaut quand la BD TOPO n'en donne pas (km/h).
fn vitesse_defaut(a: &Attr) -> u16 {
    match a.nature {
        0 => 110,
        1 => 90,
        2 => {
            if a.urbain {
                40
            } else {
                60
            }
        }
        3 => 50,
        4 => 30,
        5 => 25,
        _ => 15,
    }
}

/// Règle du sens de circulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sens {
    /// Le code de la route : tous les sens uniques.
    Code,
    /// Seulement les chaussées physiquement séparées (autoroutes, routes à
    /// 2 chaussées, bretelles) ; sens uniques urbains et ronds-points ignorés.
    Physique,
}

/// Charge limite d'un ouvrage, en tonnes (heuristique B-03).
///
/// Paliers inspirés des convois du règlement français des ponts routiers
/// (convois militaires Mc 80 et Mc 120 sur le réseau principal, à
/// vérifier) : 120, 80, 44, 26, 12, 3,5 t.
pub const PALIERS: [u16; 6] = [120, 80, 44, 26, 12, 4];

pub fn palier(importance: u8) -> usize {
    match importance {
        1 | 2 => 0,
        3 => 1,
        4 => 2,
        5 => 3,
        6 => 4,
        _ => 4,
    }
}

/// Valeurs écrites à la main (identifiant BD TOPO du pont → tonnes). Pour
/// le prototype : quelques grands ouvrages, valeurs à vérifier.
pub fn valeurs_main() -> Vec<(u64, u16, &'static str)> {
    // Rempli après inventaire des grands ponts (voir README, mesure 3).
    crate::valeurs_ponts::VALEURS.to_vec()
}

pub fn charge(p: Option<&Pont>, a: &Attr, long_mm: u32, main: &[(u64, u16, &'static str)]) -> u16 {
    if let Some(p) = p {
        if let Some(v) = main.iter().find(|v| v.0 == p.id) {
            return v.1;
        }
    }
    // Restriction réelle de la BD TOPO (pont ou tronçon).
    let reel = p.map(|p| p.poids_dt).filter(|&v| v > 0).or(Some(a.poids_dt).filter(|&v| v > 0));
    if let Some(dt) = reel {
        return dt.div_ceil(10);
    }
    let imp = p.map(|p| if p.importance < 9 { p.importance.min(a.importance) } else { a.importance }).unwrap_or(a.importance);
    let mut k = palier(imp) as i32;
    let largeur = p.map(|p| p.largeur_dm).filter(|&l| l > 0).unwrap_or(a.largeur_dm);
    let longueur_dm = p.map(|p| p.longueur_dm).filter(|&l| l > 0).unwrap_or(long_mm / 100);
    let franchit = p.map(|p| p.franchit).unwrap_or(0);
    // La largeur de la BD TOPO est une classe (3 m pour une voie, 4 m pour
    // deux). Sous 2,5 m : passerelle ; une seule voie : un palier de moins.
    if largeur > 0 && largeur < 25 && !matches!(a.nature, 0 | 1 | 3) {
        return PALIERS[5];
    }
    let separee = matches!(a.nature, 0 | 1 | 3) || p.is_some_and(|p| matches!(p.nature_route, 0 | 1 | 3));
    if largeur > 0 && largeur < 35 && !separee {
        k += 1;
    }
    // Grand ouvrage sur l'eau d'une route secondaire (ponts suspendus et
    // anciens ouvrages) : un palier de moins.
    if longueur_dm >= 1000 && franchit & 1 != 0 && imp >= 4 {
        k += 1;
    }
    // Ponceau (moins de 10 m) : un palier de plus.
    if longueur_dm > 0 && longueur_dm < 100 {
        k -= 1;
    }
    // Passage supérieur sur une route ou une voie ferrée (et pas sur l'eau :
    // « route » est surestimé par P-07, les rampes touchent le pont) :
    // construit à une norme récente, au moins 44 t.
    if franchit & 6 != 0 && franchit & 1 == 0 {
        k = k.min(2);
    }
    PALIERS[k.clamp(0, 4) as usize]
}

/// Raisons de refus d'une arête pour une classe (pour les statistiques).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refus {
    Aucun,
    Nature,
    Acces,
    Largeur,
    Hauteur,
    Poids,
    Pont,
}

pub fn refus(c: &Classe, a: &Attr, o: Option<&Ouvrage>) -> Refus {
    if a.nature > 6 || c.vmax[a.nature as usize] == 0 {
        return Refus::Nature;
    }
    if a.acces == 3 && !(c.chenille && a.nature >= 5) {
        return Refus::Acces;
    }
    // Chaussée connue plus étroite que le véhicule (la BD TOPO donne des
    // classes : 3 m pour une voie, 4 m pour deux) : refusée aux véhicules à
    // roues ; un chenillé déborde sur les accotements.
    // Chaussées séparées (autoroutes, 2 chaussées, bretelles) : largeur
    // donnée par chaussée, accotements et bande d'arrêt d'urgence en plus.
    let separee = matches!(a.nature, 0 | 1 | 3);
    if !c.chenille && !separee && a.largeur_dm > 0 && a.largeur_dm < c.largeur_dm {
        return Refus::Largeur;
    }
    if a.largeur_r_dm > 0 && a.largeur_r_dm < c.largeur_dm {
        return Refus::Largeur;
    }
    if a.hauteur_dm > 0 && a.hauteur_dm < c.hauteur_dm {
        return Refus::Hauteur;
    }
    if a.poids_dt > 0 && a.poids_dt < c.masse_t * 10 {
        return Refus::Poids;
    }
    if let Some(o) = o {
        if o.charge_t < c.masse_t {
            return Refus::Pont;
        }
        if !separee && o.largeur_dm > 0 && o.largeur_dm < c.largeur_dm {
            return Refus::Largeur;
        }
    }
    Refus::Aucun
}

/// Temps de parcours en ms, arrondi au-dessus (entiers seulement) :
/// ceil(3,6 × longueur_mm / v).
pub fn temps_ms(long_mm: u32, v_kmh: u16) -> u32 {
    let n = 36 * long_mm as u64;
    let d = 10 * v_kmh as u64;
    (n.div_ceil(d)).max(1) as u32
}

pub fn vitesse(c: &Classe, a: &Attr) -> u16 {
    let v = if a.vitesse > 0 { a.vitesse as u16 } else { vitesse_defaut(a) };
    v.min(c.vmax[a.nature as usize])
}

/// Poids des arcs : arc 2e = u → v, arc 2e + 1 = v → u.
pub fn metrique(g: &Graphe, c: &Classe, sens: Sens) -> Vec<u32> {
    let mut w = vec![INF; 2 * g.m()];
    for e in 0..g.m() {
        let a = &g.attr[e];
        let o = g.ouvrage[e].map(|o| &g.ouvrages[o as usize]);
        if refus(c, a, o) != Refus::Aucun {
            continue;
        }
        let t = temps_ms(g.long_mm[e], vitesse(c, a));
        let oriente = match sens {
            Sens::Code => true,
            Sens::Physique => matches!(a.nature, 0 | 1 | 3),
        };
        let (av, ar) = match (oriente, a.sens) {
            (true, 1) => (true, false),
            (true, 2) => (false, true),
            _ => (true, true),
        };
        if av {
            w[2 * e] = t;
        }
        if ar {
            w[2 * e + 1] = t;
        }
    }
    w
}

/// Vitesse maximale effective d'une métrique (pour l'heuristique de A*).
pub fn vmax_metrique(g: &Graphe, c: &Classe) -> u16 {
    (0..g.m())
        .filter(|&e| g.attr[e].nature <= 6)
        .map(|e| vitesse(c, &g.attr[e]))
        .max()
        .unwrap_or(1)
        .max(1)
}
