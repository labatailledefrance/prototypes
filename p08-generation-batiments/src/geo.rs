//! Grille des tiles (copiée de P-07, `geo.rs`) : tiles de 1 km sur le
//! Lambert-93, positions locales centrées sur la tile (C-01).

pub const TILE: f64 = 1000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub ix: i32,
    pub iy: i32,
}

impl TileId {
    pub fn x0(&self) -> f64 {
        self.ix as f64 * TILE
    }
    pub fn y0(&self) -> f64 {
        self.iy as f64 * TILE
    }
    /// Centre de la tile : origine des positions locales f32 centrées (C-01).
    pub fn centre(&self) -> (f64, f64) {
        (self.x0() + 500.0, self.y0() + 500.0)
    }
    pub fn nom(&self) -> String {
        format!("x{}_y{}", self.ix, self.iy)
    }
    pub fn lire(s: &str) -> TileId {
        let s = s.split(':').next().unwrap();
        let (a, b) = s.split_once('_').expect("tile xNNN_yNNNN");
        TileId { ix: a[1..].parse().unwrap(), iy: b[1..].parse().unwrap() }
    }
    pub fn voisine(&self, dx: i32, dy: i32) -> TileId {
        TileId { ix: self.ix + dx, iy: self.iy + dy }
    }
}
