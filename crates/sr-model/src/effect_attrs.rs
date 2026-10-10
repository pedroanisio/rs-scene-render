//! The attributes of the shared `effect` attribute bag that each effect type reads.
//!
//! `effectType` in the schema is one flat bag of attributes shared by every effect type, so the schema accepts any of them on any type.
//! Each type reads a subset. A document that sets an attribute its type does not read gets no effect from it, silently:
//! a vignette given `intensity` keeps the default `radius` of 4 px. The evaluator reports them (SREP 34 `INERT-I13`) from these lists.
//!
//! The lists are declared by hand next to the kernels and are checked in the tests of the renderer: every read of an effect
//! attribute during a render must be in its type's list (or in [`ALWAYS`]), so a kernel that reads an undeclared attribute fails
//! the suite. A list that is too long only loses a warning; it never causes a wrong one. The `shader` type takes any
//! `<param>` and has no list.

/// Attributes every effect type reads (or that are not parameters of the kernel at all).
pub const ALWAYS: &[&str] = &["id", "type", "enabled", "mix"];

/// Per effect type, the bag attributes its kernel reads, sorted by type.
pub static DECLARED: &[(&str, &[&str])] = &[
    ("bevel", &["angle", "intensity", "size"]),
    ("bloom", &["color", "intensity", "radius", "threshold"]),
    ("blur", &["amount", "radius"]),
    ("bulge", &["amount", "centerX", "centerY", "radius"]),
    ("cdl", &["offset", "power", "saturation", "slope"]),
    ("chroma-key", &["keyColor", "softness", "spill", "tolerance"]),
    ("chromatic-aberration", &["amount", "centerX", "centerY"]),
    ("color-grade", &["brightness", "contrast", "saturation"]),
    ("color-overlay", &["amount", "color"]),
    ("curves", &["channel", "curve"]),
    ("difference-key", &["softness", "source", "tolerance"]),
    ("directional-blur", &["angle", "offsetX", "radius", "samples"]),
    ("displacement-map", &["amount", "centerX", "centerY", "source"]),
    ("drop-shadow", &["color", "compositeOriginal", "intensity", "offsetX", "offsetY", "radius"]),
    ("echo", &["amount", "intensity", "samples"]),
    ("emboss", &["amount", "angle", "relief"]),
    ("error-diffusion", &["kernel", "palette", "serpentine"]),
    ("exposure", &["exposure"]),
    ("fill", &["amount", "color"]),
    ("film-grain", &["amount", "response", "seed", "size"]),
    ("fractal-noise", &["amount", "frequency", "intensity", "seed", "size", "speed"]),
    ("glitch", &["amount", "blocks", "channelDelay", "quantization", "seed", "size"]),
    ("glow", &["color", "intensity", "radius", "threshold"]),
    ("god-rays", &["amount", "centerX", "centerY", "intensity", "samples", "threshold"]),
    ("gradient-map", &["amount", "angle", "paint", "source"]),
    ("gradient-overlay", &["amount", "angle", "paint", "source"]),
    ("grayscale", &["amount"]),
    ("halation", &["color", "intensity", "radius", "threshold"]),
    ("halftone", &["amount", "angle", "color", "paint", "size"]),
    ("heat-haze", &["amount", "centerX", "centerY", "size", "speed"]),
    ("hue-saturation", &["brightness", "hue", "saturation"]),
    ("inner-glow", &["color", "compositeOriginal", "intensity", "offsetX", "offsetY", "radius"]),
    ("inner-shadow", &["color", "compositeOriginal", "intensity", "offsetX", "offsetY", "radius"]),
    ("invert", &["amount", "channel"]),
    ("kaleidoscope", &["angle", "centerX", "centerY", "levels"]),
    ("lens-blur", &["amount", "angle", "centerX", "centerY", "levels", "radius", "samples", "size", "threshold"]),
    ("lens-distortion", &["amount", "centerX", "centerY"]),
    ("lens-flare", &["centerX", "centerY", "color", "intensity", "size"]),
    ("letterbox", &["color", "size"]),
    ("levels", &["channel", "gamma", "inputBlack", "inputWhite", "outputBlack", "outputWhite"]),
    ("lift-gamma-gain", &["gain", "gamma", "lift"]),
    ("light-leak", &["amount", "color", "intensity", "seed", "speed"]),
    ("light-sweep", &["angle", "color", "intensity", "size", "speed"]),
    ("lighting", &["angle", "color", "intensity", "lights", "relief"]),
    ("long-shadow", &["angle", "color", "offsetX", "size"]),
    ("luma-key", &["amount", "softness", "threshold"]),
    ("lut", &["src"]),
    ("matte-choke", &["amount", "centerX", "centerY", "color", "position", "size", "softness"]),
    ("mirror", &["angle", "centerX", "centerY"]),
    ("mosaic", &["centerX", "centerY", "size"]),
    ("noise", &["amount", "channel", "saturation"]),
    ("outline", &["amount", "centerX", "centerY", "color", "position", "size", "softness"]),
    ("pixel-motion-blur", &[]),
    ("pixelate", &["centerX", "centerY", "size"]),
    ("posterize", &["levels"]),
    ("posterize-time", &["frequency"]),
    ("radial-blur", &["amount", "angle", "centerX", "centerY", "samples"]),
    ("rgb-split", &["centerX", "centerY", "offsetX", "offsetY"]),
    ("ripple", &["amount", "centerX", "centerY", "size", "speed"]),
    ("scanlines", &["amount", "size"]),
    ("segmented-sort", &["direction", "high", "low", "order", "sortKey"]),
    ("selective-color", &["amount", "brightness", "color", "hue", "saturation", "tolerance"]),
    ("sepia", &["amount"]),
    ("sharpen", &["amount"]),
    ("spherize", &["amount", "centerX", "centerY", "radius"]),
    ("spill-suppress", &["keyColor", "spill"]),
    ("stroke", &["amount", "centerX", "centerY", "color", "position", "size", "softness"]),
    ("threshold", &["threshold"]),
    ("tile", &["amount", "centerX", "centerY", "size"]),
    ("tilt-shift", &["amount", "angle", "centerX", "centerY", "levels", "radius", "samples", "size", "threshold"]),
    ("tint", &["amount", "color"]),
    ("tonemap", &["exposure", "tonemapper"]),
    ("tritone", &["amount", "color"]),
    ("turbulent-displace", &["amount", "centerX", "centerY", "seed", "size", "speed"]),
    ("twirl", &["amount", "angle", "centerX", "centerY", "radius"]),
    ("unsharp-mask", &["amount", "radius", "threshold"]),
    ("vhs", &["amount", "centerX", "centerY"]),
    ("vignette", &["amount", "centerX", "centerY", "color", "radius", "softness"]),
    ("wave-warp", &["amount", "angle", "centerX", "centerY", "frequency", "size", "speed"]),
    ("white-balance", &["temperature", "tint"]),
    ("zoom-blur", &["amount", "centerX", "centerY", "samples"]),
];

/// The attributes `ty` reads, or `None` for a type with no list.
pub fn declared(ty: &str) -> Option<&'static [&'static str]> {
    DECLARED.binary_search_by(|(t, _)| (*t).cmp(ty)).ok().map(|i| DECLARED[i].1)
}

/// Whether `ty` reads attribute `name` of the bag.
pub fn reads(ty: &str, name: &str) -> bool {
    ALWAYS.contains(&name) || declared(ty).is_none_or(|l| l.contains(&name))
}
