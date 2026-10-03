//! Visible blackbody emission for cinematic media, normalized to Y=1 at 6500 K.
//!
//! Planck radiance is integrated over 360–830 nm in 5 nm trapezoidal steps with
//! the CIE 1931 piecewise Gaussian fits of Wyman, Sloan & Shirley (2013), Eq. 4:
//! <https://jcgt.org/published/0002/02/01/>. XYZ is converted to linear sRGB,
//! clipping negative out-of-gamut channels. This preserves temperature-dependent
//! brightness; it does not normalize each temperature to white or unit intensity.
//! The volume emission scale supplies scene radiance per unit density and length;
//! these RGB values are not an SI prediction of the energy emitted by a gas.

use std::sync::OnceLock;

use crate::Error;

/// Supported temperature range, including zero as a non-emitting field value.
pub const MAX_KELVIN: f64 = 50_000.0;
/// Quadratically spaced temperature table used for bounded GPU evaluation.
pub const TABLE_INTERVALS: usize = 1024;

fn valid_temperature(kelvin: f64) -> Result<(), Error> {
    if !kelvin.is_finite() || !(0.0..=MAX_KELVIN).contains(&kelvin) {
        return Err(Error::Invalid("temperature must be finite and in 0..50000 kelvin"));
    }
    Ok(())
}

/// Planck spectral radiance in W sr⁻¹ m⁻³, at a wavelength in nanometres.
pub fn spectral_radiance(wavelength_nm: f64, kelvin: f64) -> Result<f64, Error> {
    valid_temperature(kelvin)?;
    if !wavelength_nm.is_finite() || wavelength_nm <= 0.0 {
        return Err(Error::Invalid("wavelength must be positive and finite"));
    }
    if kelvin == 0.0 {
        return Ok(0.0);
    }
    // Evaluate in log space to remain defined for extreme positive wavelengths.
    let log_lambda = wavelength_nm.ln() - 1e9_f64.ln();
    let x = (0.014_387_768_775_039_337_f64.ln() - log_lambda - kelvin.ln()).exp();
    let log_denominator = if x > 50.0 { x } else { x.exp_m1().ln() };
    let radiance = (1.191_042_972_397_188_4e-16_f64.ln() - 5.0 * log_lambda - log_denominator).exp();
    if !radiance.is_finite() {
        return Err(Error::Invalid("spectral radiance exceeds finite range"));
    }
    Ok(radiance)
}

fn cie(wavelength: f64) -> [f64; 3] {
    let lobe = |a: f64, b: f64, left: f64, right: f64| {
        let x = (wavelength - b) * if wavelength < b { left } else { right };
        a * (-0.5 * x * x).exp()
    };
    [
        lobe(0.362, 442.0, 0.0624, 0.0374) + lobe(1.056, 599.8, 0.0264, 0.0323) + lobe(-0.065, 501.1, 0.0490, 0.0382),
        lobe(0.821, 568.8, 0.0213, 0.0247) + lobe(0.286, 530.9, 0.0613, 0.0322),
        lobe(1.217, 437.0, 0.0845, 0.0278) + lobe(0.681, 459.0, 0.0385, 0.0725),
    ]
}

fn xyz(kelvin: f64) -> [f64; 3] {
    let mut xyz = [0.0; 3];
    for i in 0..=94 {
        let nm = 360.0 + f64::from(i) * 5.0;
        let weight = if i == 0 || i == 94 { 2.5e-9 } else { 5e-9 };
        let b = spectral_radiance(nm, kelvin).expect("validated temperature and visible wavelength") * weight;
        for (dst, cmf) in xyz.iter_mut().zip(cie(nm)) {
            *dst += b * cmf;
        }
    }
    xyz
}

/// Linear sRGB radiance relative to a 6500 K blackbody's photopic luminance.
pub fn blackbody_rgb(kelvin: f64) -> Result<[f64; 3], Error> {
    valid_temperature(kelvin)?;
    static REFERENCE_Y: OnceLock<f64> = OnceLock::new();
    let reference = *REFERENCE_Y.get_or_init(|| xyz(6500.0)[1]);
    let [x, y, z] = xyz(kelvin).map(|v| v / reference);
    Ok([
        3.2406 * x - 1.5372 * y - 0.4986 * z,
        -0.9689 * x + 1.8758 * y + 0.0415 * z,
        0.0557 * x - 0.2040 * y + 1.0570 * z,
    ]
    .map(|v| v.max(0.0)))
}

/// Table entry i corresponds to `MAX_KELVIN * (i / TABLE_INTERVALS)²`.
/// Shared across all media and renderers; no spectral integration in the shader.
pub fn blackbody_table() -> &'static [[f64; 3]; TABLE_INTERVALS + 1] {
    static TABLE: OnceLock<[[f64; 3]; TABLE_INTERVALS + 1]> = OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            let t = MAX_KELVIN * (i as f64 / TABLE_INTERVALS as f64).powi(2);
            blackbody_rgb(t).expect("table temperatures in range")
        })
    })
}
