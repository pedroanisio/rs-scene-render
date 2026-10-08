//! Which water takes the foam of `whitewater@foamMode="albedo"`: the criterion that the validator, the evaluator and the renderer share.
//!
//! The foam is mixed into the shading of a lit surface, so a water whose material is not opaque, is unlit or shines by itself is refused
//! by the renderer (see the SREP). The attributes are read as the document spells them: an attribute that animates is read by the renderer
//! at the time of each frame, and what is decided here from the document alone is of its value at the start.

/// Whether a water of this material takes the foam mix. `alpha_mode` is the material's `alphaMode` (opaque when absent), `unlit` its
/// `unlit`, `emissive` and `emissive_strength` its `emissive` colour and `emissiveStrength` (1 when absent): it shines when the strength is
/// over zero and the colour is not black. A colour that is not spelt as a hex literal or as `black` is taken to be a colour that shines
/// (it may be a token), so that the answer errs toward refusing a material that the renderer may also refuse.
pub fn water_takes_foam(
    alpha_mode: Option<&str>,
    unlit: Option<&str>,
    emissive: Option<&str>,
    emissive_strength: Option<&str>,
) -> bool {
    let opaque = !matches!(alpha_mode.map(str::trim), Some("mask" | "blend"));
    let unlit = matches!(unlit.map(str::trim), Some("true" | "1"));
    let strength = emissive_strength.map_or(1.0, |s| s.trim().trim_start_matches('+').parse::<f64>().unwrap_or(1.0));
    let shines = emissive.is_some_and(|c| !is_black(c)) && strength > 0.0;
    opaque && !unlit && !shines
}

/// Whether a colour literal is black: `black`, or a hex literal whose red, green and blue digits are all zero (the alpha is not light).
fn is_black(colour: &str) -> bool {
    let c = colour.trim();
    if c.eq_ignore_ascii_case("black") {
        return true;
    }
    let Some(hex) = c.strip_prefix('#') else { return false };
    let digits = match hex.len() {
        3 | 4 => 3,
        6 | 8 => 6,
        _ => return false,
    };
    hex.chars().take(digits).all(|d| d == '0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_opaque_lit_water_that_does_not_shine_takes_foam() {
        assert!(water_takes_foam(None, None, None, None), "the plain water");
        assert!(water_takes_foam(Some("opaque"), Some("false"), None, None));
        assert!(water_takes_foam(Some("opaque"), Some("0"), Some("#102040"), Some("0")), "no strength, no emission");
        for black in ["#000", "#000000", "#00000000", "#0000", "black", "BLACK", " #000000 "] {
            assert!(water_takes_foam(None, None, Some(black), Some("3")), "{black:?} gives no light");
        }
        assert!(water_takes_foam(None, None, Some("#FFFFFF"), Some("-1")), "a negative strength gives no light");
        assert!(water_takes_foam(None, None, Some("#FFFFFF"), Some("0.0")));
        assert!(!water_takes_foam(Some("blend"), None, None, None));
        assert!(!water_takes_foam(Some("mask"), None, None, None));
        assert!(!water_takes_foam(None, Some("true"), None, None));
        assert!(!water_takes_foam(None, Some("1"), None, None));
        for shining in ["#FFFFFF", "#000001", "#010000ff", "white", "token:glow"] {
            assert!(!water_takes_foam(None, None, Some(shining), None), "{shining:?}");
            assert!(!water_takes_foam(None, None, Some(shining), Some("0.5")), "{shining:?}");
        }
    }
}
