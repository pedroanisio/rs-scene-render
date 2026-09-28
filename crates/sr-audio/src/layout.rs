//! Channel layouts, panning, up/downmixing and ambisonic encoding.

use std::f64::consts::PI;

/// Output channel layouts of `audioMix/@channelLayout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum Layout {
    /// 1 channel.
    Mono,
    /// L, R.
    Stereo,
    /// L, R, C, LFE, Ls, Rs (WAV/FFmpeg "5.1" order, surrounds at ±110°).
    Surround51,
    /// L, R, C, LFE, Lb, Rb, Ls, Rs.
    Surround71,
    /// 7.1 plus top front left/right and top back left/right.
    Surround714,
    /// First-order ambisonics, ACN order, SN3D normalisation (AmbiX).
    Ambisonic1,
    /// Third-order ambisonics, ACN/SN3D.
    Ambisonic3,
}

/// A loudspeaker: azimuth (degrees, positive to the left), elevation, LFE flag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speaker {
    /// Short name.
    pub name: &'static str,
    /// Azimuth in degrees, counter-clockwise from front.
    pub azimuth: f64,
    /// Elevation in degrees.
    pub elevation: f64,
    /// Low-frequency effects channel.
    pub lfe: bool,
}

const fn sp(name: &'static str, azimuth: f64, elevation: f64) -> Speaker {
    Speaker { name, azimuth, elevation, lfe: false }
}

const LFE: Speaker = Speaker { name: "LFE", azimuth: 0.0, elevation: 0.0, lfe: true };

impl Layout {
    /// Parses `channelLayout`; `auto` picks by channel count.
    pub fn parse(name: &str, channels: u32) -> Layout {
        match name {
            "mono" => Layout::Mono,
            "stereo" => Layout::Stereo,
            "5.1" => Layout::Surround51,
            "7.1" => Layout::Surround71,
            "7.1.4" => Layout::Surround714,
            "ambisonic-1" => Layout::Ambisonic1,
            "ambisonic-3" => Layout::Ambisonic3,
            _ => Layout::for_channels(channels, ""),
        }
    }

    /// Layout of a source with `channels` channels and an FFmpeg layout name.
    pub fn for_channels(channels: u32, name: &str) -> Layout {
        if name.contains("ambisonic") {
            return if channels >= 16 { Layout::Ambisonic3 } else { Layout::Ambisonic1 };
        }
        match channels {
            1 => Layout::Mono,
            6 => Layout::Surround51,
            8 => Layout::Surround71,
            12 => Layout::Surround714,
            16 => Layout::Ambisonic3,
            _ => Layout::Stereo,
        }
    }

    /// Channel count.
    pub fn channels(&self) -> usize {
        match self {
            Layout::Mono => 1,
            Layout::Stereo => 2,
            Layout::Surround51 => 6,
            Layout::Surround71 => 8,
            Layout::Surround714 => 12,
            Layout::Ambisonic1 => 4,
            Layout::Ambisonic3 => 16,
        }
    }

    /// Ambisonic order, if the layout is ambisonic.
    pub fn ambisonic_order(&self) -> Option<usize> {
        match self {
            Layout::Ambisonic1 => Some(1),
            Layout::Ambisonic3 => Some(3),
            _ => None,
        }
    }

    /// Loudspeakers in channel order (empty for ambisonics).
    pub fn speakers(&self) -> Vec<Speaker> {
        match self {
            Layout::Mono => vec![sp("C", 0.0, 0.0)],
            Layout::Stereo => vec![sp("L", 30.0, 0.0), sp("R", -30.0, 0.0)],
            Layout::Surround51 => vec![
                sp("L", 30.0, 0.0),
                sp("R", -30.0, 0.0),
                sp("C", 0.0, 0.0),
                LFE,
                sp("Ls", 110.0, 0.0),
                sp("Rs", -110.0, 0.0),
            ],
            Layout::Surround71 | Layout::Surround714 => {
                let mut v = vec![
                    sp("L", 30.0, 0.0),
                    sp("R", -30.0, 0.0),
                    sp("C", 0.0, 0.0),
                    LFE,
                    sp("Lb", 150.0, 0.0),
                    sp("Rb", -150.0, 0.0),
                    sp("Ls", 90.0, 0.0),
                    sp("Rs", -90.0, 0.0),
                ];
                if *self == Layout::Surround714 {
                    v.extend([
                        sp("Ltf", 45.0, 45.0),
                        sp("Rtf", -45.0, 45.0),
                        sp("Ltb", 135.0, 45.0),
                        sp("Rtb", -135.0, 45.0),
                    ]);
                }
                v
            }
            Layout::Ambisonic1 | Layout::Ambisonic3 => Vec::new(),
        }
    }

    /// WAVE_FORMAT_EXTENSIBLE channel mask (0 for ambisonics).
    pub fn wav_mask(&self) -> u32 {
        match self {
            Layout::Mono => 0x4,
            Layout::Stereo => 0x3,
            Layout::Surround51 => 0x3F,
            Layout::Surround71 => 0x63F,
            Layout::Surround714 => 0x63F | 0x1000 | 0x4000 | 0x8000 | 0x20000,
            _ => 0,
        }
    }

    /// FFmpeg channel layout name.
    pub fn ffmpeg_name(&self) -> String {
        match self {
            Layout::Mono => "mono".into(),
            Layout::Stereo => "stereo".into(),
            Layout::Surround51 => "5.1".into(),
            Layout::Surround71 => "7.1".into(),
            Layout::Surround714 => "7.1.4".into(),
            Layout::Ambisonic1 => "ambisonic 1".into(),
            Layout::Ambisonic3 => "ambisonic 3".into(),
        }
    }

    /// BS.1770 channel weight: 1.41 for surrounds between 60° and 120°
    /// off-axis and behind, 0 for LFE, 1 otherwise.
    pub fn loudness_weights(&self) -> Vec<f64> {
        if let Some(_o) = self.ambisonic_order() {
            // only W carries the omnidirectional level
            let mut w = vec![0.0; self.channels()];
            w[0] = 1.0;
            return w;
        }
        self.speakers()
            .iter()
            .map(|s| {
                if s.lfe {
                    0.0
                } else if s.elevation.abs() < 30.0 && s.azimuth.abs() >= 60.0 {
                    1.41
                } else {
                    1.0
                }
            })
            .collect()
    }
}

fn factorial(n: usize) -> f64 {
    (1..=n).map(|k| k as f64).product()
}

/// Associated Legendre P_l^m(x) without the Condon–Shortley phase.
fn legendre(l: usize, m: usize, x: f64) -> f64 {
    let mut pmm = 1.0;
    let s = (1.0 - x * x).max(0.0).sqrt();
    for k in 0..m {
        pmm *= (2 * k + 1) as f64 * s;
    }
    if l == m {
        return pmm;
    }
    let mut pm1 = x * (2 * m + 1) as f64 * pmm;
    if l == m + 1 {
        return pm1;
    }
    let mut pll = 0.0;
    for ll in m + 2..=l {
        pll = (x * (2 * ll - 1) as f64 * pm1 - (ll + m - 1) as f64 * pmm) / (ll - m) as f64;
        pmm = pm1;
        pm1 = pll;
    }
    pll
}

/// Real spherical harmonics in ACN order with SN3D normalisation.
pub fn sn3d(order: usize, azimuth: f64, elevation: f64) -> Vec<f64> {
    let (az, el) = (azimuth.to_radians(), elevation.to_radians());
    let mut out = Vec::with_capacity((order + 1) * (order + 1));
    for l in 0..=order {
        for m in -(l as i64)..=(l as i64) {
            let am = m.unsigned_abs() as usize;
            let norm = ((if am == 0 { 1.0 } else { 2.0 }) * factorial(l - am) / factorial(l + am)).sqrt();
            let p = legendre(l, am, el.sin());
            let trig = if m > 0 {
                (am as f64 * az).cos()
            } else if m < 0 {
                (am as f64 * az).sin()
            } else {
                1.0
            };
            out.push(norm * p * trig);
        }
    }
    out
}

/// Gains that place a point source at `azimuth` (degrees, + left) in
/// `layout`: pairwise constant-power panning between the two nearest
/// horizontal speakers (LFE excluded), or spherical-harmonic encoding.
pub fn point_gains(layout: Layout, azimuth: f64, elevation: f64) -> Vec<f64> {
    if let Some(o) = layout.ambisonic_order() {
        return sn3d(o, azimuth, elevation);
    }
    let sp = layout.speakers();
    let n = sp.len();
    if n == 1 {
        return vec![1.0];
    }
    let mut g = vec![0.0; n];
    if n == 2 {
        // constant-power across the stereo pair, clamped to the speakers
        let x = (azimuth / 30.0).clamp(-1.0, 1.0);
        let th = (1.0 - x) * PI / 4.0;
        g[0] = th.cos();
        g[1] = th.sin();
        return g;
    }
    // horizontal speakers sorted by azimuth
    let mut ring: Vec<(f64, usize)> =
        sp.iter().enumerate().filter(|(_, s)| !s.lfe && s.elevation.abs() < 1.0).map(|(i, s)| (s.azimuth, i)).collect();
    ring.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let az = ((azimuth + 180.0).rem_euclid(360.0)) - 180.0;
    for k in 0..ring.len() {
        let (a0, i0) = ring[k];
        let (mut a1, i1) = ring[(k + 1) % ring.len()];
        if a1 <= a0 {
            a1 += 360.0;
        }
        let mut x = az;
        if x < a0 {
            x += 360.0;
        }
        if x >= a0 && x <= a1 {
            let f = (x - a0) / (a1 - a0);
            g[i0] = ((1.0 - f) * PI / 2.0).sin();
            g[i1] += (f * PI / 2.0).sin();
            return g;
        }
    }
    g
}

/// Routing matrix `[out][in]` from a source layout into the mix layout with
/// `pan` in [−1, 1]. Mono sources pan as a point across the front; channel
/// layouts map channel by channel, falling back to ITU-R BS.775 downmix
/// coefficients; stereo `pan` is a constant-power balance.
pub fn route(src: Layout, dst: Layout, pan: f64) -> Vec<Vec<f64>> {
    let (ni, no) = (src.channels(), dst.channels());
    let mut m = vec![vec![0.0; ni]; no];
    let pan = pan.clamp(-1.0, 1.0);
    let balance = |ch_left: bool| -> f64 {
        if ch_left {
            if pan > 0.0 {
                (pan * PI / 2.0).cos()
            } else {
                1.0
            }
        } else if pan < 0.0 {
            (-pan * PI / 2.0).cos()
        } else {
            1.0
        }
    };
    if src == dst && src.ambisonic_order().is_some() {
        for (i, row) in m.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        return m;
    }
    if src == Layout::Mono {
        // front arc: pan −1 → +30° (left), +1 → −30° (right); ±90° in ambisonics
        let az = if dst.ambisonic_order().is_some() { -pan * 90.0 } else { -pan * 30.0 };
        for (o, g) in point_gains(dst, az, 0.0).into_iter().enumerate() {
            m[o][0] = g;
        }
        return m;
    }
    if src.ambisonic_order().is_some() {
        // decode W (and first-order Y for left/right) to loudspeakers
        let sp = dst.speakers();
        for (o, s) in sp.iter().enumerate() {
            if s.lfe {
                continue;
            }
            m[o][0] = 1.0 / (sp.len() as f64).sqrt();
            if ni > 1 {
                m[o][1] = 0.5 * s.azimuth.to_radians().sin();
                m[o][3] = 0.5 * s.azimuth.to_radians().cos();
            }
        }
        return m;
    }
    let ss = src.speakers();
    let ds = dst.speakers();
    for (i, s) in ss.iter().enumerate() {
        let left = s.azimuth > 0.0;
        let bal = if s.azimuth.abs() < 1.0 { 1.0 } else { balance(left) };
        if s.lfe {
            if let Some(o) = ds.iter().position(|d| d.lfe) {
                m[o][i] = 1.0;
            }
            continue;
        }
        if let Some(o) = ds.iter().position(|d| d.name == s.name) {
            m[o][i] = bal;
            continue;
        }
        // downmix: fold into the nearest available position at −3 dB, or pan as a point
        let gains = if dst.ambisonic_order().is_some() {
            point_gains(dst, s.azimuth, s.elevation)
        } else {
            downmix_gains(s, &ds)
        };
        for (o, g) in gains.into_iter().enumerate() {
            m[o][i] = g * bal;
        }
    }
    m
}

fn downmix_gains(s: &Speaker, ds: &[Speaker]) -> Vec<f64> {
    let mut g = vec![0.0; ds.len()];
    let k = std::f64::consts::FRAC_1_SQRT_2;
    let find = |n: &str| ds.iter().position(|d| d.name == n);
    let side = |left: bool| -> Option<usize> {
        let names: &[&str] = if left { &["Ls", "Lb", "L"] } else { &["Rs", "Rb", "R"] };
        names.iter().find_map(|n| find(n))
    };
    match s.name {
        "C" => match (find("L"), find("R"), find("C")) {
            (_, _, Some(c)) => g[c] = 1.0,
            (Some(l), Some(r), None) => {
                g[l] = k;
                g[r] = k;
            }
            _ => g[0] = 1.0,
        },
        _ => {
            let left = s.azimuth > 0.0;
            if ds.len() == 1 {
                g[0] = k;
            } else if s.elevation > 1.0 {
                // heights fold into the bed below them
                let name = if left {
                    if s.azimuth.abs() < 90.0 {
                        "L"
                    } else {
                        "Lb"
                    }
                } else if s.azimuth.abs() < 90.0 {
                    "R"
                } else {
                    "Rb"
                };
                if let Some(o) = find(name).or_else(|| side(left)) {
                    g[o] = k;
                }
            } else if let Some(o) = side(left) {
                g[o] = if ds[o].name.len() == 1 { k } else { 1.0 };
            }
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_panning_is_constant_power() {
        for pan in [-1.0, -0.5, 0.0, 0.3, 1.0] {
            let m = route(Layout::Mono, Layout::Stereo, pan);
            let p = m[0][0] * m[0][0] + m[1][0] * m[1][0];
            assert!((p - 1.0).abs() < 1e-12, "{pan}: {p}");
        }
        let m = route(Layout::Mono, Layout::Stereo, -1.0);
        assert!((m[0][0] - 1.0).abs() < 1e-12 && m[1][0].abs() < 1e-12);
    }

    #[test]
    fn surround_panning_and_downmix() {
        let m = route(Layout::Mono, Layout::Surround51, 0.0);
        assert!((m[2][0] - 1.0).abs() < 1e-12, "centre only: {m:?}");
        let d = route(Layout::Surround51, Layout::Stereo, 0.0);
        let k = std::f64::consts::FRAC_1_SQRT_2;
        assert_eq!(d[0], vec![1.0, 0.0, k, 0.0, k, 0.0]);
        assert_eq!(d[1], vec![0.0, 1.0, k, 0.0, 0.0, k]);
        let up = route(Layout::Stereo, Layout::Surround714, 0.0);
        assert_eq!((up[0][0], up[1][1]), (1.0, 1.0));
        assert_eq!(Layout::Surround714.speakers().len(), 12);
    }

    #[test]
    fn ambisonic_encoding() {
        let front = sn3d(1, 0.0, 0.0);
        assert!(
            (front[0] - 1.0).abs() < 1e-12
                && front[1].abs() < 1e-12
                && front[2].abs() < 1e-12
                && (front[3] - 1.0).abs() < 1e-12
        );
        let left = sn3d(1, 90.0, 0.0);
        assert!((left[1] - 1.0).abs() < 1e-12 && left[3].abs() < 1e-12);
        let up = sn3d(1, 0.0, 90.0);
        assert!((up[2] - 1.0).abs() < 1e-12);
        let third = sn3d(3, 30.0, 10.0);
        assert_eq!(third.len(), 16);
        // SN3D: the sum over each order's squares at any direction is 1 (horizontal: exactly for l ≤ 1)
        let s1: f64 = third[1..4].iter().map(|v| v * v).sum();
        assert!((s1 - 1.0).abs() < 1e-9, "{s1}");
    }

    #[test]
    fn loudness_weights_follow_bs1770() {
        assert_eq!(Layout::Surround51.loudness_weights(), vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41]);
    }
}
