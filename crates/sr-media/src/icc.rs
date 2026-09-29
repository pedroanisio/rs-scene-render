//! ICC colour profiles: the matrix/TRC subset.
//!
//! Camera, phone and editing profiles (sRGB, Display P3, Adobe RGB, ProPhoto,
//! Rec. 2020 and grey) describe their colour with three colorants and three
//! tone curves, which is what this module reads: the colorants give linear RGB
//! → XYZ relative to the ICC connection white (D50), and the curves give the
//! file values → linear light. A profile carrying the ICC 4.4 `cicp` tag also
//! reports its coding-independent code points, so HDR (PQ, HLG) profiles map
//! onto the engine's transfers exactly.
//!
//! Lookup-table profiles (`A2B0` without colorants, as in CMYK and most
//! printer profiles) are reported as unsupported; the caller then uses the
//! colour space the document declares.

/// A tone curve: file value (0‥1) → linear light.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    /// y = x.
    Identity,
    /// y = x^γ.
    Gamma(f64),
    /// Samples at evenly spaced inputs, interpolated linearly.
    Table(Vec<f64>),
    /// ICC parametric curve: function type (0–4) and its parameters g a b c d e f.
    Parametric(u16, [f64; 7]),
}

impl Curve {
    /// Evaluates the curve.
    pub fn eval(&self, x: f64) -> f64 {
        match self {
            Curve::Identity => x,
            Curve::Gamma(g) => x.max(0.0).powf(*g),
            Curve::Table(t) => {
                let n = t.len();
                if n == 0 {
                    return x;
                }
                if n == 1 {
                    return t[0];
                }
                let p = x.clamp(0.0, 1.0) * (n - 1) as f64;
                let i = (p.floor() as usize).min(n - 2);
                let f = p - i as f64;
                t[i] + (t[i + 1] - t[i]) * f
            }
            Curve::Parametric(kind, [g, a, b, c, d, e, f]) => {
                let pw = |v: f64| if v > 0.0 { v.powf(*g) } else { 0.0 };
                match kind {
                    0 => pw(x),
                    1 => {
                        if x >= -b / a {
                            pw(a * x + b)
                        } else {
                            0.0
                        }
                    }
                    2 => {
                        if x >= -b / a {
                            pw(a * x + b) + c
                        } else {
                            *c
                        }
                    }
                    3 => {
                        if x >= *d {
                            pw(a * x + b)
                        } else {
                            c * x
                        }
                    }
                    _ => {
                        if x >= *d {
                            pw(a * x + b) + e
                        } else {
                            c * x + f
                        }
                    }
                }
            }
        }
    }
}

/// A matrix/TRC profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Linear RGB → XYZ relative to D50, columns = the red, green and blue colorants.
    /// A grey profile maps every channel to a third of the D50 white, so (Y, Y, Y) → Y·white.
    pub to_xyz_d50: [[f64; 3]; 3],
    /// Tone curves of red, green and blue (all three the grey curve for a grey profile).
    pub curves: [Curve; 3],
    /// Whether the profile describes a single grey channel.
    pub gray: bool,
    /// ICC 4.4 `cicp` tag: colour primaries and transfer characteristics (ITU-T H.273).
    pub cicp: Option<(u8, u8)>,
    /// The profile's description, when it has one.
    pub description: String,
}

/// The ICC connection white (D50) as XYZ.
pub const D50: [f64; 3] = [0.9642, 1.0, 0.8249];

fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

fn be16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn s15(b: &[u8], o: usize) -> Option<f64> {
    Some(be32(b, o)? as i32 as f64 / 65536.0)
}

/// Parses a profile. `Err` names why it cannot be used (not RGB or grey, lookup-table only, damaged).
pub fn parse(icc: &[u8]) -> Result<Profile, String> {
    if icc.len() < 132 || &icc[36..40] != b"acsp" {
        return Err("not an ICC profile".into());
    }
    let space = &icc[16..20];
    let gray = match space {
        b"RGB " => false,
        b"GRAY" => true,
        other => return Err(format!("{} profile", String::from_utf8_lossy(other).trim())),
    };
    let count = be32(icc, 128).ok_or("truncated tag table")? as usize;
    let mut tags = std::collections::HashMap::new();
    for i in 0..count.min(1024) {
        let o = 132 + i * 12;
        let (Some(sig), Some(off), Some(len)) = (icc.get(o..o + 4), be32(icc, o + 4), be32(icc, o + 8)) else {
            return Err("truncated tag table".into());
        };
        let (off, len) = (off as usize, len as usize);
        if off.checked_add(len).is_some_and(|e| e <= icc.len()) {
            tags.insert(<[u8; 4]>::try_from(sig).unwrap(), &icc[off..off + len]);
        }
    }
    let curve = |sig: &[u8; 4]| -> Result<Curve, String> {
        let t = tags.get(sig).ok_or_else(|| format!("no {} tag", String::from_utf8_lossy(sig)))?;
        read_curve(t).ok_or_else(|| format!("unreadable {} tag", String::from_utf8_lossy(sig)))
    };
    let xyz = |sig: &[u8; 4]| -> Result<[f64; 3], String> {
        let t = tags.get(sig).ok_or_else(|| "lookup-table profile (no colorants)".to_string())?;
        if t.get(0..4) != Some(b"XYZ ") {
            return Err(format!("unreadable {} tag", String::from_utf8_lossy(sig)));
        }
        Ok([s15(t, 8).ok_or("short XYZ")?, s15(t, 12).ok_or("short XYZ")?, s15(t, 16).ok_or("short XYZ")?])
    };
    let (to_xyz_d50, curves) = if gray {
        let k = curve(b"kTRC")?;
        let w = D50.map(|v| v / 3.0);
        ([[w[0]; 3], [w[1]; 3], [w[2]; 3]], [k.clone(), k.clone(), k])
    } else {
        let (r, g, b) = (xyz(b"rXYZ")?, xyz(b"gXYZ")?, xyz(b"bXYZ")?);
        (
            [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]],
            [curve(b"rTRC")?, curve(b"gTRC")?, curve(b"bTRC")?],
        )
    };
    let cicp = tags.get(b"cicp").filter(|t| t.len() >= 12 && &t[0..4] == b"cicp").map(|t| (t[8], t[9]));
    Ok(Profile {
        to_xyz_d50,
        curves,
        gray,
        cicp,
        description: tags.get(b"desc").map(|t| describe(t)).unwrap_or_default(),
    })
}

fn read_curve(t: &[u8]) -> Option<Curve> {
    match t.get(0..4)? {
        b"curv" => {
            let n = be32(t, 8)? as usize;
            match n {
                0 => Some(Curve::Identity),
                1 => Some(Curve::Gamma(be16(t, 12)? as f64 / 256.0)),
                _ => (0..n)
                    .map(|i| be16(t, 12 + 2 * i).map(|v| v as f64 / 65535.0))
                    .collect::<Option<_>>()
                    .map(Curve::Table),
            }
        }
        b"para" => {
            let kind = be16(t, 8)?;
            let n = [1, 3, 4, 5, 7].get(kind as usize)?;
            let mut p = [1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
            for (i, v) in p.iter_mut().enumerate().take(*n) {
                *v = s15(t, 12 + 4 * i)?;
            }
            Some(Curve::Parametric(kind, p))
        }
        _ => None,
    }
}

/// The text of a `desc` tag (v2 `desc` or v4 `mluc`, first record).
fn describe(t: &[u8]) -> String {
    match t.get(0..4) {
        Some(b"desc") => {
            let n = be32(t, 8).unwrap_or(0) as usize;
            t.get(12..12 + n).map(|s| String::from_utf8_lossy(s).trim_end_matches('\0').to_string()).unwrap_or_default()
        }
        Some(b"mluc") => {
            let (Some(len), Some(off)) = (be32(t, 20), be32(t, 24)) else { return String::new() };
            let (len, off) = (len as usize, off as usize);
            t.get(off..off + len)
                .map(|s| {
                    let u: Vec<u16> = s.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    String::from_utf16_lossy(&u)
                })
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a matrix/TRC profile: colorants (D50 XYZ) and curve tags.
    pub fn build(gray: bool, colorants: [[f64; 3]; 3], curve: &[u8], cicp: Option<(u8, u8)>) -> Vec<u8> {
        let fixed = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
        let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        if gray {
            tags.push((*b"kTRC", curve.to_vec()));
        } else {
            for (sig, c) in [(*b"rXYZ", colorants[0]), (*b"gXYZ", colorants[1]), (*b"bXYZ", colorants[2])] {
                let mut t = b"XYZ \0\0\0\0".to_vec();
                for v in c {
                    t.extend(fixed(v));
                }
                tags.push((sig, t));
            }
            for sig in [*b"rTRC", *b"gTRC", *b"bTRC"] {
                tags.push((sig, curve.to_vec()));
            }
        }
        if let Some((p, t)) = cicp {
            tags.push((*b"cicp", vec![b'c', b'i', b'c', b'p', 0, 0, 0, 0, p, t, 0, 1]));
        }
        let mut data = Vec::new();
        let mut table = Vec::new();
        let base = 132 + 12 * tags.len();
        for (sig, t) in &tags {
            table.extend(sig);
            table.extend(((base + data.len()) as u32).to_be_bytes());
            table.extend((t.len() as u32).to_be_bytes());
            data.extend(t);
            while data.len() % 4 != 0 {
                data.push(0);
            }
        }
        let mut h = vec![0u8; 128];
        h[8] = 4;
        h[12..16].copy_from_slice(b"mntr");
        h[16..20].copy_from_slice(if gray { b"GRAY" } else { b"RGB " });
        h[20..24].copy_from_slice(b"XYZ ");
        h[36..40].copy_from_slice(b"acsp");
        let mut out = h;
        out.extend((tags.len() as u32).to_be_bytes());
        out.extend(table);
        out.extend(data);
        let n = out.len() as u32;
        out[0..4].copy_from_slice(&n.to_be_bytes());
        out
    }

    /// The sRGB parametric curve tag.
    pub fn srgb_para() -> Vec<u8> {
        let mut t = b"para\0\0\0\0\0\x03\0\0".to_vec();
        for v in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
            t.extend(((v * 65536.0f64).round() as i32).to_be_bytes());
        }
        t
    }

    #[test]
    fn reads_colorants_curves_and_cicp() {
        let c = [[0.4361, 0.2225, 0.0139], [0.3851, 0.7169, 0.0971], [0.1431, 0.0606, 0.7141]];
        let p = parse(&build(false, c, &srgb_para(), Some((1, 13)))).unwrap();
        assert!((p.to_xyz_d50[0][0] - 0.4361).abs() < 1e-4 && (p.to_xyz_d50[1][1] - 0.7169).abs() < 1e-4);
        assert!((p.to_xyz_d50[2][2] - 0.7141).abs() < 1e-4);
        assert_eq!(p.cicp, Some((1, 13)));
        // The sRGB curve: linear toe below 0.04045, 0.5 → 0.2140.
        assert!((p.curves[0].eval(0.02) - 0.02 / 12.92).abs() < 1e-5);
        assert!((p.curves[1].eval(0.5) - 0.21404).abs() < 1e-4);
        let gamma = parse(&build(true, c, b"curv\0\0\0\0\0\0\0\x01\x02\x33\0\0", None)).unwrap();
        assert!(gamma.gray);
        assert!((gamma.curves[0].eval(0.5) - 0.5f64.powf(2.19921875)).abs() < 1e-9);
        let table = read_curve(b"curv\0\0\0\0\0\0\0\x03\0\0\x40\0\xff\xff").unwrap();
        assert!((table.eval(0.25) - 0.125).abs() < 1e-3);
    }

    #[test]
    fn refuses_lookup_table_and_cmyk_profiles() {
        let mut p = build(false, [[0.0; 3]; 3], &srgb_para(), None);
        p[16..20].copy_from_slice(b"CMYK");
        assert!(parse(&p).unwrap_err().contains("CMYK"));
        assert!(parse(b"garbage").is_err());
    }
}
