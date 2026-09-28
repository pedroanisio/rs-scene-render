//! Light colour and photometry: colour temperature and IES profiles.

/// Linear sRGB of a blackbody at `kelvin`, normalised so the largest channel is 1
/// (CIE 1931 xy on the Planckian locus, Kim et al. cubic fit).
pub fn kelvin_to_rgb(kelvin: f64) -> [f32; 3] {
    let t = kelvin.clamp(1667.0, 25000.0);
    let x = if t <= 4000.0 {
        -0.266_123_9e9 / t.powi(3) - 0.234_358_9e6 / t.powi(2) + 0.877_695_6e3 / t + 0.179_910
    } else {
        -3.025_846_9e9 / t.powi(3) + 2.107_037_9e6 / t.powi(2) + 0.222_634_7e3 / t + 0.240_390
    };
    let y = if t <= 2222.0 {
        -1.106_381_4 * x.powi(3) - 1.348_110_20 * x.powi(2) + 2.185_558_32 * x - 0.202_196_83
    } else if t <= 4000.0 {
        -0.954_947_6 * x.powi(3) - 1.374_185_93 * x.powi(2) + 2.091_370_15 * x - 0.167_488_67
    } else {
        3.081_758_0 * x.powi(3) - 5.873_386_70 * x.powi(2) + 3.751_129_97 * x - 0.370_014_83
    };
    let (xx, zz) = (x / y, (1.0 - x - y) / y);
    let r = 3.240_454_2 * xx - 1.537_138_5 - 0.498_531_4 * zz;
    let g = -0.969_266_0 * xx + 1.876_010_8 + 0.041_556_0 * zz;
    let b = 0.055_643_4 * xx - 0.204_025_9 + 1.057_225_2 * zz;
    let m = r.max(g).max(b).max(1e-9);
    [(r / m).max(0.0) as f32, (g / m).max(0.0) as f32, (b / m).max(0.0) as f32]
}

/// An IESNA LM-63 photometric profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Ies {
    /// Vertical angles in degrees (0 = straight down the light's axis).
    pub vertical: Vec<f32>,
    /// Horizontal angles in degrees.
    pub horizontal: Vec<f32>,
    /// Candela per [horizontal][vertical].
    pub candela: Vec<Vec<f32>>,
}

impl Ies {
    /// Parses LM-63-1995/2002/2019 (TILT=NONE only).
    pub fn parse(text: &str) -> Result<Ies, String> {
        let tilt = text.find("TILT=").ok_or("no TILT line")?;
        let rest = &text[tilt..];
        let (tl, body) = rest.split_once('\n').ok_or("truncated")?;
        if !tl.trim().eq_ignore_ascii_case("TILT=NONE") {
            return Err("only TILT=NONE profiles are supported".into());
        }
        let nums: Vec<f32> =
            body.split(|c: char| c.is_whitespace() || c == ',').filter_map(|t| t.parse().ok()).collect();
        if nums.len() < 13 {
            return Err("truncated photometric data".into());
        }
        let multiplier = nums[2];
        let (nv, nh) = (nums[3] as usize, nums[4] as usize);
        let mut k = 13;
        let need = nv + nh + nv * nh;
        if nums.len() < k + need {
            return Err(format!("expected {need} values after the header, found {}", nums.len() - k));
        }
        let vertical = nums[k..k + nv].to_vec();
        k += nv;
        let horizontal = nums[k..k + nh].to_vec();
        k += nh;
        let candela =
            (0..nh).map(|h| nums[k + h * nv..k + (h + 1) * nv].iter().map(|c| c * multiplier).collect()).collect();
        Ok(Ies { vertical, horizontal, candela })
    }

    fn interp(axis: &[f32], v: f32) -> (usize, usize, f32) {
        if axis.len() < 2 || v <= axis[0] {
            return (0, 0, 0.0);
        }
        if v >= axis[axis.len() - 1] {
            let l = axis.len() - 1;
            return (l, l, 0.0);
        }
        let i = axis.partition_point(|&a| a <= v) - 1;
        (i, i + 1, (v - axis[i]) / (axis[i + 1] - axis[i]).max(1e-6))
    }

    /// Candela towards vertical angle `theta` and horizontal angle `phi` (degrees),
    /// with the profile's symmetry (0, 0–90, 0–180 or full) unfolded.
    pub fn sample(&self, theta: f32, phi: f32) -> f32 {
        let last = self.horizontal.last().copied().unwrap_or(0.0);
        let mut p = phi.rem_euclid(360.0);
        if last <= 0.0 {
            p = 0.0;
        } else if last <= 90.0 {
            p = if p > 180.0 { 360.0 - p } else { p };
            p = if p > 90.0 { 180.0 - p } else { p };
        } else if last <= 180.0 && p > 180.0 {
            p = 360.0 - p;
        }
        let (h0, h1, hu) = Self::interp(&self.horizontal, p);
        let (v0, v1, vu) = Self::interp(&self.vertical, theta);
        let at = |h: usize, v: usize| self.candela[h][v];
        let a = at(h0, v0) + (at(h0, v1) - at(h0, v0)) * vu;
        let b = at(h1, v0) + (at(h1, v1) - at(h1, v0)) * vu;
        if theta > self.vertical.last().copied().unwrap_or(180.0) {
            return 0.0;
        }
        a + (b - a) * hu
    }

    /// A `w`×`h` table over θ ∈ [0, 180] (columns) and φ ∈ [0, 360) (rows), normalised to a peak of 1.
    pub fn bake(&self, w: usize, h: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(w * h);
        for j in 0..h {
            for i in 0..w {
                out.push(self.sample(i as f32 / (w - 1) as f32 * 180.0, j as f32 / h as f32 * 360.0));
            }
        }
        let peak = out.iter().copied().fold(0.0f32, f32::max).max(1e-9);
        out.iter_mut().for_each(|v| *v /= peak);
        out
    }
}
