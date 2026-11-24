//! Lecture des couches de la BD TOPO utiles au jeu, en structures compactes.
//!
//! Les chaînes de caractères des énumérations deviennent des octets selon des
//! listes fixes (ordre stable) ; une valeur inconnue vaut 255 et est comptée.
//! Toutes les couches sont lues en parallèle (une connexion par couche).

use crate::gpkg::{self, Geom, Ligne, Objet, Polygone, Val};
use std::path::Path;

pub const INCONNU: u8 = 255;

pub fn code(listes: &[&str], v: &str) -> u8 {
    listes.iter().position(|x| *x == v).map(|p| p as u8).unwrap_or(if v.is_empty() { 254 } else { INCONNU })
}

pub const USAGES: &[&str] = &[
    "Résidentiel", "Indifférencié", "Annexe", "Commercial et services", "Industriel", "Sportif", "Religieux", "Agricole",
];
pub const NATURES_BAT: &[&str] = &[
    "Indifférenciée", "Industriel, agricole ou commercial", "Eglise", "Chapelle", "Monument", "Serre", "Silo",
    "Château", "Tour, donjon", "Tribune", "Moulin à vent", "Fort, blockhaus, casemate", "Arc de triomphe",
    "Arène ou théâtre antique", "Phare",
];
pub const NATURES_ROUTE: &[&str] = &[
    "Type autoroutier", "Route à 2 chaussées", "Route à 1 chaussée", "Bretelle", "Rond-point", "Route empierrée",
    "Chemin", "Sentier", "Escalier", "Bac ou liaison maritime", "Piste cyclable", "Bac ou liaison fluviale",
];
pub const SENS: &[&str] = &["Double sens", "Sens direct", "Sens inverse", "Sans objet"];
pub const ACCES_VL: &[&str] = &["Libre", "A péage", "Restreint aux ayants droit", "Physiquement impossible", "Saisonnier"];
/// Classes de végétation (1 à 10 dans la couche d'occupation) ; 5 = haie.
pub const VEGETATION: &[&str] = &[
    "", "Forêt fermée de feuillus", "Forêt fermée de conifères", "Forêt fermée mixte", "Bois", "Haie",
    "Lande ligneuse", "Verger", "Vigne", "Peupleraie", "Forêt ouverte",
];
pub const CLASSES_LARGEUR: &[&str] = &["Entre 0 et 15 m", "Entre 15 et 50 m", "Plus de 50 m", "Sans objet"];

pub struct Batiment {
    pub id: u64,
    pub polys: Vec<Polygone>,
    pub nature: u8,
    pub usage1: u8,
    pub usage2: u8,
    pub leger: bool,
    pub hauteur_dm: Option<u32>,
    pub etages: Option<u32>,
    pub logements: Option<u32>,
    pub cx: f64,
    pub cy: f64,
    pub emprise: f64,
}

impl Batiment {
    /// Plancher (m²) : niveaux = étages, sinon hauteur / 3 m (P-05).
    pub fn plancher(&self) -> f64 {
        let niv = match (self.etages, self.hauteur_dm) {
            (Some(e), _) if e > 0 => e as f64,
            (_, Some(h)) => (h as f64 / 30.0).round().max(1.0),
            _ => 1.0,
        };
        self.emprise * niv
    }
    pub fn residentiel(&self) -> bool {
        self.usage1 == 0 || self.usage2 == 0 || self.logements.unwrap_or(0) > 0
    }
}

pub struct Route {
    pub id: u64,
    pub ligne: Ligne,
    pub nature: u8,
    pub importance: u8,
    pub pos_sol: i8,
    pub largeur_dm: u16,
    pub voies: u8,
    pub sens: u8,
    pub acces_vl: u8,
    pub urbain: bool,
    pub vitesse: u8,
    pub poids_dt: u16,   // restriction de poids total, en dixièmes de tonne (0 = aucune)
    pub hauteur_dm: u16, // restriction de hauteur
    pub largeur_r_dm: u16,
}

pub struct VoieFerree {
    pub id: u64,
    pub ligne: Ligne,
    pub pos_sol: i8,
    pub electrifiee: bool,
    pub voies: u8,
    pub nature: u8,
}

pub struct Construction {
    pub id: u64,
    pub geom: Geom,
    pub nature: String,
    pub toponyme: String,
}

pub struct Surface {
    pub polys: Vec<Polygone>,
    pub classe: u8,
}

pub struct LigneLarge {
    pub ligne: Ligne,
    pub demi: f64,
    pub classe: u8,
}

pub struct Commune {
    pub code: String,
    pub nom: String,
    pub dept: String,
    pub population: u32,
    pub polys: Vec<Polygone>,
}

pub struct Lieu {
    pub id: u64,
    pub type_: u8,
    pub nom: String,
    pub detail: String,
    pub geom: Geom,
    pub insee: String,
}

pub const TYPES_LIEUX: &[&str] = &[
    "Mairie", "Centrale électrique (CNPE)", "Usine de retraitement", "Port", "Gare", "Gare maritime", "Aérodrome",
    "Piste", "Barrage", "Enceinte militaire", "Centrale éolienne", "Écluse",
];

pub struct Donnees {
    pub batiments: Vec<Batiment>,
    pub bat_ecartes: usize,
    pub routes: Vec<Route>,
    pub voies: Vec<VoieFerree>,
    pub constructions: Vec<Construction>,
    pub vegetation: Vec<Surface>,
    pub haies: Vec<LigneLarge>,
    pub eaux_surf: Vec<Surface>,
    pub eaux_lin: Vec<LigneLarge>,
    pub estran: Vec<Surface>,
    pub communes: Vec<Commune>,
    pub lieux: Vec<Lieu>,
    pub inconnus: Vec<(String, usize)>,
    pub ms: Vec<(String, f64, usize)>,
}

fn polys(g: Geom) -> Vec<Polygone> {
    match g {
        Geom::Polygones(p) => p,
        _ => Vec::new(),
    }
}

fn lignes(g: Geom) -> Vec<Ligne> {
    match g {
        Geom::Lignes(l) => l,
        _ => Vec::new(),
    }
}

/// Surface (trous déduits) et centroïde, formule du lacet (P-05).
pub fn surface_centroide(ps: &[Polygone]) -> (f64, f64, f64) {
    let (mut at, mut cx, mut cy) = (0.0, 0.0, 0.0);
    for p in ps {
        for anneau in p {
            if anneau.len() < 3 {
                continue;
            }
            let (x0, y0) = (anneau[0][0], anneau[0][1]);
            let (mut a, mut sx, mut sy) = (0.0, 0.0, 0.0);
            let k = anneau.len();
            for i in 0..k {
                let (p1, p2) = (anneau[i], anneau[(i + 1) % k]);
                let (ax, ay, bx, by) = (p1[0] - x0, p1[1] - y0, p2[0] - x0, p2[1] - y0);
                let c = ax * by - bx * ay;
                a += c;
                sx += (ax + bx) * c;
                sy += (ay + by) * c;
            }
            // L'anneau extérieur et les trous ont des orientations opposées
            // en GeoPackage ; on somme les aires signées.
            at += a / 2.0;
            cx += sx / 6.0 + x0 * a / 2.0;
            cy += sy / 6.0 + y0 * a / 2.0;
        }
    }
    if at.abs() < 1e-9 {
        let b = Geom::Polygones(ps.to_vec()).bbox();
        return (0.0, (b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    }
    (at.abs(), cx / at, cy / at)
}

pub fn lire(chemin: &Path) -> Donnees {
    use std::time::Instant;
    let chrono = |nom: &str, t: Instant, n: usize| (nom.to_string(), t.elapsed().as_secs_f64() * 1e3, n);
    let en_service = "etat_de_l_objet = 'En service' or etat_de_l_objet is null";

    let ((bat, routes), ((voies, constr), ((veg, haies), ((eaux, communes), lieux)))) = rayon::join(
        || {
            rayon::join(
                || {
                    let t = Instant::now();
                    let o = gpkg::lire(
                        chemin,
                        "batiment",
                        &["cleabs", "nature", "usage_1", "usage_2", "construction_legere", "hauteur", "nombre_d_etages", "nombre_de_logements", "etat_de_l_objet"],
                        "",
                    );
                    (o, t)
                },
                || {
                    let t = Instant::now();
                    let o = gpkg::lire(
                        chemin,
                        "troncon_de_route",
                        &["cleabs", "nature", "importance", "position_par_rapport_au_sol", "largeur_de_chaussee", "nombre_de_voies", "sens_de_circulation", "acces_vehicule_leger", "urbain", "vitesse_moyenne_vl", "restriction_de_poids_total", "restriction_de_hauteur", "restriction_de_largeur", "fictif"],
                        en_service,
                    );
                    (o, t)
                },
            )
        },
        || {
            rayon::join(
                || {
                    rayon::join(
                        || {
                            let t = Instant::now();
                            (gpkg::lire(chemin, "troncon_de_voie_ferree", &["cleabs", "position_par_rapport_au_sol", "electrifie", "nombre_de_voies", "nature"], en_service), t)
                        },
                        || {
                            let t = Instant::now();
                            let mut a = gpkg::lire(chemin, "construction_lineaire", &["cleabs", "nature", "toponyme"], en_service);
                            a.extend(gpkg::lire(chemin, "construction_surfacique", &["cleabs", "nature", "toponyme"], en_service));
                            (a, t)
                        },
                    )
                },
                || {
                    rayon::join(
                        || {
                            rayon::join(
                                || {
                                    let t = Instant::now();
                                    (gpkg::lire(chemin, "zone_de_vegetation", &["nature"], ""), t)
                                },
                                || {
                                    let t = Instant::now();
                                    (gpkg::lire(chemin, "haie", &["largeur"], ""), t)
                                },
                            )
                        },
                        || {
                            rayon::join(
                                || {
                                    rayon::join(
                                        || {
                                            let t = Instant::now();
                                            let s = gpkg::lire(chemin, "surface_hydrographique", &["nature", "position_par_rapport_au_sol"], "");
                                            let l = gpkg::lire(chemin, "troncon_hydrographique", &["classe_de_largeur", "position_par_rapport_au_sol", "fictif"], "");
                                            let e = gpkg::lire(chemin, "zone_d_estran", &["nature"], "");
                                            ((s, l, e), t)
                                        },
                                        || {
                                            let t = Instant::now();
                                            let c = gpkg::lire(chemin, "commune", &["code_insee", "nom_officiel", "code_insee_du_departement", "population"], "");
                                            let a = gpkg::lire(chemin, "arrondissement_municipal", &["code_insee", "nom_officiel", "code_insee_de_la_commune_de_rattach", "population"], "");
                                            ((c, a), t)
                                        },
                                    )
                                },
                                || {
                                    let t = Instant::now();
                                    let z = gpkg::lire(chemin, "zone_d_activite_ou_d_interet", &["cleabs", "nature", "nature_detaillee", "toponyme", "insee_commune", "categorie"], "");
                                    let e = gpkg::lire(chemin, "equipement_de_transport", &["cleabs", "nature", "nature_detaillee", "toponyme", "insee_commune"], "");
                                    let a = gpkg::lire(chemin, "aerodrome", &["cleabs", "nature", "toponyme", "code_icao"], "");
                                    let p = gpkg::lire(chemin, "piste_d_aerodrome", &["cleabs", "nature", "fonction"], "");
                                    ((z, e, a, p), t)
                                },
                            )
                        },
                    )
                },
            )
        },
    );

    let mut ms = Vec::new();
    let mut inconnus: std::collections::BTreeMap<String, usize> = Default::default();
    let mut note = |couche: &str, v: &Val, c: u8| {
        if c == INCONNU {
            *inconnus.entry(format!("{couche}:{}", v.s())).or_default() += 1;
        }
    };

    // Bâtiments « En service » (comme P-05).
    let (bat_o, t) = bat;
    let nb_bat_lus = bat_o.len();
    let mut batiments = Vec::with_capacity(bat_o.len());
    let mut bat_ecartes = 0;
    for Objet { geom, att, .. } in bat_o {
        if !(att[8].s() == "En service" || matches!(att[8], Val::Nul)) {
            bat_ecartes += 1;
            continue;
        }
        let ps = polys(geom);
        if ps.is_empty() {
            bat_ecartes += 1;
            continue;
        }
        let (emprise, cx, cy) = surface_centroide(&ps);
        let nature = code(NATURES_BAT, att[1].s());
        note("batiment.nature", &att[1], nature);
        let usage1 = code(USAGES, att[2].s());
        note("batiment.usage_1", &att[2], usage1);
        batiments.push(Batiment {
            id: gpkg::cle(&att[0]),
            polys: ps,
            nature,
            usage1,
            usage2: code(USAGES, att[3].s()),
            leger: att[4].b(),
            hauteur_dm: att[5].f().filter(|h| *h > 0.0).map(|h| (h * 10.0).round() as u32),
            etages: att[6].i().map(|v| v.max(0) as u32),
            logements: att[7].i().map(|v| v.max(0) as u32),
            cx,
            cy,
            emprise,
        });
    }
    ms.push(chrono("batiment", t, nb_bat_lus));

    let (rt_o, t) = routes;
    let n = rt_o.len();
    let mut routes = Vec::with_capacity(n);
    for Objet { geom, att, .. } in rt_o {
        let nature = code(NATURES_ROUTE, att[1].s());
        note("route.nature", &att[1], nature);
        for l in lignes(geom) {
            routes.push(Route {
                id: gpkg::cle(&att[0]),
                ligne: l,
                nature,
                importance: att[2].i().unwrap_or(7).clamp(0, 9) as u8,
                pos_sol: att[3].i().unwrap_or(0).clamp(-9, 9) as i8,
                largeur_dm: att[4].f().map(|v| (v * 10.0).round() as u16).unwrap_or(0),
                voies: att[5].i().unwrap_or(0).clamp(0, 255) as u8,
                sens: code(SENS, att[6].s()),
                acces_vl: code(ACCES_VL, att[7].s()),
                urbain: att[8].b(),
                vitesse: att[9].i().unwrap_or(0).clamp(0, 255) as u8,
                poids_dt: att[10].f().map(|v| (v * 10.0).round() as u16).unwrap_or(0),
                hauteur_dm: att[11].f().map(|v| (v * 10.0).round() as u16).unwrap_or(0),
                largeur_r_dm: att[12].f().map(|v| (v * 10.0).round() as u16).unwrap_or(0),
            });
        }
    }
    ms.push(chrono("troncon_de_route", t, n));

    let (vf_o, t) = voies;
    let n = vf_o.len();
    let mut voies = Vec::new();
    for Objet { geom, att, .. } in vf_o {
        for l in lignes(geom) {
            voies.push(VoieFerree {
                id: gpkg::cle(&att[0]),
                ligne: l,
                pos_sol: att[1].i().unwrap_or(0).clamp(-9, 9) as i8,
                electrifiee: att[2].b(),
                voies: att[3].i().unwrap_or(1).clamp(0, 255) as u8,
                nature: code(&["Voie ferrée principale", "Voie de service", "LGV", "Tramway", "Métro", "Funiculaire ou crémaillère", "Sans objet"], att[4].s()),
            });
        }
    }
    ms.push(chrono("troncon_de_voie_ferree", t, n));

    let (co_o, t) = constr;
    let n = co_o.len();
    let constructions: Vec<Construction> = co_o
        .into_iter()
        .map(|Objet { geom, att, .. }| Construction { id: gpkg::cle(&att[0]), geom, nature: att[1].s().to_string(), toponyme: att[2].s().to_string() })
        .collect();
    ms.push(chrono("construction_lineaire+surfacique", t, n));

    let (vg_o, t1) = veg;
    let (hs_o, t2) = haies;
    let n = vg_o.len();
    let mut vegetation = Vec::with_capacity(n);
    for Objet { geom, att, .. } in vg_o {
        let c = code(VEGETATION, att[0].s());
        note("vegetation.nature", &att[0], c);
        vegetation.push(Surface { polys: polys(geom), classe: if c == INCONNU { 4 } else { c } });
    }
    ms.push(chrono("zone_de_vegetation", t1, n));
    let n = hs_o.len();
    let mut haies_v = Vec::with_capacity(n);
    for Objet { geom, att, .. } in hs_o {
        // Largeur renseignée en m (à vérifier) ; 3 m par défaut (hypothèse).
        let l = att[0].f().filter(|v| *v > 0.0 && *v < 50.0).unwrap_or(3.0);
        for ligne in lignes(geom) {
            haies_v.push(LigneLarge { ligne, demi: l / 2.0, classe: 5 });
        }
    }
    ms.push(chrono("haie", t2, n));

    let ((sh_o, th_o, es_o), t) = eaux;
    let n = sh_o.len() + th_o.len() + es_o.len();
    let mut eaux_surf = Vec::new();
    for Objet { geom, att, .. } in sh_o {
        if att[1].i().unwrap_or(0) < 0 {
            continue; // souterrain
        }
        let salee = matches!(att[0].s(), "Estuaire" | "Lagune" | "Eau salée");
        eaux_surf.push(Surface { polys: polys(geom), classe: if salee { 2 } else { 1 } });
    }
    let mut eaux_lin = Vec::new();
    for Objet { geom, att, .. } in th_o {
        if att[1].i().unwrap_or(0) < 0 || att[2].b() {
            continue; // souterrain ou fictif
        }
        // Largeur : la classe ne donne qu'un intervalle ; les cours larges
        // ont aussi une surface. Hypothèse : 2 m pour « 0 à 15 m », 15 m
        // au-delà.
        let demi = match att[0].s() {
            "Entre 15 et 50 m" | "Plus de 50 m" => 7.5,
            _ => 1.0,
        };
        for ligne in lignes(geom) {
            eaux_lin.push(LigneLarge { ligne, demi, classe: 1 });
        }
    }
    let estran = es_o.into_iter().map(|o| Surface { polys: polys(o.geom), classe: 3 }).collect();
    ms.push(chrono("hydrographie+estran", t, n));

    let ((cm_o, ar_o), t) = communes;
    let n = cm_o.len() + ar_o.len();
    let mut communes: Vec<Commune> = Vec::new();
    // Paris, Lyon, Marseille : les arrondissements remplacent la commune.
    let avec_arr: std::collections::BTreeSet<String> = ar_o.iter().map(|o| o.att[2].s().to_string()).collect();
    for Objet { geom, att, .. } in cm_o {
        if avec_arr.contains(att[0].s()) {
            continue;
        }
        communes.push(Commune {
            code: att[0].s().into(),
            nom: att[1].s().into(),
            dept: att[2].s().into(),
            population: att[3].i().unwrap_or(0) as u32,
            polys: polys(geom),
        });
    }
    for Objet { geom, att, .. } in ar_o {
        let code = att[0].s().to_string();
        communes.push(Commune {
            dept: code[..2].to_string(),
            code,
            nom: att[1].s().into(),
            population: att[3].i().unwrap_or(0) as u32,
            polys: polys(geom),
        });
    }
    communes.sort_by(|a, b| a.code.cmp(&b.code));
    ms.push(chrono("commune+arrondissement", t, n));

    let ((zo, eq, ae, pi), t) = lieux;
    let n = zo.len() + eq.len() + ae.len() + pi.len();
    let mut lieux = Vec::new();
    for Objet { geom, att, .. } in zo {
        let (nat, det, top) = (att[1].s(), att[2].s(), att[3].s());
        let ty = match nat {
            "Mairie" if det.is_empty() || det == "Hôtel de ville" => Some(0),
            "Centrale électrique" if det == "Parc éolien" => Some(10),
            "Centrale électrique" => Some(1),
            "Usine" if top.contains("Retraitement") => Some(2),
            "Enceinte militaire" => Some(9),
            _ => None,
        };
        if let Some(ty) = ty {
            lieux.push(Lieu { id: gpkg::cle(&att[0]), type_: ty, nom: top.into(), detail: det.into(), geom, insee: att[4].s().into() });
        }
    }
    for Objet { geom, att, .. } in eq {
        let nat = att[1].s();
        let ty = match nat {
            "Port" => Some(3),
            "Gare voyageurs et fret" | "Gare voyageurs uniquement" | "Gare fret uniquement" => Some(4),
            "Gare maritime" => Some(5),
            _ => None,
        };
        if let Some(ty) = ty {
            lieux.push(Lieu { id: gpkg::cle(&att[0]), type_: ty, nom: att[3].s().into(), detail: format!("{nat} / {}", att[2].s()), geom, insee: att[4].s().into() });
        }
    }
    for Objet { geom, att, .. } in ae {
        if att[1].s() == "Aérodrome" || att[1].s() == "Altiport" {
            lieux.push(Lieu { id: gpkg::cle(&att[0]), type_: 6, nom: att[2].s().into(), detail: att[3].s().into(), geom, insee: String::new() });
        }
    }
    for Objet { geom, att, .. } in pi {
        lieux.push(Lieu { id: gpkg::cle(&att[0]), type_: 7, nom: String::new(), detail: format!("{} / {}", att[1].s(), att[2].s()), geom, insee: String::new() });
    }
    for c in &constructions {
        let ty = match c.nature.as_str() {
            "Barrage" => Some(8),
            "Ecluse" => Some(11),
            _ => None,
        };
        if let Some(ty) = ty {
            lieux.push(Lieu { id: c.id, type_: ty, nom: c.toponyme.clone(), detail: c.nature.clone(), geom: c.geom.clone(), insee: String::new() });
        }
    }
    lieux.sort_by_key(|l| (l.type_, l.id));
    ms.push(chrono("lieux", t, n));

    Donnees {
        batiments,
        bat_ecartes,
        routes,
        voies,
        constructions,
        vegetation,
        haies: haies_v,
        eaux_surf,
        eaux_lin,
        estran,
        communes,
        lieux,
        inconnus: inconnus.into_iter().collect(),
        ms,
    }
}

pub fn _inutilise(_: &[Ligne]) {}
