//! Golden-frame comparison. A frame matches its golden when PSNR ≥ 50 dB
//! and the largest CIEDE2000 difference is ≤ 1, both measured on
//! display-referred sRGB.

/// Comparison of two images of equal size (straight RGBA, display sRGB in [0, 1]).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Comparison {
    /// Peak signal-to-noise ratio over RGBA, dB (infinite when identical).
    pub psnr: f64,
    /// Largest per-pixel ΔE2000.
    pub max_delta_e: f64,
    /// Mean ΔE2000.
    pub mean_delta_e: f64,
}

impl Comparison {
    /// The acceptance rule for golden frames.
    pub fn passes(&self) -> bool {
        self.psnr >= 50.0 && self.max_delta_e <= 1.0
    }
}

fn lin(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Display sRGB → CIE L*a*b* (D65).
pub fn srgb_to_lab(c: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = c.map(lin);
    let x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = (0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883;
    let f = |t: f64| if t > 216.0 / 24389.0 { t.cbrt() } else { (24389.0 / 27.0 * t + 16.0) / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 colour difference (Sharma, Wu and Dalal 2005).
pub fn delta_e2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let deg = |r: f64| r * 180.0 / PI;
    let rad = |d: f64| d * PI / 180.0;
    let c1 = (a[1] * a[1] + a[2] * a[2]).sqrt();
    let c2 = (b[1] * b[1] + b[2] * b[2]).sqrt();
    let cm = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let (a1, a2) = (a[1] * (1.0 + g), b[1] * (1.0 + g));
    let (c1p, c2p) = ((a1 * a1 + a[2] * a[2]).sqrt(), (a2 * a2 + b[2] * b[2]).sqrt());
    let hue = |x: f64, y: f64| {
        if x == 0.0 && y == 0.0 {
            0.0
        } else {
            let h = deg(y.atan2(x));
            if h < 0.0 {
                h + 360.0
            } else {
                h
            }
        }
    };
    let (h1, h2) = (hue(a1, a[2]), hue(a2, b[2]));
    let dl = b[0] - a[0];
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2 - h1).abs() <= 180.0 {
        h2 - h1
    } else if h2 - h1 > 180.0 {
        h2 - h1 - 360.0
    } else {
        h2 - h1 + 360.0
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * rad(dh / 2.0).sin();
    let lm = (a[0] + b[0]) / 2.0;
    let cmp = (c1p + c2p) / 2.0;
    let hm = if c1p * c2p == 0.0 {
        h1 + h2
    } else if (h1 - h2).abs() <= 180.0 {
        (h1 + h2) / 2.0
    } else if h1 + h2 < 360.0 {
        (h1 + h2 + 360.0) / 2.0
    } else {
        (h1 + h2 - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * rad(hm - 30.0).cos() + 0.24 * rad(2.0 * hm).cos() + 0.32 * rad(3.0 * hm + 6.0).cos()
        - 0.20 * rad(4.0 * hm - 63.0).cos();
    let dtheta = 30.0 * (-((hm - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cmp.powi(7) / (cmp.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lm - 50.0).powi(2) / (20.0 + (lm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cmp;
    let sh = 1.0 + 0.015 * cmp * t;
    let rt = -rad(2.0 * dtheta).sin() * rc;
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhh / sh).powi(2) + rt * (dc / sc) * (dhh / sh)).sqrt()
}

/// Compares two images of straight display-sRGB RGBA values in [0, 1].
pub fn compare(a: &[[f64; 4]], b: &[[f64; 4]]) -> Comparison {
    assert_eq!(a.len(), b.len(), "images differ in size");
    let mut se = 0.0;
    let (mut max_de, mut sum_de) = (0.0f64, 0.0);
    for (p, q) in a.iter().zip(b) {
        for k in 0..4 {
            se += (p[k] - q[k]).powi(2);
        }
        // compare colours composited over mid grey so alpha differences count
        let over = |c: [f64; 4]| [0, 1, 2].map(|k| c[k] * c[3] + 0.5 * (1.0 - c[3]));
        let de = delta_e2000(srgb_to_lab(over(*p)), srgb_to_lab(over(*q)));
        max_de = max_de.max(de);
        sum_de += de;
    }
    let mse = se / (a.len().max(1) * 4) as f64;
    Comparison {
        psnr: if mse == 0.0 { f64::INFINITY } else { 10.0 * (1.0 / mse).log10() },
        max_delta_e: max_de,
        mean_delta_e: sum_de / a.len().max(1) as f64,
    }
}

/// 8-bit RGBA bytes → normalised values.
pub fn from_rgba8(px: &[u8]) -> Vec<[f64; 4]> {
    px.as_chunks::<4>().0.iter().map(|c| c.map(|v| v as f64 / 255.0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ciede2000_matches_the_published_test_data() {
        // Sharma, Wu and Dalal (2005), table 1, pairs 1, 7 and 19
        let cases = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.5, 0.0], [50.0, 0.0, -2.5], 4.3065),
        ];
        for (a, b, want) in cases {
            let got = delta_e2000(a, b);
            assert!((got - want).abs() < 1e-4, "{a:?} {b:?}: {got} vs {want}");
        }
    }

    #[test]
    fn psnr_and_acceptance() {
        let a = vec![[0.5, 0.5, 0.5, 1.0]; 100];
        assert!(compare(&a, &a).passes());
        let b = vec![[0.5 + 1.0 / 255.0, 0.5, 0.5, 1.0]; 100];
        let c = compare(&a, &b);
        assert!(c.psnr > 50.0 && c.max_delta_e < 1.0 && c.passes(), "{c:?}");
        let d = vec![[0.6, 0.5, 0.5, 1.0]; 100];
        assert!(!compare(&a, &d).passes());
    }
}
