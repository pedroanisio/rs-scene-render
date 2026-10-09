//! The map camera: which projection, where it looks, how far in, and smooth
//! moves between views.
//!
//! * Cylindrical, pseudo-cylindrical and conic projections turn the globe
//!   about its axis to bring the centre longitude to the middle and shift to
//!   the centre latitude, so parallels stay straight; azimuthal projections
//!   (orthographic, stereographic, equal-area, equidistant) turn the globe to
//!   face the centre.
//! * `zoom` counts doublings from the fitted view (zoom 0 shows the fit
//!   target: the whole sphere by default); Web Mercator uses MapLibre's
//!   absolute zoom levels instead (the world is 512·2^zoom pixels wide).
//! * A fly-to follows van Wijk and Nuij's optimal path (d3-interpolate's
//!   `interpolateZoom`), zooming out as far as the distance warrants.

use std::f64::consts::{SQRT_2, TAU};

use crate::data::Geometry;
use crate::project::{Projection, Raw};
use crate::sphere::{DEG, RAD};

/// A projection by name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Mercator,
    WebMercator,
    Equirectangular,
    EqualEarth,
    NaturalEarth,
    Albers,
    LambertConformal,
    Orthographic,
    Stereographic,
    AzimuthalEqualArea,
    AzimuthalEquidistant,
}

impl Kind {
    /// Parses a schema name.
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "mercator" => Kind::Mercator,
            "web-mercator" => Kind::WebMercator,
            "equirectangular" => Kind::Equirectangular,
            "equal-earth" => Kind::EqualEarth,
            "natural-earth" => Kind::NaturalEarth,
            "albers" => Kind::Albers,
            "lambert-conformal" => Kind::LambertConformal,
            "orthographic" => Kind::Orthographic,
            "stereographic" => Kind::Stereographic,
            "azimuthal-equal-area" => Kind::AzimuthalEqualArea,
            "azimuthal-equidistant" => Kind::AzimuthalEquidistant,
            _ => return None,
        })
    }

    /// Whether the projection faces its centre (turns the globe on both axes).
    pub fn azimuthal(&self) -> bool {
        matches!(self, Kind::Orthographic | Kind::Stereographic | Kind::AzimuthalEqualArea | Kind::AzimuthalEquidistant)
    }

    /// The raw projection; conics use `parallels` (degrees; d3's defaults otherwise).
    pub fn raw(&self, parallels: Option<[f64; 2]>) -> Raw {
        match self {
            Kind::Mercator | Kind::WebMercator => Raw::Mercator,
            Kind::Equirectangular => Raw::Equirectangular,
            Kind::EqualEarth => Raw::EqualEarth,
            Kind::NaturalEarth => Raw::NaturalEarth,
            Kind::Albers => {
                let p = parallels.unwrap_or([29.5, 45.5]);
                Raw::conic_equal_area(p[0] * RAD, p[1] * RAD)
            }
            Kind::LambertConformal => {
                let p = parallels.unwrap_or([30.0, 30.0]);
                Raw::conic_conformal(p[0] * RAD, p[1] * RAD)
            }
            Kind::Orthographic => Raw::Orthographic,
            Kind::Stereographic => Raw::Stereographic,
            Kind::AzimuthalEqualArea => Raw::AzimuthalEqualArea,
            Kind::AzimuthalEquidistant => Raw::AzimuthalEquidistant,
        }
    }
}

/// Where the map looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Centre longitude and latitude (degrees).
    pub lon: f64,
    pub lat: f64,
    /// Doublings from the fitted view (Web Mercator: absolute zoom level).
    pub zoom: f64,
    /// Turn of the map on screen (degrees, clockwise).
    pub rotation: f64,
}

/// A map: projection, frame and the scale at zoom 0.
#[derive(Clone, Debug, PartialEq)]
pub struct Map {
    pub kind: Kind,
    pub parallels: Option<[f64; 2]>,
    /// Frame size in pixels.
    pub size: [f64; 2],
    /// Pixels per radian at zoom 0.
    pub base_scale: f64,
    /// A floor on the effective scale (`base_scale * 2^zoom`), in pixels per radian; 0 for none. A conformal cone
    /// sends the pole opposite its parallels to infinity, so the fit of the whole sphere has no finite extent and
    /// `base_scale` collapses to a view that draws nothing; the floor is the scale at which a window of one
    /// equirectangular world round the centre fills the frame. Views that already drew more keep their meaning.
    pub min_scale: f64,
    /// Extra room round the frame kept by the planar clip (pixels), for strokes and markers.
    pub margin: f64,
    /// Resampling precision (pixels).
    pub precision: f64,
}

/// Web Mercator's scale at zoom 0: a 512-pixel world (MapLibre).
const WEB_MERCATOR_BASE: f64 = 512.0 / TAU;

impl Map {
    /// A map whose zoom 0 shows `fit` (the whole sphere when empty) inside the frame less
    /// `padding`, looking at `center` (the fit target's middle when `None`).
    pub fn new(
        kind: Kind,
        parallels: Option<[f64; 2]>,
        size: [f64; 2],
        fit: &[&Geometry],
        padding: f64,
        center: Option<[f64; 2]>,
    ) -> (Map, [f64; 2]) {
        let sphere = Geometry::Sphere;
        let targets: Vec<&Geometry> = if fit.is_empty() { vec![&sphere] } else { fit.to_vec() };
        let center = center.unwrap_or_else(|| {
            let mut b: Option<[[f64; 2]; 2]> = None;
            for g in &targets {
                if let Some(e) = g.extent() {
                    b = Some(match b {
                        None => e,
                        Some(b) => {
                            [[b[0][0].min(e[0][0]), b[0][1].min(e[0][1])], [b[1][0].max(e[1][0]), b[1][1].max(e[1][1])]]
                        }
                    });
                }
            }
            b.map(|b| [(b[0][0] + b[1][0]) / 2.0, (b[0][1] + b[1][1]) / 2.0]).unwrap_or([0.0, 0.0])
        });
        let mut map =
            Map { kind, parallels, size, base_scale: 150.0, min_scale: 0.0, margin: 0.0, precision: 0.5_f64.sqrt() };
        if kind == Kind::WebMercator {
            map.base_scale = WEB_MERCATOR_BASE;
            return (map, center);
        }
        // Bounds of the targets at scale 150 round the centre's position, then the largest
        // scale that keeps them inside the padded frame with the centre in the middle.
        let view = View { lon: center[0], lat: center[1], zoom: 0.0, rotation: 0.0 };
        let mut p = map.projection(&view);
        p.set_extent(None).set_translate(0.0, 0.0);
        let c = p.point_unclipped(center[0], center[1]);
        let mut reach = [0f64; 2];
        for g in targets {
            if let Some(b) = p.project(g).bounds() {
                reach[0] = reach[0].max((b[0][0] - c[0]).abs()).max((b[1][0] - c[0]).abs());
                reach[1] = reach[1].max((b[0][1] - c[1]).abs()).max((b[1][1] - c[1]).abs());
            }
        }
        let half = [(size[0] / 2.0 - padding).max(1.0), (size[1] / 2.0 - padding).max(1.0)];
        let fitted = |reach: [f64; 2]| (half[0] / reach[0].max(1e-9)).min(half[1] / reach[1].max(1e-9));
        let k = fitted(reach);
        if k.is_finite() && k > 0.0 {
            map.base_scale = 150.0 * k;
        }
        // Without a `fit`, a conformal cone's whole-sphere fit collapses (a default `lambert-conformal` map drew
        // nothing): the effective scale is floored at the fit of a window of one equirectangular world.
        if fit.is_empty() && kind == Kind::LambertConformal {
            let window = std::f64::consts::PI * 150.0;
            let floor = 150.0 * fitted([reach[0].min(window), reach[1].min(window)]);
            if floor.is_finite() && floor > map.base_scale {
                map.min_scale = floor;
            }
        }
        (map, center)
    }

    /// The projection showing `view`.
    pub fn projection(&self, view: &View) -> Projection {
        let mut p = Projection::new(self.kind.raw(self.parallels));
        p.set_scale((self.base_scale * 2f64.powf(view.zoom)).max(self.min_scale))
            .set_translate(self.size[0] / 2.0, self.size[1] / 2.0)
            .set_angle(-view.rotation)
            .set_precision(self.precision);
        if self.kind.azimuthal() {
            p.set_rotate([-view.lon, -view.lat, 0.0]);
        } else {
            p.set_rotate([-view.lon, 0.0, 0.0]).set_center(0.0, view.lat);
        }
        let m = self.margin;
        p.set_extent(Some([[-m, -m], [self.size[0] + m, self.size[1] + m]]));
        p
    }

    /// The width of the view in degrees at `zoom` (the fly-to's measure of scale).
    pub fn view_width(&self, zoom: f64) -> f64 {
        self.size[0] / (self.base_scale * 2f64.powf(zoom)) * DEG
    }

    /// The zoom showing a view `width` degrees wide.
    pub fn zoom_for_width(&self, width: f64) -> f64 {
        (self.size[0] / (width * RAD) / self.base_scale).log2()
    }
}

/// van Wijk and Nuij's smooth zoom-and-pan between [ux, uy, w] views (d3-interpolate `interpolateZoom`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomPath {
    p0: [f64; 3],
    d: [f64; 2],
    rho: f64,
    s: f64,
    general: Option<(f64, f64)>, // (d1, r0)
}

fn cosh(x: f64) -> f64 {
    let e = x.exp();
    (e + 1.0 / e) / 2.0
}
fn sinh(x: f64) -> f64 {
    let e = x.exp();
    (e - 1.0 / e) / 2.0
}
fn tanh(x: f64) -> f64 {
    let e = (2.0 * x).exp();
    (e - 1.0) / (e + 1.0)
}

impl ZoomPath {
    /// The path from `p0` to `p1` with curvature `rho` (√2 by default in d3).
    pub fn new(p0: [f64; 3], p1: [f64; 3], rho: f64) -> ZoomPath {
        let rho = rho.max(1e-3);
        let (rho2, rho4) = (rho * rho, rho * rho * rho * rho);
        let d = [p1[0] - p0[0], p1[1] - p0[1]];
        let d2 = d[0] * d[0] + d[1] * d[1];
        let (w0, w1) = (p0[2], p1[2]);
        if d2 < 1e-12 {
            return ZoomPath { p0, d, rho, s: (w1 / w0).ln() / rho, general: None };
        }
        let d1 = d2.sqrt();
        let b0 = (w1 * w1 - w0 * w0 + rho4 * d2) / (2.0 * w0 * rho2 * d1);
        let b1 = (w1 * w1 - w0 * w0 - rho4 * d2) / (2.0 * w1 * rho2 * d1);
        let r0 = ((b0 * b0 + 1.0).sqrt() - b0).ln();
        let r1 = ((b1 * b1 + 1.0).sqrt() - b1).ln();
        ZoomPath { p0, d, rho, s: (r1 - r0) / rho, general: Some((d1, r0)) }
    }

    /// d3's recommended duration in seconds.
    pub fn duration(&self) -> f64 {
        self.s * self.rho / SQRT_2
    }

    /// The view at fraction `t`.
    pub fn at(&self, t: f64) -> [f64; 3] {
        let [ux0, uy0, w0] = self.p0;
        let s = t * self.s;
        match self.general {
            None => [ux0 + t * self.d[0], uy0 + t * self.d[1], w0 * (self.rho * s).exp()],
            Some((d1, r0)) => {
                let rho2 = self.rho * self.rho;
                let c = cosh(r0);
                let u = w0 / (rho2 * d1) * (c * tanh(self.rho * s + r0) - sinh(r0));
                [ux0 + u * self.d[0], uy0 + u * self.d[1], w0 * c / cosh(self.rho * s + r0)]
            }
        }
    }
}

/// A fly-to move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fly {
    /// Start time (seconds on the map's clock).
    pub begin: f64,
    /// Length (seconds); `None` takes d3's recommended duration for the distance.
    pub duration: Option<f64>,
    /// Target centre and zoom.
    pub lon: f64,
    pub lat: f64,
    pub zoom: f64,
    /// Curvature: how far out the move zooms (√2 by default).
    pub rho: f64,
}

impl Map {
    /// The zoom path of a move between views (longitude taking the short way round).
    pub fn path(&self, from: &View, to: &Fly) -> ZoomPath {
        let mut lon = to.lon;
        while lon - from.lon > 180.0 {
            lon -= 360.0;
        }
        while lon - from.lon < -180.0 {
            lon += 360.0;
        }
        ZoomPath::new([from.lon, from.lat, self.view_width(from.zoom)], [lon, to.lat, self.view_width(to.zoom)], to.rho)
    }

    /// The view at time `t`: `base` until the first move begins; then each move from where the
    /// previous one ended (the first from `rest`, the map's unanimated view), holding its target.
    /// `ease` maps a move's linear progress to its eased progress.
    pub fn view_at(&self, t: f64, base: View, rest: View, flies: &[Fly], ease: &dyn Fn(f64) -> f64) -> View {
        let mut flies: Vec<&Fly> = flies.iter().collect();
        flies.sort_by(|a, b| a.begin.total_cmp(&b.begin));
        if flies.first().is_none_or(|f| t < f.begin) {
            return base;
        }
        let mut cur = rest;
        for f in flies {
            if t < f.begin {
                break;
            }
            let path = self.path(&cur, f);
            let dur = f.duration.unwrap_or_else(|| path.duration()).max(0.0);
            let u = if dur <= 0.0 { 1.0 } else { ((t - f.begin) / dur).clamp(0.0, 1.0) };
            if u < 1.0 {
                let [lon, lat, w] = path.at(ease(u));
                return View { lon: normalize_lon(lon), lat, zoom: self.zoom_for_width(w), rotation: base.rotation };
            }
            cur = View { lon: f.lon, lat: f.lat, zoom: f.zoom, rotation: base.rotation };
        }
        cur
    }
}

fn normalize_lon(l: f64) -> f64 {
    let l = (l + 180.0).rem_euclid(360.0) - 180.0;
    if l == -180.0 {
        180.0
    } else {
        l
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_zero_fits_the_sphere() {
        let (m, c) = Map::new(Kind::EqualEarth, None, [960.0, 500.0], &[], 10.0, None);
        assert_eq!(c, [0.0, 0.0]);
        let b = m
            .projection(&View { lon: 0.0, lat: 0.0, zoom: 0.0, rotation: 0.0 })
            .project(&Geometry::Sphere)
            .bounds()
            .unwrap();
        // Equal Earth is about 2.06 times wider than tall: width-limited.
        assert!((b[0][0] - 10.0).abs() < 1e-6 && (b[1][0] - 950.0).abs() < 1e-6, "{b:?}");
    }

    #[test]
    fn views_follow_the_moves() {
        let (m, _) = Map::new(Kind::Mercator, None, [800.0, 600.0], &[], 0.0, None);
        let rest = View { lon: 0.0, lat: 0.0, zoom: 0.0, rotation: 0.0 };
        let fly = Fly { begin: 1.0, duration: Some(2.0), lon: 170.0, lat: 10.0, zoom: 3.0, rho: SQRT_2 };
        let lin = |u: f64| u;
        assert_eq!(m.view_at(0.5, rest, rest, &[fly], &lin), rest);
        let end = m.view_at(3.5, rest, rest, &[fly], &lin);
        assert!((end.lon - 170.0).abs() < 1e-9 && (end.zoom - 3.0).abs() < 1e-9);
        // Between two close-up views far apart, the optimal path zooms out on the way.
        let near = View { zoom: 3.0, ..rest };
        let mid = m.view_at(2.0, near, near, &[fly], &lin);
        assert!(mid.zoom < 2.0, "{mid:?}");
        // Crossing the antimeridian takes the short way.
        let west = Fly { lon: -170.0, ..fly };
        let from = View { lon: 170.0, ..rest };
        let v = m.view_at(1.2, from, from, &[west], &lin);
        assert!(v.lon > 170.0 || v.lon < -170.0, "{v:?}");
    }
}
