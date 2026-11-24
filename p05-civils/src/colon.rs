//! Un colon : son état compact et sa logique de vie, **communs aux trois
//! représentations** (A : entité ECS, B : dossier du bâtiment, C : seulement
//! pour les paramètres). A et B appellent la même fonction `vivre` sur le
//! même enregistrement de 32 octets : ils doivent donner exactement le même
//! état, ce que vérifie une somme de contrôle.
//!
//! Tous les paramètres sont des **hypothèses de prototype**, pas des
//! statistiques de jeu.

use crate::alea::{hacher, unite};

/// Ticks par seconde de jeu.
pub const TPS: u32 = 20;
pub const TICKS_JOUR: u32 = 86_400 * TPS;
/// Lieu absent (pas de travail).
pub const NUL: u32 = u32::MAX;

// Ressources consommées par les civils : noms exacts de RESOURCES.md.
pub const VIVRES: usize = 0;
pub const EAU: usize = 1;
pub const MEDICAMENTS: usize = 2;
pub const VETEMENTS: usize = 3;
pub const COUVERTURES: usize = 4;
pub const EAU_NP: usize = 5;
pub const NB_RES: usize = 6;
pub const NOMS_RES: [&str; NB_RES] =
    ["Vivres frais et en vrac", "Eau potable", "Médicaments", "Vêtements chauds", "Couvertures", "Eau non potable"];
/// Unité de stock des médicaments : une dose (un jour de traitement) vaut
/// `TICKS_JOUR` unités, consommées tick par tick pendant le traitement.
pub const DOSE: u64 = TICKS_JOUR as u64;

// Drapeaux.
pub const A_VETEMENTS: u8 = 1;
pub const A_COUVERTURE: u8 = 2;
/// Exposé à un zombie (assaut) : zombifié 10 s plus tard.
pub const EXPOSE: u8 = 4;
/// Armé (milicien) : tire par les fenêtres pendant un assaut.
pub const ARME: u8 = 8;

/// État compact d'un colon : **32 octets**. Identité, expérience,
/// compétences, jauges, maladie, logement et travail. Le lieu où il se
/// trouve est implicite (le dossier qui le contient, ou l'entité).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(C)]
pub struct Colon {
    pub id: u32,
    /// Tick de la dernière mise à jour (pour l'intervalle exact).
    pub maj: u32,
    pub logement: u32,
    pub travail: u32,
    /// Faim, soif, fatigue, moral, usure : 0 à 65 535 pour 0 à 1.
    pub j: [u16; 5],
    pub experience: u16,
    /// Compétences liées aux ressources (bits : électricien, soudeur,
    /// chimiste…).
    pub competences: u16,
    /// 0 sain, 1 malade.
    pub malade: u8,
    pub drapeaux: u8,
}

pub const FAIM: usize = 0;
pub const SOIF: usize = 1;
pub const FATIGUE: usize = 2;
pub const MORAL: usize = 3;
pub const USURE: usize = 4;

#[inline(always)]
pub fn f(j: u16) -> f32 {
    j as f32 * (1.0 / 65_535.0)
}

/// Quantification avec tramage déterministe : sans biais, même pour des
/// incréments plus petits que le pas de 1/65 535.
#[inline(always)]
pub fn q(x: f32, h: u64) -> u16 {
    (x.clamp(0.0, 1.0) * 65_535.0 + unite(h)).floor().min(65_535.0) as u16
}

/// Hypothèses de la vie des colons (prototype).
#[derive(Clone, Debug)]
pub struct ParamsCivils {
    /// Ticks entre deux mises à jour d'un bâtiment (20 : 1 Hz).
    pub periode: u32,
    pub graine: u64,
    /// Heure de jeu au tick 0 (h).
    pub heure_debut: f64,
    /// Température moyenne de l'air (°C) et amplitude jour/nuit.
    pub temperature: f32,
    pub amplitude: f32,
    /// Logements chauffés (bois ou électricité livrés).
    pub chauffage: bool,
    /// Faim : passage de 0 à 1 (h) ; seuil et gain d'un repas.
    pub faim_h: f32,
    pub repas: f32,
    pub soif_h: f32,
    pub boire: f32,
    /// Fatigue : au travail, éveillé au repos, récupération au sommeil (h).
    pub fatigue_travail_h: f32,
    pub fatigue_eveil_h: f32,
    pub recup_h: f32,
    /// Usure : récupération au repos en tile sûre (h) ; hausse par décès
    /// dans le bâtiment.
    pub usure_recup_h: f32,
    pub usure_deces: f32,
    /// Moral : constante de temps (h) ; baisse par décès dans le bâtiment.
    pub moral_h: f32,
    pub moral_deces: f32,
    /// Maladie : taux de base (par jour, à risque 1, sans froid ni faim).
    pub maladie_jour: f32,
    /// Guérison par jour avec médicaments, sans médicaments ; mort par jour
    /// sans médicaments.
    pub guerison_jour: f32,
    pub guerison_seul_jour: f32,
    pub mort_jour: f32,
    /// Poids du froid dans la chance de tomber malade.
    pub poids_froid: f32,
    /// Vitesse de marche et détour des trajets abstraits.
    pub marche: f32,
    pub detour: f32,
    /// Prise de poste : 6 h + tirage sur `etalement_poste_h` ; durée du poste.
    pub etalement_poste_h: f32,
    pub poste_h: f32,
}

impl Default for ParamsCivils {
    fn default() -> Self {
        ParamsCivils {
            periode: 20,
            graine: 0x05C1_7E25,
            heure_debut: 3.0,
            temperature: 12.0,
            amplitude: 4.0,
            chauffage: true,
            faim_h: 16.0,
            repas: 0.25,
            soif_h: 8.0,
            boire: 0.2,
            fatigue_travail_h: 10.0,
            fatigue_eveil_h: 20.0,
            recup_h: 4.0,
            usure_recup_h: 48.0,
            usure_deces: 0.05,
            moral_h: 1.0,
            moral_deces: 0.05,
            maladie_jour: 0.004,
            guerison_jour: 0.4,
            guerison_seul_jour: 0.08,
            mort_jour: 0.15,
            poids_froid: 6.0,
            marche: 1.4,
            detour: 1.3,
            etalement_poste_h: 3.0,
            poste_h: 8.0,
        }
    }
}

/// Ce qu'un lieu offre à ses occupants pendant une mise à jour.
#[derive(Clone, Copy, Debug, Default)]
pub struct CtxLieu {
    pub lieu: u32,
    pub colonie: u16,
    /// Risque sanitaire de la case (carte par tile, A-17).
    pub risque: f32,
    pub chauffe: bool,
    pub exterieur: bool,
    /// Décès dans le lieu depuis sa dernière mise à jour (événements).
    pub deces: u16,
}

/// Coefficients par seconde, précalculés une fois par tick (pas de
/// division dans la boucle).
#[derive(Clone, Copy, Debug)]
pub struct Coefs {
    pub faim: f32,
    pub soif: f32,
    pub fatigue_travail: f32,
    pub fatigue_eveil: f32,
    pub recup: f32,
    pub usure_recup: f32,
    pub moral: f32,
    pub maladie: f32,
    pub guerison: f32,
    pub guerison_seul: f32,
    pub mort: f32,
}

impl ParamsCivils {
    pub fn coefs(&self) -> Coefs {
        let h = |x: f32| 1.0 / (x * 3600.0);
        let j = |x: f32| x / 86_400.0;
        Coefs {
            faim: h(self.faim_h),
            soif: h(self.soif_h),
            fatigue_travail: h(self.fatigue_travail_h),
            fatigue_eveil: h(self.fatigue_eveil_h),
            recup: h(self.recup_h),
            usure_recup: h(self.usure_recup_h),
            moral: h(self.moral_h),
            maladie: j(self.maladie_jour),
            guerison: j(self.guerison_jour),
            guerison_seul: j(self.guerison_seul_jour),
            mort: j(self.mort_jour),
        }
    }
}

/// Ce que le monde offre : heure, météo, taux de service des stocks de
/// chaque colonie, positions des lieux (trajets).
pub struct CtxGlobal<'a> {
    pub p: &'a ParamsCivils,
    pub k: Coefs,
    pub maintenant: u32,
    /// Heure du jour (s).
    pub seconde_jour: f32,
    pub temp_air: f32,
    /// Par colonie : part des demandes servies, par ressource.
    pub taux: &'a [[f32; NB_RES]],
    pub pos_lieux: &'a [[f64; 2]],
    /// Graine du tick (mélangée à l'identifiant).
    pub graine: u64,
}

impl CtxGlobal<'_> {
    pub fn heure(p: &ParamsCivils, tick: u32) -> f32 {
        ((p.heure_debut * 3600.0 + tick as f64 / TPS as f64) % 86_400.0) as f32
    }
    pub fn temperature(p: &ParamsCivils, seconde_jour: f32) -> f32 {
        // Minimum vers 5 h, maximum vers 17 h.
        let phase = (seconde_jour / 86_400.0 - 17.0 / 24.0) * std::f32::consts::TAU;
        p.temperature + p.amplitude * phase.cos()
    }
}

/// Suite de tirages bon marché (une multiplication chacun), à partir d'un
/// seul hachage par colon et par mise à jour.
pub struct Tirages(u64);

impl Tirages {
    #[inline(always)]
    pub fn new(h: u64) -> Self {
        Tirages(h)
    }
    #[inline(always)]
    pub fn u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let z = (self.0 ^ (self.0 >> 32)).wrapping_mul(0xD6E8_FEB8_6659_FD93);
        z ^ (z >> 32)
    }
}

/// Deux flottants de [0, 1) sur 24 bits.
#[inline(always)]
pub fn deux(x: u64) -> (f32, f32) {
    const K: f32 = 1.0 / (1u32 << 24) as f32;
    ((x & 0xFF_FFFF) as f32 * K, ((x >> 24) & 0xFF_FFFF) as f32 * K)
}

/// Quantification avec un tramage de 16 bits.
#[inline(always)]
pub fn q16(x: f32, d: u64) -> u16 {
    // Valeur positive : la troncature vaut `floor`, sans appel de
    // bibliothèque (binaire portable, sans SSE4.1).
    ((x.clamp(0.0, 1.0) * 65_535.0 + (d & 0xFFFF) as f32 * (1.0 / 65_536.0)) as u32).min(65_535) as u16
}

/// Résultats d'un lot de mises à jour, par colonie. Sommes entières :
/// commutatives, donc indépendantes de l'ordre des lots.
#[derive(Default, Clone)]
pub struct Sorties {
    pub demande: Vec<[u64; NB_RES]>,
    pub servi: Vec<[u64; NB_RES]>,
    /// Ticks de travail effectués (main-d'œuvre).
    pub travail: Vec<u64>,
    pub nouveaux_malades: Vec<u64>,
    pub gueris: Vec<u64>,
    pub departs: Vec<Depart>,
    /// Décès : (identifiant, lieu).
    pub morts: Vec<(u32, u32)>,
    /// Eau non potable bue, par lieu : (lieu, prises).
    pub eau_np: Vec<(u32, u32)>,
    /// Compteur courant d'eau non potable (remis à zéro par l'appelant).
    pub eau_np_n: u32,
    pub mises_a_jour: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Depart {
    pub c: Colon,
    pub de: u32,
    pub vers: u32,
    pub arrivee: u32,
    /// Entité du colon s'il est individualisé (bits de `Entity`), sinon
    /// `SANS_ENTITE`.
    pub e: u64,
}

pub const SANS_ENTITE: u64 = u64::MAX;

impl Sorties {
    pub fn preparer(&mut self, colonies: usize) {
        if self.demande.len() != colonies {
            self.demande = vec![[0; NB_RES]; colonies];
            self.servi = vec![[0; NB_RES]; colonies];
            self.travail = vec![0; colonies];
            self.nouveaux_malades = vec![0; colonies];
            self.gueris = vec![0; colonies];
        }
    }
    pub fn vider(&mut self) {
        for v in &mut self.demande {
            *v = [0; NB_RES];
        }
        for v in &mut self.servi {
            *v = [0; NB_RES];
        }
        self.travail.iter_mut().for_each(|x| *x = 0);
        self.nouveaux_malades.iter_mut().for_each(|x| *x = 0);
        self.gueris.iter_mut().for_each(|x| *x = 0);
        self.departs.clear();
        self.morts.clear();
        self.eau_np.clear();
        self.eau_np_n = 0;
        self.mises_a_jour = 0;
    }
    /// Ajoute `o` (les listes sont triées plus tard).
    pub fn fusionner(&mut self, o: &mut Sorties) {
        self.preparer(o.demande.len());
        for c in 0..o.demande.len() {
            for r in 0..NB_RES {
                self.demande[c][r] += o.demande[c][r];
                self.servi[c][r] += o.servi[c][r];
            }
            self.travail[c] += o.travail[c];
            self.nouveaux_malades[c] += o.nouveaux_malades[c];
            self.gueris[c] += o.gueris[c];
        }
        self.departs.append(&mut o.departs);
        self.morts.append(&mut o.morts);
        self.eau_np.append(&mut o.eau_np);
        self.mises_a_jour += o.mises_a_jour;
    }
}

pub enum Issue {
    Reste,
    Part { dest: u32, arrivee: u32 },
    Meurt,
}

/// Début du poste d'un colon (s depuis minuit) : tiré une fois pour toutes
/// de son identifiant.
#[inline(always)]
pub fn debut_poste(p: &ParamsCivils, id: u32) -> f32 {
    let u = ((id as u64 ^ 0x7057E).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f32 / (1u32 << 24) as f32;
    6.0 * 3600.0 + u * p.etalement_poste_h * 3600.0
}

#[inline(always)]
pub fn en_service(p: &ParamsCivils, c: &Colon, s: f32) -> bool {
    if c.travail == NUL {
        return false;
    }
    let d = debut_poste(p, c.id);
    // s et d sont dans [0, 86 400) : un seul repli suffit.
    let mut x = s - d;
    if x < 0.0 {
        x += 86_400.0;
    }
    x < p.poste_h * 3600.0
}

#[inline(always)]
pub fn endormi(s: f32) -> bool {
    !(6.0 * 3600.0..22.0 * 3600.0).contains(&s)
}

/// Durée d'un trajet abstrait (ticks) entre deux lieux.
#[inline(always)]
pub fn trajet(g: &CtxGlobal, a: u32, b: u32) -> u32 {
    let (pa, pb) = (g.pos_lieux[a as usize], g.pos_lieux[b as usize]);
    let d = ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2)).sqrt() as f32;
    ((d * g.p.detour / g.p.marche) * TPS as f32).ceil() as u32 + 1
}

/// Facteur de froid d'une personne (0 à 1) : météo, abri, équipement.
#[inline(always)]
pub fn froid(c: &Colon, l: &CtxLieu, g: &CtxGlobal, dort: bool) -> f32 {
    let mut t = g.temp_air;
    if !l.exterieur {
        t += if l.chauffe { 15.0 } else { 5.0 };
    }
    if c.drapeaux & A_VETEMENTS != 0 {
        t += 7.0;
    }
    if dort && c.drapeaux & A_COUVERTURE != 0 {
        t += 8.0;
    }
    ((12.0 - t) * (1.0 / 20.0)).clamp(0.0, 1.0)
}

/// Demande d'une ressource : servie selon le taux de la colonie (tirage
/// déterministe `u`).
#[inline(always)]
fn demander(r: usize, n: u64, u: f32, col: usize, g: &CtxGlobal, s: &mut Sorties) -> bool {
    s.demande[col][r] += n;
    if u < g.taux[col][r] {
        s.servi[col][r] += n;
        true
    } else {
        false
    }
}

/// Une mise à jour d'un colon qui se trouve dans le lieu `l`. Intervalle
/// exact depuis sa dernière mise à jour (un trajet ou un changement de
/// bâtiment change la phase).
#[inline(always)]
pub fn vivre(c: &mut Colon, l: &CtxLieu, g: &CtxGlobal, s: &mut Sorties) -> Issue {
    let p = g.p;
    let k = &g.k;
    let dt_ticks = g.maintenant.wrapping_sub(c.maj);
    c.maj = g.maintenant;
    if dt_ticks == 0 {
        return Issue::Reste;
    }
    s.mises_a_jour += 1;
    let dt = dt_ticks as f32 * (1.0 / TPS as f32);
    let mut tir = Tirages::new(hacher(g.graine, c.id as u64));
    let col = l.colonie as usize;
    let chez_soi = l.lieu == c.logement;
    let sec = g.seconde_jour;
    let dort = chez_soi && endormi(sec);
    let malade = c.malade != 0;
    let travaille = !chez_soi && l.lieu == c.travail && !malade;

    // Jauges : faim, soif, fatigue.
    let mut faim = f(c.j[FAIM]) + dt * k.faim;
    let mut soif = f(c.j[SOIF]) + dt * k.soif;
    let mut fatigue = f(c.j[FATIGUE]);
    fatigue += dt * if dort {
        -k.recup
    } else if travaille {
        k.fatigue_travail
    } else {
        k.fatigue_eveil
    };
    // Repas et eau : demandés au stock de la colonie (cantine au travail).
    let (u_repas, u_eau) = deux(tir.u64());
    if !dort && faim > p.repas && demander(VIVRES, 1, u_repas, col, g, s) {
        faim -= p.repas;
    }
    if !dort && soif > p.boire {
        if !demander(EAU, 1, u_eau, col, g, s) {
            // Faute d'eau potable : eau non potable, source de risque.
            s.demande[col][EAU_NP] += 1;
            s.servi[col][EAU_NP] += 1;
            s.eau_np_n += 1;
        }
        soif -= p.boire;
    }
    // Froid et équipement.
    let mut fr = froid(c, l, g, dort);
    if fr > 0.0 {
        let (u_v, u_c) = deux(tir.u64());
        if c.drapeaux & A_VETEMENTS == 0 && demander(VETEMENTS, 1, u_v, col, g, s) {
            c.drapeaux |= A_VETEMENTS;
            fr = froid(c, l, g, dort);
        }
        if dort && fr > 0.0 && c.drapeaux & A_COUVERTURE == 0 && demander(COUVERTURES, 1, u_c, col, g, s) {
            c.drapeaux |= A_COUVERTURE;
            fr = froid(c, l, g, dort);
        }
    }
    // Maladie (A-17) : risque du lieu × froid × faim et fatigue.
    let mut issue = Issue::Reste;
    let (u_mal, u_med) = deux(tir.u64());
    if !malade {
        let chance = k.maladie * dt * l.risque * (1.0 + p.poids_froid * fr) * (1.0 + faim + fatigue);
        if u_mal < chance {
            c.malade = 1;
            s.nouveaux_malades[col] += 1;
        }
    } else {
        let soigne = demander(MEDICAMENTS, dt_ticks as u64, u_med, col, g, s);
        if soigne {
            if u_mal < k.guerison * dt {
                c.malade = 0;
                s.gueris[col] += 1;
            }
        } else if u_mal < k.mort * dt {
            issue = Issue::Meurt;
        } else if u_mal < (k.mort + k.guerison_seul) * dt {
            c.malade = 0;
            s.gueris[col] += 1;
        }
    }
    // Moral et usure.
    let cible = (1.0 - 0.4 * faim - 0.3 * soif - 0.2 * fatigue - 0.3 * fr - 0.2 * c.malade as f32).clamp(0.0, 1.0);
    let km = (dt * k.moral).min(1.0);
    let mut moral = f(c.j[MORAL]);
    moral += (cible - moral) * km - p.moral_deces * l.deces as f32;
    let mut usure = f(c.j[USURE]) + p.usure_deces * l.deces as f32;
    if chez_soi {
        usure -= dt * k.usure_recup;
    }
    let tr = tir.u64();
    // Expérience par le travail (A-18) : une unité par minute travaillée.
    if travaille {
        s.travail[col] += dt_ticks as u64;
        let gain = (dt * (1.0 / 60.0) + deux(tr >> 16).0) as u32;
        c.experience = c.experience.saturating_add(gain.min(u16::MAX as u32) as u16);
    }
    let d = tir.u64();
    c.j[FAIM] = q16(faim.max(0.0), d);
    c.j[SOIF] = q16(soif.max(0.0), d >> 16);
    c.j[FATIGUE] = q16(fatigue, d >> 32);
    c.j[MORAL] = q16(moral, d >> 48);
    c.j[USURE] = q16(usure, tr);
    if matches!(issue, Issue::Meurt) {
        return issue;
    }
    // Cycle quotidien : départ au travail, retour au logement.
    let service = en_service(p, c, sec);
    if chez_soi && service && c.malade == 0 {
        return Issue::Part { dest: c.travail, arrivee: g.maintenant + trajet(g, l.lieu, c.travail) };
    }
    if !chez_soi && (!service || c.malade != 0) {
        return Issue::Part { dest: c.logement, arrivee: g.maintenant + trajet(g, l.lieu, c.logement) };
    }
    Issue::Reste
}

/// Empreinte d'un colon (somme de contrôle, insensible à l'ordre).
#[inline(always)]
pub fn empreinte(c: &Colon, ou: u32) -> u64 {
    let a = (c.id as u64) << 32 | c.maj as u64;
    let b = (c.logement as u64) << 32 | c.travail as u64;
    let j = c.j;
    let cc = (j[0] as u64) | (j[1] as u64) << 16 | (j[2] as u64) << 32 | (j[3] as u64) << 48;
    let d = (j[4] as u64) | (c.experience as u64) << 16 | (c.competences as u64) << 32 | (c.malade as u64) << 48
        | (c.drapeaux as u64) << 56;
    hacher(hacher(hacher(a, b), hacher(cc, d)), ou as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taille_du_colon() {
        assert_eq!(std::mem::size_of::<Colon>(), 32);
    }

    #[test]
    fn tramage_sans_biais() {
        // 10 000 incréments de 0,3 pas : la moyenne doit suivre.
        let mut j = 0u16;
        for k in 0..10_000u64 {
            j = q(f(j) + 0.3 / 65_535.0, hacher(7, k));
        }
        assert!((j as i32 - 3_000).abs() < 150, "{j}");
    }
}
