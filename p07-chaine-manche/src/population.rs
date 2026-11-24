//! Habitants initiaux par bâtiment, tirés des carreaux de 200 m de l'INSEE
//! (Filosofi 2021), d'où le nombre initial de zombies par tile et par
//! fragment (A-10, A-19).
//!
//! Les carreaux sont sur la grille INSPIRE en ETRS89-LAEA (EPSG 3035), pas en
//! Lambert-93 : un carreau est à cheval sur plusieurs tiles. Répartition
//! **dasymétrique** : les habitants d'un carreau vont à ses bâtiments
//! résidentiels, au prorata de leur plancher ; à défaut, à ses autres
//! bâtiments (sauf annexes) ; à défaut, à 100 points répartis sur le carreau.
//!
//! Conservation exacte : chaque carreau porte un entier (ind arrondi), réparti
//! par la méthode du plus fort reste, égalités départagées par l'identifiant.
//! Le résultat ne dépend que des données, pas de l'ordre de traitement : deux
//! départements voisins qui ont le même bâti dans leur zone tampon répartissent
//! un carreau frontalier de la même façon (aucune perte, aucun doublon).

use crate::geo::Laea2L93;
use crate::objets::Batiment;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::path::Path;

pub struct Carreau {
    pub e: i64, // coin sud-ouest, m LAEA
    pub n: i64,
    pub ind: f64,
    pub communes: String,
}

/// Lit les carreaux de 200 m dont le coin est dans la boîte LAEA donnée.
pub fn lire_200m(chemin: &Path, boite: [f64; 4]) -> (Vec<Carreau>, usize) {
    let f = File::open(chemin).expect("Parquet des carreaux");
    let r = SerializedFileReader::new(f).expect("lecture Parquet");
    let mut out = Vec::new();
    let mut total = 0;
    for ligne in r.get_row_iter(None).unwrap() {
        let ligne = ligne.unwrap();
        total += 1;
        let (mut id, mut ind, mut lcog) = (String::new(), 0.0f64, String::new());
        for (nom, champ) in ligne.get_column_iter() {
            match (nom.as_str(), champ) {
                ("idcar_200m", Field::Str(s)) => id = s.clone(),
                ("ind", Field::Double(v)) => ind = *v,
                ("ind", Field::Float(v)) => ind = *v as f64,
                ("ind", Field::Long(v)) => ind = *v as f64,
                ("ind", Field::Int(v)) => ind = *v as f64,
                ("lcog_geo", Field::Str(s)) => lcog = s.clone(),
                _ => {}
            }
        }
        // « CRS3035RES200mN2892000E3172000 » : coin sud-ouest.
        let pn = id.find("mN").expect("identifiant de carreau") + 1;
        let pe = id.rfind('E').unwrap();
        let n: i64 = id[pn + 1..pe].parse().unwrap();
        let e: i64 = id[pe + 1..].parse().unwrap();
        if (e as f64) + 200.0 < boite[0] || (e as f64) > boite[2] || (n as f64) + 200.0 < boite[1] || (n as f64) > boite[3] {
            continue;
        }
        out.push(Carreau { e, n, ind, communes: lcog });
    }
    out.sort_by_key(|c| (c.n, c.e));
    (out, total)
}

/// Résultat de la répartition.
pub struct Repartition {
    /// Habitants par bâtiment (indice dans la liste des bâtiments).
    pub par_bat: HashMap<usize, u32>,
    /// Habitants posés sur des points (carreaux sans bâtiment) : (x, y L93, n).
    pub points: Vec<(f64, f64, u32)>,
    pub carreaux: usize,
    pub somme_ind: f64,
    pub somme_entiers: u64,
    pub sur_residentiel: usize,
    pub sur_autres: usize,
    pub sur_points: usize,
}

/// Plus fort reste : répartit `total` selon `poids` (≥ 0), égalités
/// départagées par la clé.
pub fn plus_fort_reste(total: u64, poids: &[(u64, f64)]) -> Vec<u64> {
    let s: f64 = poids.iter().map(|p| p.1).sum();
    if s <= 0.0 {
        let mut v = vec![0; poids.len()];
        if !v.is_empty() {
            v[0] = total;
        }
        return v;
    }
    let mut parts: Vec<u64> = Vec::with_capacity(poids.len());
    let mut restes: Vec<(f64, u64, usize)> = Vec::with_capacity(poids.len());
    let mut donne = 0u64;
    for (k, (cle, p)) in poids.iter().enumerate() {
        let x = total as f64 * p / s;
        let e = x.floor() as u64;
        parts.push(e);
        donne += e;
        restes.push((x - e as f64, *cle, k));
    }
    restes.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(a.1.cmp(&b.1)));
    let mut reste = total - donne;
    for (_, _, k) in restes {
        if reste == 0 {
            break;
        }
        parts[k] += 1;
        reste -= 1;
    }
    parts
}

pub fn repartir(carreaux: &[Carreau], bats: &[Batiment], conv: &Laea2L93) -> Repartition {
    // Chaque bâtiment dans le carreau qui contient son centroïde (en LAEA).
    let mut par_carreau: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (k, b) in bats.iter().enumerate() {
        let (lat, lon) = conv.l93.inverse(b.cx, b.cy);
        let (e, n) = conv.laea.avant(lat, lon);
        let c = ((e / 200.0).floor() as i64 * 200, (n / 200.0).floor() as i64 * 200);
        par_carreau.entry(c).or_default().push(k);
    }
    let mut r = Repartition {
        par_bat: HashMap::new(),
        points: Vec::new(),
        carreaux: 0,
        somme_ind: 0.0,
        somme_entiers: 0,
        sur_residentiel: 0,
        sur_autres: 0,
        sur_points: 0,
    };
    for c in carreaux {
        let total = c.ind.round().max(0.0) as u64;
        r.carreaux += 1;
        r.somme_ind += c.ind;
        r.somme_entiers += total;
        if total == 0 {
            continue;
        }
        let vide = Vec::new();
        let ks = par_carreau.get(&(c.e, c.n)).unwrap_or(&vide);
        let mut poids: Vec<(u64, f64, usize)> =
            ks.iter().filter(|&&k| bats[k].residentiel()).map(|&k| (bats[k].id, bats[k].plancher(), k)).collect();
        if !poids.is_empty() {
            r.sur_residentiel += 1;
        } else {
            poids = ks.iter().filter(|&&k| bats[k].usage1 != 2 && !bats[k].leger).map(|&k| (bats[k].id, bats[k].plancher(), k)).collect();
            if !poids.is_empty() {
                r.sur_autres += 1;
            }
        }
        if !poids.is_empty() {
            poids.sort_by_key(|p| p.0);
            let parts = plus_fort_reste(total, &poids.iter().map(|p| (p.0, p.1)).collect::<Vec<_>>());
            for (p, n) in poids.iter().zip(parts) {
                if n > 0 {
                    *r.par_bat.entry(p.2).or_default() += n as u32;
                }
            }
        } else {
            // 100 points au centre de cases de 20 m.
            r.sur_points += 1;
            let parts = plus_fort_reste(total, &(0..100u64).map(|k| (k, 1.0)).collect::<Vec<_>>());
            for (k, n) in parts.iter().enumerate() {
                if *n == 0 {
                    continue;
                }
                let (e, nn) = (c.e as f64 + 10.0 + 20.0 * (k % 10) as f64, c.n as f64 + 10.0 + 20.0 * (k / 10) as f64);
                let (x, y) = conv.conv(e, nn);
                r.points.push((x, y, *n as u32));
            }
        }
    }
    r
}
