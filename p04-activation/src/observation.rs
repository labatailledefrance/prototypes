//! Mode observation : une image du bloc toutes les `fenetre` secondes, à
//! 5 m par pixel. Bâti en gris, tiles inactives assombries (brouillard de
//! guerre), tiles en chantier teintées de bleu, densité de passage des
//! zombies du jaune au violet (échelle logarithmique), humains en vert,
//! barricades en rouge, frontières de tiles en pointillé.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::sim::{Corps, EtatTuile, Simulation};
use crate::ville::COTE;

const PIX: usize = 5;

pub struct Observateur {
    dossier: PathBuf,
    etiquette: String,
    fenetre: u64,
    w: usize,
    h: usize,
    ox: f64,
    oy: f64,
    bati: Vec<u8>,
    passages: Vec<u32>,
    debut: u64,
}

impl Observateur {
    pub fn new(dossier: &Path, etiquette: &str, fenetre: u32, sim: &Simulation) -> Observateur {
        std::fs::create_dir_all(dossier).expect("dossier d'observation");
        let c = sim.carte();
        let g = c.geo;
        let (w, h) = (g.w * COTE / PIX, g.h * COTE / PIX);
        let mut bati = vec![0u8; w * h];
        for py in 0..h {
            for px in 0..w {
                let (t, lx, ly) = (g.tile((px * PIX / COTE) as i32, (py * PIX / COTE) as i32).unwrap(), (px * PIX) % COTE, (py * PIX) % COTE);
                let mut n = 0;
                for dy in 0..PIX {
                    for dx in 0..PIX {
                        n += c.tuiles[t].ville.bati((lx + dx) as i32, (ly + dy) as i32) as u8;
                    }
                }
                bati[py * w + px] = n;
            }
        }
        Observateur {
            dossier: dossier.to_path_buf(),
            etiquette: etiquette.to_string(),
            fenetre: fenetre as u64 * 20,
            w,
            h,
            ox: g.x0 as f64 * 1000.0,
            oy: g.y0 as f64 * 1000.0,
            bati,
            passages: vec![0; w * h],
            debut: sim.horloge(),
        }
    }

    pub fn tick(&mut self, sim: &mut Simulation) {
        let k = sim.horloge() - self.debut;
        if k % 10 == 0 {
            let mut q = sim.world.query::<&Corps>();
            for c in q.iter(&sim.world) {
                let (px, py) = (((c.pos[0] - self.ox) / PIX as f64) as usize, ((c.pos[1] - self.oy) / PIX as f64) as usize);
                if px < self.w && py < self.h {
                    self.passages[py * self.w + px] += 1;
                }
            }
        }
        if k > 0 && k % self.fenetre == 0 {
            self.image(sim, k / 20);
            self.passages.fill(0);
        }
    }

    fn image(&self, sim: &Simulation, t_s: u64) {
        let g = sim.carte().geo;
        let topo = sim.topo();
        let mut img = vec![0u8; self.w * self.h * 3];
        let vmax = (self.fenetre as f32 / 10.0 * 8.0).ln_1p();
        for py in 0..self.h {
            for px in 0..self.w {
                let t = g.tile((px * PIX / COTE) as i32, (py * PIX / COTE) as i32).unwrap();
                let b = self.bati[py * self.w + px] as f32 / (PIX * PIX) as f32;
                let mut c = [245.0 - 120.0 * b, 245.0 - 120.0 * b, 245.0 - 120.0 * b];
                let n = self.passages[py * self.w + px];
                if n > 0 {
                    let f = ((n as f32).ln_1p() / vmax).min(1.0);
                    c = [250.0 - 160.0 * f, 220.0 - 200.0 * f, 60.0 + 100.0 * f];
                }
                match topo.etat[t] {
                    EtatTuile::Inerte => c = c.map(|v| v * 0.55),
                    EtatTuile::Chantier(_) => c = [c[0] * 0.6, c[1] * 0.7, c[2] * 0.6 + 90.0],
                    EtatTuile::Actif(_) => {}
                }
                let (lx, ly) = ((px * PIX) % COTE, (py * PIX) % COTE);
                if (lx < PIX || ly < PIX) && (px + py) % 4 < 2 {
                    c = [40.0, 40.0, 40.0];
                }
                // Image : nord en haut.
                let i = ((self.h - 1 - py) * self.w + px) * 3;
                img[i..i + 3].copy_from_slice(&c.map(|v| v.clamp(0.0, 255.0) as u8));
            }
        }
        let mut poser = |p: [f64; 2], c: [u8; 3]| {
            let (px, py) = (((p[0] - self.ox) / PIX as f64) as usize, ((p[1] - self.oy) / PIX as f64) as usize);
            if px < self.w && py < self.h {
                let i = ((self.h - 1 - py) * self.w + px) * 3;
                img[i..i + 3].copy_from_slice(&c);
            }
        };
        for (t, tu) in sim.carte().tuiles.iter().enumerate() {
            let o = g.origine(t);
            for b in &tu.barricades {
                let pv = b.pv.load(std::sync::atomic::Ordering::Relaxed);
                poser([o[0] + b.x as f64, o[1] + b.y as f64], if pv > 0 { [220, 30, 30] } else { [90, 20, 20] });
            }
        }
        for p in &sim.humains().pos {
            poser(*p, [20, 170, 40]);
        }
        let f = self.dossier.join(format!("{}-{:04}s.ppm", self.etiquette, t_s));
        let mut w = std::io::BufWriter::new(std::fs::File::create(&f).expect("image"));
        write!(w, "P6\n{} {}\n255\n", self.w, self.h).unwrap();
        w.write_all(&img).unwrap();
    }

    pub fn finir(self) {}
}
