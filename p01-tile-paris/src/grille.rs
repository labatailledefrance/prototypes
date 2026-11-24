//! Grille spatiale uniforme, reconstruite à chaque tick par tri par
//! dénombrement (counting sort).
//!
//! Analogie : un index inversé « case → occupants » reconstruit de zéro à
//! chaque tick plutôt que mis à jour : deux passes linéaires, aucun verrou,
//! et des voisins rangés côte à côte en mémoire.

#[derive(Clone, Copy, Default)]
pub struct Elem {
    pub p: [f32; 2],
    pub id: u32,
    /// Alerte relayable : tick (+1) de la dernière observation directe de la
    /// cible ; 0 = aucune alerte.
    pub alerte: u32,
    pub cible: [f32; 2],
}

pub struct Grille {
    pub cote: usize,
    inv_taille: f32,
    /// `debut[c]..debut[c + 1]` : occupants de la case c dans `elems`.
    debut: Vec<u32>,
    curseur: Vec<u32>,
    pub elems: Vec<Elem>,
}

impl Grille {
    /// Grille couvrant un carré de côté `etendue`, cases de côté `taille`.
    pub fn new(etendue: f32, taille: f32) -> Self {
        let cote = (etendue / taille).ceil() as usize;
        Grille {
            cote,
            inv_taille: 1.0 / taille,
            debut: vec![0; cote * cote + 1],
            curseur: vec![0; cote * cote],
            elems: Vec::new(),
        }
    }

    #[inline(always)]
    pub fn coord(&self, p: [f32; 2]) -> (i32, i32) {
        let m = self.cote as i32 - 1;
        (
            ((p[0] * self.inv_taille) as i32).clamp(0, m),
            ((p[1] * self.inv_taille) as i32).clamp(0, m),
        )
    }

    /// Reconstruit la grille à partir de `n` points ; l'identifiant d'un
    /// élément est son rang. Ordre déterministe.
    pub fn reconstruire(&mut self, n: usize, point: impl Fn(usize) -> [f32; 2]) {
        self.reconstruire_avec(n, point, |_| (0, [0.0; 2]));
    }

    /// Idem, avec l'alerte de chaque élément.
    pub fn reconstruire_avec(
        &mut self,
        n: usize,
        point: impl Fn(usize) -> [f32; 2],
        alerte: impl Fn(usize) -> (u32, [f32; 2]),
    ) {
        self.debut.fill(0);
        for i in 0..n {
            let (x, y) = self.coord(point(i));
            self.debut[y as usize * self.cote + x as usize + 1] += 1;
        }
        for c in 1..self.debut.len() {
            self.debut[c] += self.debut[c - 1];
        }
        self.curseur.copy_from_slice(&self.debut[..self.debut.len() - 1]);
        self.elems.resize(n, Elem::default());
        for i in 0..n {
            let p = point(i);
            let (x, y) = self.coord(p);
            let c = &mut self.curseur[y as usize * self.cote + x as usize];
            let (alerte, cible) = alerte(i);
            self.elems[*c as usize] = Elem {
                p,
                id: i as u32,
                alerte,
                cible,
            };
            *c += 1;
        }
    }

    /// Occupants de la case (x, y), supposée dans la grille.
    #[inline(always)]
    pub fn case(&self, x: i32, y: i32) -> &[Elem] {
        let c = y as usize * self.cote + x as usize;
        &self.elems[self.debut[c] as usize..self.debut[c + 1] as usize]
    }

    /// Plus grand nombre d'occupants d'une case.
    pub fn densite_max(&self) -> u32 {
        self.debut.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0)
    }
}
