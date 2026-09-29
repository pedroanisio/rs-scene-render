//! Colour spaces and transfer functions.
//!
//! Every conversion goes through CIE XYZ with a D65 white; spaces with
//! another white point (DCI-P3, the ACES spaces) are adapted with the
//! Bradford transform. Images decode on the CPU into the working space, the
//! compositor works in that space (linear unless `linearLight="false"`),
//! and output encoding happens on readback.

use sr_model::model::{ColorSpace, Transfer};

/// A 3×3 matrix, row major.
pub type M3 = [[f64; 3]; 3];

/// Matrix product.
pub fn mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

/// Matrix inverse.
pub fn inv(m: &M3) -> M3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let k = 1.0 / det;
    [
        [(e * i - f * h) * k, (c * h - b * i) * k, (b * f - c * e) * k],
        [(f * g - d * i) * k, (a * i - c * g) * k, (c * d - a * f) * k],
        [(d * h - e * g) * k, (b * g - a * h) * k, (a * e - b * d) * k],
    ]
}

/// Matrix times vector.
pub fn apply(m: &M3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

const D65: [f64; 2] = [0.3127, 0.3290];
const DCI_WHITE: [f64; 2] = [0.314, 0.351];
const ACES_WHITE: [f64; 2] = [0.32168, 0.33767];

fn xyz(xy: [f64; 2]) -> [f64; 3] {
    [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]]
}

/// RGB → XYZ for primaries and white point (SMPTE RP 177).
fn rgb_to_xyz(r: [f64; 2], g: [f64; 2], b: [f64; 2], w: [f64; 2]) -> M3 {
    let (xr, xg, xb) = (xyz(r), xyz(g), xyz(b));
    let p = [[xr[0], xg[0], xb[0]], [xr[1], xg[1], xb[1]], [xr[2], xg[2], xb[2]]];
    let s = apply(&inv(&p), xyz(w));
    [0, 1, 2].map(|i| [p[i][0] * s[0], p[i][1] * s[1], p[i][2] * s[2]])
}

/// Bradford adaptation from white `src` to white `dst`.
fn bradford(src: [f64; 2], dst: [f64; 2]) -> M3 {
    const B: M3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
    let (s, d) = (apply(&B, xyz(src)), apply(&B, xyz(dst)));
    let scale = [[d[0] / s[0], 0.0, 0.0], [0.0, d[1] / s[1], 0.0], [0.0, 0.0, d[2] / s[2]]];
    mul(&inv(&B), &mul(&scale, &B))
}

/// Bradford adaptation from the ICC connection white (D50 = 0.9642, 1, 0.8249) to D65.
pub fn icc_d50_to_d65() -> M3 {
    let d50 = [0.9642, 1.0, 0.8249];
    let s = d50[0] + d50[1] + d50[2];
    bradford([d50[0] / s, d50[1] / s], D65)
}

/// Linear RGB of `space` → XYZ (D65).
pub fn to_xyz_d65(space: ColorSpace) -> M3 {
    let srgb = ([0.64, 0.33], [0.30, 0.60], [0.15, 0.06]);
    let p3 = ([0.680, 0.320], [0.265, 0.690], [0.150, 0.060]);
    let r2020 = ([0.708, 0.292], [0.170, 0.797], [0.131, 0.046]);
    let ap1 = ([0.713, 0.293], [0.165, 0.830], [0.128, 0.044]);
    let ap0 = ([0.7347, 0.2653], [0.0, 1.0], [0.0001, -0.0770]);
    let (prim, white) = match space {
        ColorSpace::Srgb | ColorSpace::LinearSrgb | ColorSpace::Rec709 => (srgb, D65),
        ColorSpace::DisplayP3 => (p3, D65),
        ColorSpace::DciP3 => (p3, DCI_WHITE),
        ColorSpace::Rec2020 => (r2020, D65),
        ColorSpace::Acescg | ColorSpace::Acescct => (ap1, ACES_WHITE),
        ColorSpace::Aces2065_1 => (ap0, ACES_WHITE),
        ColorSpace::XyzD65 | ColorSpace::Raw => return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };
    let m = rgb_to_xyz(prim.0, prim.1, prim.2, white);
    if white == D65 {
        m
    } else {
        mul(&bradford(white, D65), &m)
    }
}

/// Linear RGB conversion matrix from `src` to `dst`.
pub fn convert(src: ColorSpace, dst: ColorSpace) -> M3 {
    if src == dst || src == ColorSpace::Raw || dst == ColorSpace::Raw {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    mul(&inv(&to_xyz_d65(dst)), &to_xyz_d65(src))
}

/// The transfer `auto` means for a colour space.
pub fn default_transfer(space: ColorSpace) -> Transfer {
    match space {
        ColorSpace::Srgb | ColorSpace::DisplayP3 => Transfer::Srgb,
        ColorSpace::Rec709 | ColorSpace::Rec2020 => Transfer::Bt1886,
        ColorSpace::DciP3 => Transfer::Gamma26,
        ColorSpace::Acescct => Transfer::Acescct,
        ColorSpace::LinearSrgb | ColorSpace::Acescg | ColorSpace::Aces2065_1 | ColorSpace::XyzD65 | ColorSpace::Raw => {
            Transfer::Linear
        }
    }
}

/// Resolves `auto` against the colour space.
pub fn resolve(space: ColorSpace, t: Transfer) -> Transfer {
    if t == Transfer::Auto {
        default_transfer(space)
    } else {
        t
    }
}

/// PQ luminance of HDR reference white (ITU-R BT.2408), in cd/m².
pub const PQ_REFERENCE_WHITE: f64 = 203.0;

/// HLG scene light of reference white (75 % signal, ITU-R BT.2408).
pub fn hlg_reference() -> f64 {
    let (a, b, c) = (0.17883277, 0.28466892, 0.55991073);
    (libm::exp((0.75 - c) / a) + b) / 12.0
}

/// Encoded value → linear. Scene-linear results put diffuse white at 1.0:
/// PQ divides by the 203 cd/m² reference white, HLG by the scene light of
/// its 75 % reference level, and the camera log curves decode to the
/// scene reflectance their makers define (18 % grey = 0.18).
pub fn decode(t: Transfer, v: f64) -> f64 {
    let p10 = |x: f64| libm::pow(10.0, x);
    match t {
        Transfer::Auto | Transfer::Linear => v,
        Transfer::Srgb => {
            if v <= 0.04045 {
                v / 12.92
            } else {
                libm::pow((v + 0.055) / 1.055, 2.4)
            }
        }
        Transfer::Bt1886 => libm::pow(v.max(0.0), 2.4),
        Transfer::Gamma22 => libm::pow(v.max(0.0), 2.2),
        Transfer::Gamma26 => libm::pow(v.max(0.0), 2.6),
        Transfer::Pq => {
            let (m1, m2, c1, c2, c3) = (0.1593017578125, 78.84375, 0.8359375, 18.8515625, 18.6875);
            let p = libm::pow(v.max(0.0), 1.0 / m2);
            libm::pow(((p - c1).max(0.0)) / (c2 - c3 * p), 1.0 / m1) * 10000.0 / PQ_REFERENCE_WHITE
        }
        Transfer::Hlg => {
            let (a, b, c) = (0.17883277, 0.28466892, 0.55991073);
            let e = if v <= 0.5 { v * v / 3.0 } else { (libm::exp((v - c) / a) + b) / 12.0 };
            e / hlg_reference()
        }
        Transfer::Acescct => {
            if v <= 0.155_251_141_552_511 {
                (v - 0.072_905_534_195_835_5) / 10.540_237_741_654_5
            } else {
                libm::pow(2.0, v * 17.52 - 9.72)
            }
        }
        Transfer::Acescc => {
            if v < (9.72 - 15.0) / 17.52 {
                (libm::pow(2.0, v * 17.52 - 9.72) - libm::pow(2.0, -16.0)) * 2.0
            } else {
                libm::pow(2.0, v * 17.52 - 9.72)
            }
        }
        // Sony S-Log3
        Transfer::Slog3 => {
            if v >= 171.210_294_692_9 / 1023.0 {
                p10((v * 1023.0 - 420.0) / 261.5) * (0.18 + 0.01) - 0.01
            } else {
                (v * 1023.0 - 95.0) * 0.01125 / (171.210_294_692_9 - 95.0)
            }
        }
        // ARRI LogC3, EI 800
        Transfer::Logc3 => {
            let (cut, a, b, c, d, e, f) = (0.010591, 5.555556, 0.052272, 0.247190, 0.385537, 5.367655, 0.092809);
            if v > e * cut + f {
                (p10((v - d) / c) - b) / a
            } else {
                (v - f) / e
            }
        }
        // ARRI LogC4
        Transfer::Logc4 => {
            let (a, b, c, s, t) = logc4_constants();
            if v >= 0.0 {
                (libm::pow(2.0, 14.0 * (v - c) / b + 6.0) - 64.0) / a
            } else {
                v * s + t
            }
        }
        // Panasonic V-Log
        Transfer::Vlog => {
            if v < 0.181 {
                (v - 0.125) / 5.6
            } else {
                p10((v - 0.598206) / 0.241514) - 0.00873
            }
        }
        // Canon Log 3, reflectance
        Transfer::Clog3 => {
            let x = if v < 0.097465473 {
                -(p10((0.12783901 - v) / 0.36726845) - 1.0) / 14.98325
            } else if v <= 0.15277891 {
                (v - 0.12512219) / 1.9754798
            } else {
                (p10((v - 0.12240537) / 0.36726845) - 1.0) / 14.98325
            };
            x * 0.9
        }
        // RED Log3G10
        Transfer::Redlog3g10 => {
            let (a, b, c, g) = (0.224282, 155.975327, 0.01, 15.1927);
            if v < 0.0 {
                v / g - c
            } else {
                (p10(v / a) - 1.0) / b - c
            }
        }
        // Fujifilm F-Log2
        Transfer::Flog2 => {
            let (a, b, c, d, e, f) = (5.555556, 0.064829, 0.245281, 0.384316, 8.799461, 0.092864);
            if v >= 0.100_686_685_370_811 {
                (p10((v - d) / c) - b) / a
            } else {
                (v - f) / e
            }
        }
        // Nikon N-Log
        Transfer::Nlog => {
            if v < 452.0 / 1023.0 {
                libm::pow(v * 1023.0 / 650.0, 3.0) - 0.0075
            } else {
                libm::exp((v * 1023.0 - 619.0) / 150.0)
            }
        }
    }
}

fn logc4_constants() -> (f64, f64, f64, f64, f64) {
    let a = (libm::pow(2.0, 18.0) - 16.0) / 117.45;
    let b = (1023.0 - 95.0) / 1023.0;
    let c = 95.0 / 1023.0;
    let s = (7.0 * std::f64::consts::LN_2 * libm::pow(2.0, 7.0 - 14.0 * c / b)) / (a * b);
    let t = (libm::pow(2.0, 14.0 * (-c / b) + 6.0) - 64.0) / a;
    (a, b, c, s, t)
}

/// Linear → encoded value (the inverse of [`decode`]).
pub fn encode(t: Transfer, v: f64) -> f64 {
    let l10 = libm::log10;
    match t {
        Transfer::Auto | Transfer::Linear => v,
        Transfer::Srgb => {
            if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * libm::pow(v, 1.0 / 2.4) - 0.055
            }
        }
        Transfer::Bt1886 => libm::pow(v.max(0.0), 1.0 / 2.4),
        Transfer::Gamma22 => libm::pow(v.max(0.0), 1.0 / 2.2),
        Transfer::Gamma26 => libm::pow(v.max(0.0), 1.0 / 2.6),
        Transfer::Acescct => {
            if v <= 0.0078125 {
                10.540_237_741_654_5 * v + 0.072_905_534_195_835_5
            } else {
                (libm::log2(v) + 9.72) / 17.52
            }
        }
        Transfer::Acescc => {
            if v <= 0.0 {
                (-16.0 + 9.72) / 17.52
            } else if v < libm::pow(2.0, -15.0) {
                (libm::log2(libm::pow(2.0, -16.0) + v * 0.5) + 9.72) / 17.52
            } else {
                (libm::log2(v) + 9.72) / 17.52
            }
        }
        Transfer::Pq => {
            let (m1, m2, c1, c2, c3) = (0.1593017578125, 78.84375, 0.8359375, 18.8515625, 18.6875);
            let y = libm::pow((v * PQ_REFERENCE_WHITE / 10000.0).max(0.0), m1);
            libm::pow((c1 + c2 * y) / (1.0 + c3 * y), m2)
        }
        Transfer::Hlg => {
            let (a, b, c) = (0.17883277, 0.28466892, 0.55991073);
            let e = (v * hlg_reference()).max(0.0);
            if e <= 1.0 / 12.0 {
                libm::sqrt(3.0 * e)
            } else {
                a * libm::log(12.0 * e - b) + c
            }
        }
        Transfer::Slog3 => {
            if v >= 0.011_250_00 {
                (420.0 + l10((v + 0.01) / (0.18 + 0.01)) * 261.5) / 1023.0
            } else {
                (v * (171.210_294_692_9 - 95.0) / 0.01125 + 95.0) / 1023.0
            }
        }
        Transfer::Logc3 => {
            let (cut, a, b, c, d, e, f) = (0.010591, 5.555556, 0.052272, 0.247190, 0.385537, 5.367655, 0.092809);
            if v > cut {
                c * l10(a * v + b) + d
            } else {
                e * v + f
            }
        }
        Transfer::Logc4 => {
            let (a, b, c, s, t) = logc4_constants();
            if v >= t {
                (libm::log2(a * v + 64.0) - 6.0) / 14.0 * b + c
            } else {
                (v - t) / s
            }
        }
        Transfer::Vlog => {
            if v < 0.01 {
                5.6 * v + 0.125
            } else {
                0.241514 * l10(v + 0.00873) + 0.598206
            }
        }
        Transfer::Clog3 => {
            let x = v / 0.9;
            if x < -0.014 {
                -0.36726845 * l10(1.0 - 14.98325 * x) + 0.12783901
            } else if x <= 0.014 {
                1.9754798 * x + 0.12512219
            } else {
                0.36726845 * l10(14.98325 * x + 1.0) + 0.12240537
            }
        }
        Transfer::Redlog3g10 => {
            let (a, b, c, g) = (0.224282, 155.975327, 0.01, 15.1927);
            let x = v + c;
            if x < 0.0 {
                x * g
            } else {
                a * l10(x * b + 1.0)
            }
        }
        Transfer::Flog2 => {
            let (a, b, c, d, e, f) = (5.555556, 0.064829, 0.245281, 0.384316, 8.799461, 0.092864);
            if v >= 0.000889 {
                c * l10(a * v + b) + d
            } else {
                e * v + f
            }
        }
        Transfer::Nlog => {
            if v < 0.328 {
                650.0 * libm::cbrt(v + 0.0075) / 1023.0
            } else {
                (150.0 * libm::log(v) + 619.0) / 1023.0
            }
        }
    }
}

/// Numeric id of a transfer, shared with the shaders.
pub fn transfer_id(t: Transfer) -> u32 {
    Transfer::ALL.iter().position(|x| *x == t).unwrap_or(0) as u32
}

/// The working space of a document: colour space, and whether compositing
/// runs on linear values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Working {
    /// Working colour space.
    pub space: ColorSpace,
    /// Blend on linear light (otherwise on encoded values).
    pub linear: bool,
}

impl Working {
    /// Working space of a scene: `colorManagement/@workingSpace` when the
    /// section exists, else `project/@workingColorSpace`.
    pub fn of(scene: &sr_model::model::Scene) -> Working {
        let space =
            scene.color_management.as_ref().map(|c| c.working_space).unwrap_or(scene.project.working_color_space);
        Working { space, linear: scene.project.linear_light }
    }

    /// Converts a straight, linear colour in the working space from linear sRGB.
    pub fn from_linear_srgb(&self, c: [f64; 4]) -> [f64; 4] {
        let m = convert(ColorSpace::LinearSrgb, self.space);
        let v = apply(&m, [c[0], c[1], c[2]]);
        self.store([v[0], v[1], v[2], c[3]])
    }

    /// A straight, linear working-space colour in the representation the
    /// compositor uses (encoded when `linear` is false).
    pub fn store(&self, c: [f64; 4]) -> [f64; 4] {
        if self.linear {
            c
        } else {
            let t = default_transfer(self.space);
            let t = if t == Transfer::Linear { Transfer::Srgb } else { t };
            [encode(t, c[0]), encode(t, c[1]), encode(t, c[2]), c[3]]
        }
    }

    /// Stored working value → linear working value.
    pub fn to_linear(&self, c: [f64; 3]) -> [f64; 3] {
        if self.linear {
            c
        } else {
            let t = default_transfer(self.space);
            let t = if t == Transfer::Linear { Transfer::Srgb } else { t };
            c.map(|v| decode(t, v))
        }
    }

    /// Converts a stored working value to 8-bit-ready display sRGB (encoded, straight).
    pub fn to_display_srgb(&self, c: [f64; 3]) -> [f64; 3] {
        let lin = self.to_linear(c);
        let m = convert(self.space, ColorSpace::LinearSrgb);
        apply(&m, lin).map(|v| encode(Transfer::Srgb, v.clamp(0.0, 1.0)))
    }

    /// A document colour literal into the stored working representation.
    /// Literals are display sRGB whatever the working space (the working space does not change how they are read): decoded with the sRGB transfer, converted from sRGB
    /// primaries to the working space, then stored. `#808080` therefore
    /// displays as 128 in `srgb` and `linear-srgb` scenes alike.
    pub fn from_literal(&self, c: [f64; 4]) -> [f64; 4] {
        let d = |v: f64| decode(Transfer::Srgb, v);
        self.from_linear_srgb([d(c[0]), d(c[1]), d(c[2]), c[3]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_round_trip_and_match_references() {
        let m = to_xyz_d65(ColorSpace::Srgb);
        // IEC 61966-2-1 row for Y
        assert!((m[1][0] - 0.2126).abs() < 1e-3 && (m[1][1] - 0.7152).abs() < 1e-3);
        let c = convert(ColorSpace::LinearSrgb, ColorSpace::Acescg);
        // linear sRGB → ACEScg (Bradford), ACES reference
        assert!((c[0][0] - 0.6131).abs() < 2e-3 && (c[0][1] - 0.3395).abs() < 2e-3, "{c:?}");
        let back = mul(&convert(ColorSpace::Acescg, ColorSpace::LinearSrgb), &c);
        for (i, row) in back.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                assert!((v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-9);
            }
        }
        // white stays white
        let w = apply(&convert(ColorSpace::DisplayP3, ColorSpace::Rec2020), [1.0, 1.0, 1.0]);
        assert!(w.iter().all(|v| (v - 1.0).abs() < 1e-9));
    }

    #[test]
    fn transfers_invert() {
        for t in [
            Transfer::Srgb,
            Transfer::Bt1886,
            Transfer::Gamma22,
            Transfer::Gamma26,
            Transfer::Pq,
            Transfer::Hlg,
            Transfer::Acescct,
        ] {
            for v in [0.0, 0.01, 0.18, 0.5, 1.0] {
                let r = decode(t, encode(t, v));
                assert!((r - v).abs() < 1e-6, "{t:?} {v} → {r}");
            }
        }
        assert!((decode(Transfer::Srgb, 0.5) - 0.214).abs() < 1e-3);
    }
}
