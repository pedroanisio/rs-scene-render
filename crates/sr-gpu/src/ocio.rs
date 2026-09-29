//! OpenColorIO configurations (`colorManagement@ocioConfig`), evaluated by OpenColorIO itself.
//!
//! OCIO is a C++ library with no C interface, so instead of linking it the renderer runs OCIO's
//! own `ociobakelut` (shipped with the `opencolorio` Python package and OCIO builds), the way it
//! runs FFmpeg for media. The config's transform from the working space through the looks to
//! the display and view is baked onto a 129³ lattice indexed by the ACEScct curve of each working
//! channel (so the lattice follows the working space's own axes, where configs clamp), which
//! covers scene-linear values up to about 222: straight from the config's ACEScct when the working
//! space is AP1, otherwise through a log shaper of the config and resampled.
//! The frame is encoded into that lattice, looked up, and the display code values are decoded
//! with the output's transfer and primaries back to display-linear working colour, so the output
//! stage's own encoding reproduces them.
//!
//! The tool is `$SR_OCIOBAKELUT`, else `ociobakelut` on `PATH`.

use std::path::PathBuf;
use std::process::Command;

use sr_model::model::{ColorSpace, Transfer};

use crate::color;

/// Lattice size of the table (ACES 2.0 gamut mapping bends sharply near the gamut boundary:
/// 129³ keeps trilinear lookups within about 0.01 of OCIO's analytic result).
pub const SIZE: u32 = 129;

/// A baked display transform: `SIZE`³ display code values indexed by the ACEScct curve of each
/// working channel, red fastest.
#[derive(Clone, Debug)]
pub struct Baked {
    pub table: Vec<[f32; 3]>,
    /// The config colour space the table was baked from, and the shaper when it was resampled.
    pub input: String,
    pub shaper: Option<String>,
    /// Limits of the bake, for the render notes.
    pub notes: Vec<String>,
}

/// The `ociobakelut` executable, if one can be found.
pub fn tool() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SR_OCIOBAKELUT").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let exe = if cfg!(windows) { "ociobakelut.exe" } else { "ociobakelut" };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(exe)).find(|p| p.is_file())
}

/// Config colour-space names (or aliases and roles) that can hold the working space.
fn input_names(working: ColorSpace) -> &'static [&'static str] {
    match working {
        ColorSpace::Acescg => &["ACEScg", "lin_ap1", "ACES - ACEScg", "acescg"],
        ColorSpace::Aces2065_1 => &["ACES2065-1", "lin_ap0", "ACES - ACES2065-1", "aces_interchange"],
        ColorSpace::LinearSrgb => {
            &["lin_rec709_srgb", "Linear Rec.709 (sRGB)", "lin_rec709", "lin_srgb", "Linear Rec.709", "Linear"]
        }
        ColorSpace::Rec2020 => &["lin_rec2020", "Linear Rec.2020"],
        ColorSpace::DisplayP3 | ColorSpace::DciP3 => &["lin_p3d65", "Linear P3-D65"],
        ColorSpace::XyzD65 => &["CIE XYZ-D65 - Scene-referred", "Linear CIE-XYZ D65", "cie_xyz_d65_interchange"],
        _ => &[],
    }
}

/// Names under which configs carry ACEScct, the renderer's own lattice encoding.
const ACESCCT: &[&str] = &["ACEScct", "acescct_ap1", "ACES - ACEScct"];

/// Log spaces (or roles) to shape the scene-linear range with otherwise.
const SHAPERS: &[&str] = &["compositing_log", "color_timing", "AgX Log", "Filmic Log", "lg10"];

fn run(tool: &PathBuf, args: &[&str]) -> Result<String, String> {
    let out = Command::new(tool).args(args).output().map_err(|e| format!("{}: {e}", tool.display()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr);
        let msg = msg.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string();
        Err(msg)
    }
}

/// Whether the config knows colour space (or role or alias) `name`.
fn has_space(tool: &PathBuf, config: &str, name: &str) -> bool {
    run(
        tool,
        &[
            "--iconfig",
            config,
            "--inputspace",
            name,
            "--outputspace",
            name,
            "--format",
            "spi1d",
            "--cubesize",
            "2",
            "--stdout",
        ],
    )
    .is_ok()
}

/// Whether config space `name` is ACEScct of the working colour: a 17³ bake from it to `input`
/// must match the renderer's own ACEScct decoding and AP1 → working matrix.
fn is_acescct(tool: &PathBuf, config: &str, name: &str, input: &str, working: ColorSpace) -> bool {
    let Ok(text) = run(
        tool,
        &[
            "--iconfig",
            config,
            "--inputspace",
            name,
            "--outputspace",
            input,
            "--format",
            "resolve_cube",
            "--cubesize",
            "17",
            "--stdout",
        ],
    ) else {
        return false;
    };
    let Ok(cube) = Cube::parse(&text) else { return false };
    let m = color::convert(ColorSpace::Acescct, working);
    if cube.n3 == 0 {
        // per channel: only when the working space is AP1 itself
        let identity = (0..3).all(|i| (0..3).all(|j| (m[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 1e-6));
        return identity
            && cube.shaper.len() == 17
            && cube.range1 == [0.0, 1.0]
            && cube.shaper.iter().enumerate().all(|(k, row)| {
                let want = color::decode(Transfer::Acescct, k as f64 / 16.0);
                row.iter().all(|&g| (g as f64 - want).abs() <= 1e-3 * want.abs().max(1.0))
            });
    }
    if cube.n3 != 17 || !cube.shaper.is_empty() {
        return false;
    }
    (0..17usize.pow(3)).step_by(7).all(|k| {
        let e = [k % 17, (k / 17) % 17, k / 289].map(|v| color::decode(Transfer::Acescct, v as f64 / 16.0));
        let got = cube.table[k];
        (0..3).all(|i| {
            let want: f64 = (0..3).map(|j| m[i][j] * e[j]).sum();
            (got[i] as f64 - want).abs() <= 1e-3 * want.abs().max(1.0)
        })
    })
}

/// Bakes `config`'s transform from `working` through `looks` to `display`/`view`.
pub fn bake(config: &str, working: ColorSpace, display: &str, view: &str, looks: &[String]) -> Result<Baked, String> {
    let tool = tool().ok_or(
        "ociobakelut (OpenColorIO's tool, e.g. from `pip install opencolorio`) was not found; set SR_OCIOBAKELUT",
    )?;
    let input = input_names(working)
        .iter()
        .find(|n| has_space(&tool, config, n))
        .ok_or_else(|| format!("the config has no colour space for the working space {}", working.as_str()))?
        .to_string();
    let joined = looks.join(",");
    let size = SIZE.to_string();
    let mut tail: Vec<&str> = Vec::new();
    if !looks.is_empty() {
        tail.extend(["--looks", joined.as_str()]);
    }
    tail.extend(["--displayview", display, view, "--format", "resolve_cube", "--cubesize"]);
    // an AP1 working space and the config's own ACEScct: bake straight onto the lattice
    let direct = if working == ColorSpace::Acescg {
        ACESCCT.iter().find(|n| has_space(&tool, config, n) && is_acescct(&tool, config, n, &input, working))
    } else {
        None
    };
    if let Some(cct) = direct {
        let mut args = vec!["--iconfig", config, "--inputspace", cct];
        args.extend(tail.iter().copied());
        args.extend([size.as_str(), "--stdout"]);
        let cube = Cube::parse(&run(&tool, &args).map_err(|e| format!("ociobakelut: {e}"))?)?;
        if cube.n3 != SIZE as usize || !cube.shaper.is_empty() {
            return Err("ociobakelut: unexpected table size".into());
        }
        return Ok(Baked { table: cube.table, input: cct.to_string(), shaper: None, notes: Vec::new() });
    }
    // otherwise shape the working space with a log space of the config and resample
    let shaper = SHAPERS.iter().find(|n| has_space(&tool, config, n)).map(|s| s.to_string());
    let mut args = vec!["--iconfig", config, "--inputspace", &input];
    if let Some(s) = &shaper {
        args.extend(["--shaperspace", s, "--shapersize", "65536"]);
    }
    args.extend(tail.iter().copied());
    args.extend([size.as_str(), "--stdout"]);
    let cube = Cube::parse(&run(&tool, &args).map_err(|e| format!("ociobakelut: {e}"))?)?;
    let mut notes = Vec::new();
    if shaper.is_none() {
        notes.push("colorManagement/@ocioConfig: the config has no log space to shape the bake with; scene values above 1 clip".into());
    }
    Ok(Baked { table: cube.resample(), input, shaper, notes })
}

/// A Resolve cube: an optional 1D shaper with its input range, then a 3D table with its own.
struct Cube {
    shaper: Vec<[f32; 3]>,
    range1: [f64; 2],
    n3: usize,
    table: Vec<[f32; 3]>,
    range3: [f64; 2],
}

impl Cube {
    fn parse(text: &str) -> Result<Cube, String> {
        let (mut n1, mut n3) = (0usize, 0usize);
        let (mut range1, mut range3) = ([0.0, 1.0], [0.0, 1.0]);
        let mut rows = Vec::new();
        for l in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let mut it = l.split_whitespace();
            let key = it.next().unwrap_or("");
            let nums: Vec<f64> = it.filter_map(|v| v.parse().ok()).collect();
            match key {
                "LUT_1D_SIZE" => n1 = nums.first().copied().unwrap_or(0.0) as usize,
                "LUT_3D_SIZE" => n3 = nums.first().copied().unwrap_or(0.0) as usize,
                "LUT_1D_INPUT_RANGE" if nums.len() == 2 => range1 = [nums[0], nums[1]],
                "LUT_3D_INPUT_RANGE" if nums.len() == 2 => range3 = [nums[0], nums[1]],
                k if k.parse::<f64>().is_ok() && nums.len() == 2 => {
                    rows.push([k.parse::<f32>().unwrap_or(0.0), nums[0] as f32, nums[1] as f32])
                }
                _ => {}
            }
        }
        // a per-channel transform bakes to a 1D table alone
        if n3 == 0 && n1 >= 2 && rows.len() == n1 {
            return Ok(Cube { shaper: rows, range1, n3: 0, table: Vec::new(), range3 });
        }
        if n3 < 2 || rows.len() != n1 + n3 * n3 * n3 {
            return Err(format!(
                "ociobakelut: unexpected cube ({} rows for a {n1} shaper and a {n3}³ table)",
                rows.len()
            ));
        }
        let table = rows.split_off(n1);
        Ok(Cube { shaper: rows, range1, n3, table, range3 })
    }

    fn shape(&self, c: usize, v: f64) -> f64 {
        if self.shaper.len() < 2 {
            return v;
        }
        let n = self.shaper.len() - 1;
        let x = ((v - self.range1[0]) / (self.range1[1] - self.range1[0])).clamp(0.0, 1.0) * n as f64;
        let i = (x.floor() as usize).min(n - 1);
        let f = x - i as f64;
        self.shaper[i][c] as f64 * (1.0 - f) + self.shaper[i + 1][c] as f64 * f
    }

    fn lookup(&self, s: [f64; 3]) -> [f32; 3] {
        let n = self.n3;
        let at = |r: usize, g: usize, b: usize| self.table[r + g * n + b * n * n];
        let mut i = [0usize; 3];
        let mut f = [0.0f64; 3];
        for k in 0..3 {
            let x = ((s[k] - self.range3[0]) / (self.range3[1] - self.range3[0])).clamp(0.0, 1.0) * (n - 1) as f64;
            i[k] = (x.floor() as usize).min(n - 2);
            f[k] = x - i[k] as f64;
        }
        let mut out = [0.0f64; 3];
        for (dr, wr) in [(0, 1.0 - f[0]), (1, f[0])] {
            for (dg, wg) in [(0, 1.0 - f[1]), (1, f[1])] {
                for (db, wb) in [(0, 1.0 - f[2]), (1, f[2])] {
                    let w = wr * wg * wb;
                    let c = at(i[0] + dr, i[1] + dg, i[2] + db);
                    for k in 0..3 {
                        out[k] += w * c[k] as f64;
                    }
                }
            }
        }
        out.map(|v| v as f32)
    }

    /// The table on the renderer's lattice: the ACEScct curve of each working channel.
    fn resample(&self) -> Vec<[f32; 3]> {
        let n = SIZE as usize;
        let mut out = Vec::with_capacity(n * n * n);
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let w = [r, g, b].map(|k| color::decode(Transfer::Acescct, k as f64 / (n - 1) as f64));
                    let s = [0, 1, 2].map(|c| self.shape(c, w[c]));
                    out.push(self.lookup(s));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_with_shaper_parses_and_samples() {
        // shaper doubles into [0, 1] over input [0, 0.5]; the 2³ table is the identity
        let mut text = String::from("LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE 0 0.5\nLUT_3D_SIZE 2\n0 0 0\n1 1 1\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text += &format!("{r} {g} {b}\n");
                }
            }
        }
        let c = Cube::parse(&text).unwrap();
        assert_eq!(c.shaper.len(), 2);
        let s = [0.1, 0.25, 0.4].map(|v| c.shape(0, v));
        assert!((s[1] - 0.5).abs() < 1e-9);
        let o = c.lookup(s);
        assert!((o[0] - 0.2).abs() < 1e-6 && (o[2] - 0.8).abs() < 1e-6, "{o:?}");
        assert!(Cube::parse("LUT_3D_SIZE 2\n0 0 0\n").is_err());
    }
}
