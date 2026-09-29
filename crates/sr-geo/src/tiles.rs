//! Map tiles: the Web Mercator tile grid, which tiles a view needs, and tile
//! geometry on the map.
//!
//! Tiles are chosen by walking the quadtree from the world tile down to the
//! display zoom, keeping each tile whose outline, projected like any
//! geometry, reaches the frame; so a globe gets the tiles of its visible cap
//! and a rotated or conic map the tiles it covers. Tile geometry is clipped to
//! its tile (tiles carry a buffer beyond their edges), then placed: through an
//! exact affine map for Web Mercator views, and through the spherical pipeline
//! (so it curves and clips correctly) for every other projection.

use crate::clip::Rect;
use crate::data::Geometry;
use crate::project::{Planar, Projection, Raw};
use crate::sphere::{DEG, RAD};

/// Latitude limit of the Web Mercator square.
pub const MAX_LAT: f64 = 85.051_128_779_806_59;

/// A tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tile {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

/// Longitude and latitude of a point in tile units (x, y from the tile grid's north-west corner).
pub fn lonlat(z: u8, x: f64, y: f64) -> [f64; 2] {
    let n = (1u64 << z) as f64;
    let lon = x / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * y / n)).sinh().atan() * DEG;
    [lon, lat]
}

/// Tile units of a longitude and latitude at zoom `z`.
pub fn tile_xy(z: u8, lon: f64, lat: f64) -> [f64; 2] {
    let n = (1u64 << z) as f64;
    let lat = lat.clamp(-MAX_LAT, MAX_LAT) * RAD;
    [(lon + 180.0) / 360.0 * n, (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n]
}

/// The map zoom (MapLibre's: a 512-pixel world at zoom 0) of a projection's scale.
pub fn map_zoom(p: &Projection) -> f64 {
    (std::f64::consts::TAU * p.scale() / 512.0).log2()
}

/// The tile zoom that shows `tile_size`-pixel tiles closest to their size.
pub fn tile_zoom(p: &Projection, tile_size: f64, min: u8, max: u8) -> u8 {
    let z = (std::f64::consts::TAU * p.scale() / tile_size).log2().round();
    z.clamp(min as f64, max as f64) as u8
}

/// Longitude and latitude at a screen point of a Web Mercator view (`None` for other projections):
/// the inverse of the view's affine map from (longitude, Mercator y) to the screen.
pub fn screen_lonlat(proj: &Projection, x: f64, y: f64) -> Option<[f64; 2]> {
    if proj.raw() != Raw::Mercator || proj.rotate()[1] != 0.0 || proj.rotate()[2] != 0.0 {
        return None;
    }
    let lon0 = -proj.rotate()[0];
    let merc = |lat: f64| (std::f64::consts::FRAC_PI_4 + lat * RAD / 2.0).tan().ln();
    let o = proj.point_unclipped(lon0, 0.0);
    let ex = proj.point_unclipped(lon0 + 1.0, 0.0);
    let ey = proj.point_unclipped(lon0, 1.0);
    // columns: screen change per degree of longitude, per unit of Mercator y
    let (a, b) = (ex[0] - o[0], ex[1] - o[1]);
    let my = merc(1.0);
    let (c, d) = ((ey[0] - o[0]) / my, (ey[1] - o[1]) / my);
    let det = a * d - b * c;
    if det.abs() < 1e-12 {
        return None;
    }
    let (px, py) = (x - o[0], y - o[1]);
    let dlon = (d * px - c * py) / det;
    let yy = (-b * px + a * py) / det;
    let lat = (2.0 * yy.exp().atan() - std::f64::consts::FRAC_PI_2) * DEG;
    Some([lon0 + dlon, lat])
}

/// The outline of a tile as a lon/lat polygon (edges sampled so they bend as parallels and
/// meridians do), exterior clockwise.
fn outline(t: Tile) -> Geometry {
    const N: usize = 8;
    let mut ring = Vec::with_capacity(4 * N);
    let (x, y) = (t.x as f64, t.y as f64);
    let at = |u: f64, v: f64| lonlat(t.z, x + u, y + v);
    for i in 0..N {
        ring.push(at(i as f64 / N as f64, 0.0));
    }
    for i in 0..N {
        ring.push(at(1.0, i as f64 / N as f64));
    }
    for i in 0..N {
        ring.push(at(1.0 - i as f64 / N as f64, 1.0));
    }
    for i in 0..N {
        ring.push(at(0.0, 1.0 - i as f64 / N as f64));
    }
    Geometry::Polygons(vec![vec![ring]])
}

/// The tiles of zoom `z` whose outline reaches the projection's clip extent (the frame).
pub fn visible(p: &Projection, z: u8) -> Vec<Tile> {
    let mut out = Vec::new();
    let mut stack = vec![Tile { z: 0, x: 0, y: 0 }];
    while let Some(t) = stack.pop() {
        if p.project(&outline(t)).polygons.is_empty() {
            continue;
        }
        if t.z >= z {
            out.push(t);
            continue;
        }
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            stack.push(Tile { z: t.z + 1, x: 2 * t.x + dx, y: 2 * t.y + dy });
        }
    }
    out.sort();
    out
}

/// The tile holding tile `t`'s content at archive zoom `max` (overzooming beyond it), and the
/// square of `t` inside it in the parent's tile units (x, y, size).
pub fn source(t: Tile, max: u8) -> (Tile, [f64; 3]) {
    if t.z <= max {
        return (t, [0.0, 0.0, 1.0]);
    }
    let d = t.z - max;
    let p = Tile { z: max, x: t.x >> d, y: t.y >> d };
    let k = (1u32 << d) as f64;
    (p, [(t.x as f64) / k - p.x as f64, (t.y as f64) / k - p.y as f64, 1.0 / k])
}

/// Places tile geometry on the map.
pub struct Placer<'p> {
    proj: &'p Projection,
    tile: Tile,
    extent: f64,
    /// Tile units → screen, when exact (Web Mercator views).
    affine: Option<[f64; 6]>,
    clip: Rect,
}

impl<'p> Placer<'p> {
    /// A placer for features of `tile` whose coordinates run 0‥`extent` (clipped to the square
    /// `window` of the tile, as from [`source`], so overzoomed tiles keep only their part).
    pub fn new(proj: &'p Projection, tile: Tile, extent: u32, window: [f64; 3]) -> Placer<'p> {
        let e = extent as f64;
        let clip = Rect {
            x0: window[0] * e,
            y0: window[1] * e,
            x1: (window[0] + window[2]) * e,
            y1: (window[1] + window[2]) * e,
        };
        let affine = if proj.raw() == Raw::Mercator && proj.rotate()[1] == 0.0 && proj.rotate()[2] == 0.0 {
            // exact when the tile's longitudes do not wrap round the view's antimeridian
            let [w, n] = lonlat(tile.z, tile.x as f64, tile.y as f64);
            let [east, s] = lonlat(tile.z, tile.x as f64 + 1.0, tile.y as f64 + 1.0);
            let wrap = |l: f64| (l + proj.rotate()[0] + 180.0).rem_euclid(360.0) - 180.0;
            if wrap(w) < wrap(east - 1e-9) {
                let o = proj.point_unclipped(w, n);
                let ex = proj.point_unclipped(east, n);
                let ey = proj.point_unclipped(w, s);
                Some([(ex[0] - o[0]) / e, (ex[1] - o[1]) / e, (ey[0] - o[0]) / e, (ey[1] - o[1]) / e, o[0], o[1]])
            } else {
                None
            }
        } else {
            None
        };
        Placer { proj, tile, extent: e, affine, clip }
    }

    fn to_lonlat(&self, q: [f64; 2]) -> [f64; 2] {
        lonlat(self.tile.z, self.tile.x as f64 + q[0] / self.extent, self.tile.y as f64 + q[1] / self.extent)
    }

    fn map(&self, q: [f64; 2]) -> [f64; 2] {
        let [a, b, c, d, e, f] = self.affine.expect("affine");
        [a * q[0] + c * q[1] + e, b * q[0] + d * q[1] + f]
    }

    /// Lines (tile units) on the map.
    pub fn lines(&self, parts: &[Vec<[f64; 2]>]) -> Vec<Vec<[f64; 2]>> {
        let clipped: Vec<Vec<[f64; 2]>> = parts.iter().flat_map(|l| self.clip.clip_line(l)).collect();
        match self.affine {
            Some(_) => {
                let post = self.proj.extent_rect();
                clipped
                    .iter()
                    .flat_map(|l| {
                        let m: Vec<[f64; 2]> = l.iter().map(|q| self.map(*q)).collect();
                        match post {
                            Some(r) => r.clip_line(&m),
                            None => vec![m],
                        }
                    })
                    .collect()
            }
            None => {
                let g =
                    Geometry::Lines(clipped.iter().map(|l| l.iter().map(|q| self.to_lonlat(*q)).collect()).collect());
                self.proj.project(&g).lines
            }
        }
    }

    /// Polygons (tile units, each exterior ring then its holes, MVT winding) on the map: rings
    /// to fill non-zero.
    pub fn polygons(&self, polys: &[Vec<Vec<[f64; 2]>>]) -> Vec<Vec<[f64; 2]>> {
        let mut out = Vec::new();
        for rings in polys {
            let clipped = self.clip.clip_polygon(rings);
            if clipped.is_empty() {
                continue;
            }
            match self.affine {
                Some(_) => {
                    let m: Vec<Vec<[f64; 2]>> =
                        clipped.iter().map(|r| r.iter().map(|q| self.map(*q)).collect()).collect();
                    match self.proj.extent_rect() {
                        Some(r) => out.extend(r.clip_polygon(&m)),
                        None => out.extend(m),
                    }
                }
                None => {
                    // an MVT exterior (positive area with y down) runs east, south, west, north:
                    // clockwise in longitude and latitude, as the spherical pipeline wants
                    let ll: Vec<Vec<[f64; 2]>> =
                        clipped.iter().map(|r| r.iter().map(|q| self.to_lonlat(*q)).collect()).collect();
                    let pl: Planar = self.proj.project(&Geometry::Polygons(vec![ll]));
                    out.extend(pl.polygons.into_iter().flatten());
                }
            }
        }
        out
    }

    /// Points (tile units) on the map, inside the tile and visible.
    pub fn points(&self, parts: &[Vec<[f64; 2]>]) -> Vec<[f64; 2]> {
        parts
            .iter()
            .flatten()
            .filter(|q| q[0] >= self.clip.x0 && q[0] < self.clip.x1 && q[1] >= self.clip.y0 && q[1] < self.clip.y1)
            .filter_map(|q| {
                let [lon, lat] = self.to_lonlat(*q);
                self.proj.point(lon, lat)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{Kind, Map, View};

    #[test]
    fn tile_coordinates_round_trip() {
        let [x, y] = tile_xy(14, -9.1375, 38.711);
        assert_eq!((x as u32, y as u32), (7776, 6278));
        let [lon, lat] = lonlat(14, x, y);
        assert!((lon + 9.1375).abs() < 1e-9 && (lat - 38.711).abs() < 1e-9);
        assert!((lonlat(1, 1.0, 0.0)[1] - MAX_LAT).abs() < 1e-9);
    }

    #[test]
    fn views_pick_the_tiles_they_show() {
        // Web Mercator zoom 2 over (0, 0): the 512 × 512 frame shows the whole zoom-1 world
        let (m, _) = Map::new(Kind::WebMercator, None, [512.0, 512.0], &[], 0.0, Some([0.0, 0.0]));
        let p = m.projection(&View { lon: 0.0, lat: 0.0, zoom: 0.0, rotation: 0.0 });
        assert_eq!(tile_zoom(&p, 512.0, 0, 14), 0);
        assert_eq!(tile_zoom(&p, 256.0, 0, 14), 1);
        assert_eq!(visible(&p, 1).len(), 4);
        // zoomed in on Lisbon, a 1024 × 512 frame at zoom 14 needs a few z14 tiles round it
        let (m, _) = Map::new(Kind::WebMercator, None, [1024.0, 512.0], &[], 0.0, Some([-9.1375, 38.711]));
        let p = m.projection(&View { lon: -9.1375, lat: 38.711, zoom: 14.0, rotation: 0.0 });
        let ts = visible(&p, 14);
        assert!(ts.len() >= 3 && ts.len() <= 9, "{ts:?}");
        assert!(ts.contains(&tile_of(14, -9.1375, 38.711)));
        // a globe facing Europe does not ask for the Pacific
        let (m, _) = Map::new(Kind::Orthographic, None, [512.0, 512.0], &[], 0.0, Some([10.0, 50.0]));
        let p = m.projection(&View { lon: 10.0, lat: 50.0, zoom: 0.0, rotation: 0.0 });
        let ts = visible(&p, 3);
        assert!(ts.contains(&tile_of(3, 10.0, 50.0)) && !ts.contains(&tile_of(3, -160.0, 0.0)), "{ts:?}");
    }

    fn tile_of(z: u8, lon: f64, lat: f64) -> Tile {
        let [x, y] = tile_xy(z, lon, lat);
        Tile { z, x: x as u32, y: y as u32 }
    }

    #[test]
    fn the_fast_path_agrees_with_the_sphere() {
        let (m, _) = Map::new(Kind::WebMercator, None, [800.0, 600.0], &[], 0.0, Some([-9.14, 38.71]));
        let p = m.projection(&View { lon: -9.14, lat: 38.71, zoom: 13.0, rotation: 0.0 });
        let t = tile_of(13, -9.14, 38.71);
        let fast = Placer::new(&p, t, 4096, [0.0, 0.0, 1.0]);
        assert!(fast.affine.is_some());
        let mut slow = Placer::new(&p, t, 4096, [0.0, 0.0, 1.0]);
        slow.affine = None;
        let sq = vec![vec![[100.0, 100.0], [900.0, 100.0], [900.0, 700.0], [100.0, 700.0]]];
        let (a, b) = (fast.polygons(std::slice::from_ref(&sq)), slow.polygons(std::slice::from_ref(&sq)));
        let area = |rs: &Vec<Vec<[f64; 2]>>| rs.iter().map(|r| crate::mvt::signed_area(r).abs()).sum::<f64>();
        assert!((area(&a) - area(&b)).abs() < 1e-3 * area(&a), "{} {}", area(&a), area(&b));
        let (la, lb) =
            (fast.lines(&[vec![[0.0, 0.0], [4096.0, 4096.0]]]), slow.lines(&[vec![[0.0, 0.0], [4096.0, 4096.0]]]));
        let (ea, eb) = (la[0].last().unwrap(), lb[0].last().unwrap());
        assert!((ea[0] - eb[0]).abs() < 0.1 && (ea[1] - eb[1]).abs() < 0.1, "{ea:?} {eb:?}");
    }

    #[test]
    fn screen_points_invert() {
        let (m, _) = Map::new(Kind::WebMercator, None, [800.0, 600.0], &[], 0.0, Some([-9.14, 38.71]));
        let p = m.projection(&View { lon: -9.14, lat: 38.71, zoom: 12.0, rotation: 30.0 });
        let q = p.point_unclipped(-9.2, 38.75);
        let ll = screen_lonlat(&p, q[0], q[1]).unwrap();
        assert!((ll[0] + 9.2).abs() < 1e-9 && (ll[1] - 38.75).abs() < 1e-9, "{ll:?}");
    }

    #[test]
    fn overzoom_takes_the_parents_quarter() {
        let (p, w) = source(Tile { z: 16, x: 4 * 7775 + 1, y: 4 * 6278 + 3 }, 14);
        assert_eq!(p, Tile { z: 14, x: 7775, y: 6278 });
        assert_eq!(w, [0.25, 0.75, 0.25]);
    }
}
