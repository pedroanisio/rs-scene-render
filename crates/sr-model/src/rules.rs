//! The Schematron rules of `schema/scene-render-1.1.sch` (patterns p1–p42 and p50–p60).
//!
//! Each rule reproduces its XPath 1.0 test exactly, including the edge cases
//! the XPath semantics imply:
//!
//! * `number()` follows libxml2 (the reference XPath engine named by the
//!   schema): decimal and exponent forms parse, `+1`, `INF` and `NaN` become
//!   `NaN`, and every comparison with `NaN` is false;
//! * attribute tests see only attributes written in the document, never XSD
//!   defaults;
//! * string comparisons are literal (`@reverse='false'` does not accept `0`).
//!
//! Diagnostic codes are the Schematron assert ids (`V1`–`V7`, `C1`–`C53`,
//! `R1`–`R37`, with per-attribute families `R24-<attr>`, `R25-<attr>`,
//! `R30-<attr>` and `R31-<attr>`).

//!
//! Further checks keep what later stages build within bounds: `P02` (a
//! count far beyond what a renderer can draw), `P03` (a symbol that contains
//! itself), `P04` (an expression nested too deeply) and `P05` (cyclic matte dependencies).

use std::collections::{HashMap, HashSet};

use roxmltree::{Document, Node};

use crate::diag::{element_path, Diagnostic, Loc};

/// Attributes that carry paints and may reference `url(#id)` or `var(--name)`.
pub const PAINT_ATTRS: [&str; 8] =
    ["fill", "stroke", "color", "background", "paint", "activeColor", "highlight", "strokeColor"];

const ALPHA_CODECS: [&str; 9] =
    ["prores", "vp9", "ffv1", "png-sequence", "exr-sequence", "tiff-sequence", "apng", "webp", "gif"];

const V1_SECTIONS: [&str; 11] = [
    "metadata",
    "parameters",
    "styles",
    "colorManagement",
    "layouts",
    "safeAreas",
    "paints",
    "symbols",
    "markers",
    "tracking",
    "captions",
];
const V3_ELEMENTS: [&str; 15] = [
    "sequence",
    "instance",
    "include",
    "repeat",
    "adjustment",
    "transition",
    "skeleton",
    "expression",
    "motionPath",
    "link",
    "timeRemap",
    "textAnimator",
    "textPath",
    "shapeModifier",
    "transformConstraint",
];
const V4_ASSETS: [&str; 9] =
    ["imageSequence", "lottie", "font", "generator", "chart", "audiogram", "code", "formula", "generated"];
const V6_ELEMENTS: [&str; 19] = [
    "audioEffect",
    "blob",
    "burst",
    "bus",
    "destination",
    "erosion",
    "flock",
    "fluid",
    "geo",
    "map",
    "master",
    "param",
    "pin",
    "poster",
    "representation",
    "shake",
    "slime",
    "span",
    "thumbnail",
];
const V7_PRIMITIVES: [&str; 7] = ["capsule", "clay", "cone", "cylinder", "extrude", "text", "torus"];
/// More paint attributes checked for `url(#id)` (p51, `R30-<attr>`).
const PAINT_REF_ATTRS: [&str; 7] = ["colorEnd", "colorHigh", "colorLow", "headFill", "noData", "outline", "paint2"];
/// More colour attributes checked for `var(--name)` (p52, `R31-<attr>`).
const TOKEN_REF_ATTRS: [&str; 15] = [
    "attenuationColor",
    "baseColor",
    "colorEnd",
    "colorHigh",
    "colorLow",
    "emissive",
    "foreground",
    "headFill",
    "keyColor",
    "noData",
    "outline",
    "paint2",
    "shadowColor",
    "sheenColor",
    "specularColor",
];

// ------------------------------------------------------------------ XPath helpers

/// `number()` of a string, as libxml2 evaluates it: XPath 1.0 `Number`
/// (`-?(digits(.digits?)?|.digits)` between XML whitespace) extended with an
/// optional exponent `[eE][+-]?digits*`. Anything else is `NaN`.
pub fn xpath_number(s: &str) -> f64 {
    let t = s.trim_matches([' ', '\t', '\n', '\r']).as_bytes();
    let mut i = 0;
    let neg = t.first() == Some(&b'-');
    if neg {
        i += 1;
    }
    let (mut mant, mut digits) = (String::new(), 0);
    while i < t.len() && t[i].is_ascii_digit() {
        mant.push(t[i] as char);
        i += 1;
        digits += 1;
    }
    if i < t.len() && t[i] == b'.' {
        mant.push('.');
        i += 1;
        while i < t.len() && t[i].is_ascii_digit() {
            mant.push(t[i] as char);
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return f64::NAN;
    }
    let mut exp: i64 = 0;
    if i < t.len() && (t[i] == b'e' || t[i] == b'E') {
        i += 1;
        let eneg = match t.get(i) {
            Some(b'-') => {
                i += 1;
                true
            }
            Some(b'+') => {
                i += 1;
                false
            }
            _ => false,
        };
        while i < t.len() && t[i].is_ascii_digit() {
            exp = (exp * 10 + (t[i] - b'0') as i64).min(100_000);
            i += 1;
        }
        if eneg {
            exp = -exp;
        }
    }
    if i != t.len() {
        return f64::NAN;
    }
    let v: f64 = format!("{mant}e{exp}").parse().unwrap_or(f64::NAN);
    if neg {
        -v
    } else {
        v
    }
}

fn num(n: Node, attr: &str) -> f64 {
    n.attribute(attr).map(xpath_number).unwrap_or(f64::NAN)
}

/// XPath 1.0 `normalize-space()`.
pub fn normalize_space(s: &str) -> String {
    crate::xsd::simple::collapse(s)
}

/// EXSLT `str:tokenize(normalize-space(s), ' ')`.
fn tokens(s: &str) -> usize {
    normalize_space(s).split(' ').filter(|t| !t.is_empty()).count()
}

fn is(n: Node, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name && n.tag_name().namespace().is_none()
}

fn kids<'a, 'i: 'a>(n: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children().filter(move |c| is(*c, name))
}

fn has_kid(n: Node, name: &str) -> bool {
    kids(n, name).next().is_some()
}

/// XPath `substring-before(substring-after(s, open), ')')`.
fn between(s: &str, open: &str) -> String {
    match s.split_once(open) {
        Some((_, after)) => after.split_once(')').map(|(b, _)| b.to_string()).unwrap_or_default(),
        None => String::new(),
    }
}

// ------------------------------------------------------------------ indexes of /scene/...

#[derive(Default)]
struct Sets<'a> {
    assets: HashSet<&'a str>,
    text_assets: HashSet<&'a str>,
    font_assets: HashSet<&'a str>,
    mesh_assets: HashSet<&'a str>,
    mesh_sequences: HashSet<&'a str>,
    sequence_colliders: HashSet<&'a str>,
    volume_assets: HashSet<&'a str>,
    pyro_colliders: HashSet<&'a str>,
    /// Objects that hold a native pyro volume.
    pyro_volumes: HashSet<&'a str>,
    /// 3D particle emitters that throw out the ejecta of a crater, whose particles carry a mass.
    ejecta_emitters: HashSet<&'a str>,
    /// Objects an ocean can use as a deformable bed (a crater on a plane or a mesh)
    /// or as a closed body (any other supported primitive, without a crater).
    ocean_colliders: HashSet<&'a str>,
    dynamic_pyro_geometry: HashSet<&'a str>,
    geo_assets: Vec<&'a str>,
    image_assets: HashSet<&'a str>,
    layered_images: HashSet<&'a str>,
    /// Image, image sequence, video and generator assets (sprites).
    sprite_assets: HashSet<&'a str>,
    /// `@id` of every `/scene/physics/forceField`.
    force_fields: Vec<&'a str>,
    map_assets: Vec<&'a str>,
    tiles_assets: Vec<&'a str>,
    audio_assets: HashSet<&'a str>,
    generated_audio: HashSet<&'a str>,
    video_with_audio: HashSet<&'a str>,
    materials: HashSet<&'a str>,
    markers: HashSet<&'a str>,
    text_styles: HashSet<&'a str>,
    token_names: HashSet<&'a str>,
    paints: HashSet<&'a str>,
    buses: HashSet<&'a str>,
    audio_tracks: HashSet<&'a str>,
    /// `@id` of every `/scene/audioMix/audioTrack` (empty string when absent).
    mix_tracks: Vec<&'a str>,
    caption_tracks: HashSet<&'a str>,
    layouts: HashSet<&'a str>,
    variants: HashSet<&'a str>,
    params: HashSet<&'a str>,
    list_params: HashSet<&'a str>,
    data: HashSet<&'a str>,
    safe_areas: HashSet<&'a str>,
    symbols: HashSet<&'a str>,
    track_data: HashSet<&'a str>,
    /// `@id` of every `/scene/effects/effect` (empty string when absent).
    effects: Vec<&'a str>,
    /// `@id` of every `/scene/lights/light` (empty string when absent).
    lights: Vec<&'a str>,
    composition_desc: HashSet<&'a str>,
    symbols_desc: HashSet<&'a str>,
}

fn build_sets<'a>(scene: Option<Node<'a, '_>>) -> Sets<'a> {
    let mut s = Sets::default();
    let Some(scene) = scene else { return s };
    for emitter in scene.descendants().filter(|n| is(*n, "particles3D")) {
        if let Some(id) = emitter.attribute("id") {
            if emitter.children().any(|b| is(b, "burst") && b.attribute("crater").is_some()) {
                s.ejecta_emitters.insert(id);
            }
        }
    }
    for object in scene.descendants().filter(|n| is(*n, "object3D")) {
        let Some(id) = object.attribute("id") else { continue };
        let crater = kids(object, "crater").next().is_some();
        let primitive = object.attribute("primitive");
        if if crater {
            matches!(primitive, Some("plane" | "mesh"))
        } else {
            matches!(
                primitive,
                Some(
                    "box"
                        | "sphere"
                        | "globe"
                        | "cylinder"
                        | "cone"
                        | "capsule"
                        | "torus"
                        | "mesh"
                        | "text"
                        | "extrude"
                        | "clay"
                )
            )
        } {
            s.ocean_colliders.insert(id);
        }
        if matches!(
            object.attribute("primitive"),
            Some(
                "box"
                    | "sphere"
                    | "globe"
                    | "plane"
                    | "cylinder"
                    | "cone"
                    | "capsule"
                    | "torus"
                    | "mesh"
                    | "text"
                    | "extrude"
                    | "clay"
            )
        ) {
            s.pyro_colliders.insert(id);
        }
        if object.children().any(|child| is(child, "pyro")) {
            s.pyro_volumes.insert(id);
        }
        let procedural_motion = object.children().any(|child| match object.attribute("primitive") {
            Some("text") => matches!(child.attribute("property"), Some("text" | "font" | "bevel")),
            Some("extrude") => matches!(child.attribute("property"), Some("path" | "bevel")),
            Some("clay") => {
                matches!(child.attribute("property"), Some("resolution" | "fingerprints" | "seed" | "boil"))
            }
            _ => false,
        }) || (object.attribute("primitive") == Some("clay")
            && (kids(object, "blob").any(|b| b.children().any(|c| c.attribute("property").is_some()))
                || (object.attribute("boil").and_then(|s| s.trim().parse::<f64>().ok()).is_some_and(|v| v > 0.)
                    && object
                        .attribute("fingerprints")
                        .and_then(|s| s.trim().parse::<f64>().ok())
                        .is_some_and(|v| v > 0.))));
        if procedural_motion
            || object.children().any(|child| {
                matches!(
                    child.attribute("property"),
                    Some("primitive" | "mesh" | "radius" | "width" | "height" | "depth" | "segments")
                ) || (object.attribute("primitive") == Some("globe")
                    && object.attribute("terrain").is_some()
                    && child.attribute("property") == Some("exaggeration"))
            })
        {
            s.dynamic_pyro_geometry.insert(id);
        }
    }
    for assets in kids(scene, "assets") {
        for a in assets.children().filter(|c| c.is_element()) {
            let Some(i) = a.attribute("id") else { continue };
            s.assets.insert(i);
            if is(a, "image") || is(a, "imageSequence") || is(a, "video") || is(a, "generator") {
                s.sprite_assets.insert(i);
            }
            if is(a, "image") {
                s.image_assets.insert(i);
                if a.attribute("layer").is_some() {
                    s.layered_images.insert(i);
                }
            }
            if is(a, "text") {
                s.text_assets.insert(i);
            } else if is(a, "font") {
                s.font_assets.insert(i);
            } else if is(a, "mesh") {
                s.mesh_assets.insert(i);
            } else if is(a, "meshSequence") {
                s.mesh_sequences.insert(i);
            } else if is(a, "volume") {
                s.volume_assets.insert(i);
            } else if is(a, "geo") {
                s.geo_assets.push(i);
            } else if is(a, "tiles") {
                s.tiles_assets.push(i);
            } else if is(a, "map") {
                s.map_assets.push(i);
            } else if is(a, "audio") {
                s.audio_assets.insert(i);
            } else if is(a, "generated") && matches!(a.attribute("kind"), Some("speech" | "music" | "sound-effect")) {
                s.generated_audio.insert(i);
            } else if is(a, "video") && matches!(a.attribute("hasAudio"), Some("true" | "1")) {
                s.video_with_audio.insert(i);
            }
        }
    }
    let collect = |section: &'static str, child: &'static str, set: &mut HashSet<&'a str>| {
        for sec in kids(scene, section) {
            for c in sec.children().filter(|c| is(*c, child)) {
                if let Some(i) = c.attribute("id") {
                    set.insert(i);
                }
            }
        }
    };
    collect("materials", "material", &mut s.materials);
    collect("markers", "marker", &mut s.markers);
    collect("styles", "textStyle", &mut s.text_styles);
    collect("audioMix", "bus", &mut s.buses);
    collect("audioMix", "audioTrack", &mut s.audio_tracks);
    for m in kids(scene, "audioMix") {
        s.mix_tracks.extend(kids(m, "audioTrack").map(|x| x.attribute("id").unwrap_or("")));
    }
    collect("captions", "captionTrack", &mut s.caption_tracks);
    collect("layouts", "layout", &mut s.layouts);
    collect("parameters", "variant", &mut s.variants);
    collect("parameters", "param", &mut s.params);
    collect("parameters", "data", &mut s.data);
    collect("safeAreas", "safeArea", &mut s.safe_areas);
    collect("symbols", "symbol", &mut s.symbols);
    collect("tracking", "trackData", &mut s.track_data);
    for p in kids(scene, "parameters") {
        for c in kids(p, "param").filter(|c| c.attribute("type") == Some("list")) {
            if let Some(i) = c.attribute("id") {
                s.list_params.insert(i);
            }
        }
    }
    for st in kids(scene, "styles") {
        s.token_names.extend(kids(st, "token").filter_map(|t| t.attribute("name")));
    }
    for p in kids(scene, "paints") {
        s.paints.extend(p.children().filter(|c| c.is_element()).filter_map(|c| c.attribute("id")));
    }
    for p in kids(scene, "physics") {
        s.force_fields.extend(kids(p, "forceField").map(|x| x.attribute("id").unwrap_or("")));
    }
    for e in kids(scene, "effects") {
        s.effects.extend(kids(e, "effect").map(|x| x.attribute("id").unwrap_or("")));
    }
    for l in kids(scene, "lights") {
        s.lights.extend(kids(l, "light").map(|x| x.attribute("id").unwrap_or("")));
    }
    for c in kids(scene, "composition") {
        s.composition_desc.extend(c.descendants().skip(1).filter(|d| d.is_element()).filter_map(|d| d.attribute("id")));
    }
    for c in kids(scene, "symbols") {
        s.symbols_desc.extend(c.descendants().skip(1).filter(|d| d.is_element()).filter_map(|d| d.attribute("id")));
    }
    for object in scene.descendants().filter(|n| is(*n, "object3D")) {
        if object.attribute("primitive") == Some("mesh") && contains(&s.mesh_sequences, object.attribute("mesh")) {
            if let Some(id) = object.attribute("id") {
                s.sequence_colliders.insert(id);
            }
        }
    }
    s
}

fn contains(set: &HashSet<&str>, v: Option<&str>) -> bool {
    v.is_some_and(|v| set.contains(v))
}

/// XPath `count(tokens) = count(items[contains(concat(' ',normalize-space(list),' '), concat(' ',@id,' '))])`.
fn every_token_names(list: &str, items: &[&str]) -> bool {
    let norm = format!(" {} ", normalize_space(list));
    let matched = items.iter().filter(|id| norm.contains(&format!(" {id} "))).count();
    tokens(list) == matched
}

/// `not(child[number(@attr) < number(preceding-sibling::child[1]/@attr)])`.
fn non_decreasing(n: Node, child: &str, attr: &str) -> bool {
    let mut prev = f64::NAN;
    for c in kids(n, child) {
        let cur = num(c, attr);
        if cur < prev {
            return false;
        }
        prev = cur;
    }
    true
}

// ------------------------------------------------------------------ evaluation

struct Eval<'a> {
    sets: Sets<'a>,
    out: Vec<Diagnostic>,
}

/// Cells between a pyro source and an open face below which W02 warns. It comes from one plume (the hero scene's,
/// 64 x 52 x 64 cells), one face (the bottom) and one kind of source, and the side faces were not measured.
const OPEN_FACE_CELLS: f64 = 12.0;

impl<'a> Eval<'a> {
    fn check(&mut self, ok: bool, n: Node, code: &str, message: impl FnOnce() -> String) {
        if !ok {
            self.out.push(Diagnostic::error(code, message(), Loc::of(n), element_path(n)));
        }
    }

    /// A warning: the document is valid and a renderer can run it.
    fn warn(&mut self, n: Node, code: &str, message: String) {
        self.out.push(Diagnostic::warning(code, message, Loc::of(n), element_path(n)));
    }

    /// W02: a pyro source or impulse whose place is in the document and which is closer than
    /// [`OPEN_FACE_CELLS`] cells to an open face of a volume that has room to put it further in.
    /// The face is a zero-pressure outlet and an inlet for the surrounding air, so a source this near
    /// it changes the flow of the whole cloud, which the engine measured on a plume of 64 x 52 x 64
    /// cells (the peak density 22 % lower with the face 7 cells from the source than with it 33).
    fn pyro_source_near_open_face(&mut self, n: Node) {
        let Some(pyro) = n.parent_element().filter(|p| is(*p, "pyro")) else { return };
        let number = |e: Node, k: &str, default: f64| e.attribute(k).map_or(default, xpath_number);
        // a source from a crater has no place in the document, and an animated one moves
        if pyro.attribute("boundary") != Some("open")
            || n.attribute("crater").is_some()
            || n.children().any(|c| c.is_element())
        {
            return;
        }
        let size = [number(pyro, "width", f64::NAN), number(pyro, "height", f64::NAN), number(pyro, "depth", f64::NAN)];
        let voxel = number(pyro, "voxelSize", f64::NAN);
        let centre = [number(n, "x", 0.0), number(n, "y", 0.0), number(n, "z", 0.0)];
        let scale = [number(n, "scaleX", 1.0).abs(), number(n, "scaleY", 1.0).abs(), number(n, "scaleZ", 1.0).abs()];
        let widest = scale.iter().cloned().fold(0.0, f64::max);
        let turned = ["rotation", "rotationX", "rotationY"].iter().any(|k| number(n, k, 0.0) != 0.0);
        // how far the shape reaches from its centre along each axis, or None where it cannot be told
        let reach: [f64; 3] = match n.attribute("shape").unwrap_or("sphere") {
            "sphere" => [number(n, "radius", 1.0) * widest; 3],
            "box" => {
                let half = [number(n, "width", f64::NAN), number(n, "height", f64::NAN), number(n, "depth", f64::NAN)];
                if turned {
                    let radius = half.iter().zip(&scale).map(|(h, s)| (0.5 * h * s).powi(2)).sum::<f64>().sqrt();
                    [radius; 3]
                } else {
                    std::array::from_fn(|i| 0.5 * half[i] * scale[i])
                }
            }
            _ => return,
        };
        if !(voxel.is_finite() && voxel > 0.0 && size.iter().chain(&reach).chain(&centre).all(|v| v.is_finite())) {
            return;
        }
        let names = [["-x", "+x"], ["-y", "+y"], ["-z", "+z"]];
        let mut nearest: Option<(f64, &str)> = None;
        for axis in 0..3 {
            let cells = size[axis] / voxel;
            // room to put the source OPEN_FACE_CELLS from both faces of this axis
            if cells < 2.0 * OPEN_FACE_CELLS + 2.0 * reach[axis] / voxel {
                continue;
            }
            let half = 0.5 * size[axis];
            for (side, gap) in [(0, centre[axis] - reach[axis] + half), (1, half - centre[axis] - reach[axis])] {
                let gap = gap / voxel;
                if gap < OPEN_FACE_CELLS && nearest.is_none_or(|(least, _)| gap < least) {
                    nearest = Some((gap, names[axis][side]));
                }
            }
        }
        if let Some((gap, face)) = nearest {
            self.warn(
                n,
                "W02",
                format!(
                    "the edge of this pyro source is {gap:.1} cells from the open {face} face of its volume; closer \
                     than {OPEN_FACE_CELLS} cells, the face changes the flow of the cloud (that number comes from one \
                     plume and one face)."
                ),
            );
        }
    }

    /// BH1 to BH8 and W03 to W05: a Schwarzschild black hole, its disk and the camera that traces geodesics.
    fn black_hole(&mut self, n: Node, local: &str) {
        let root = n.document().root_element();
        let all = |name: &'static str| root.descendants().filter(move |d| d.is_element() && is(*d, name));
        let number = |e: Node, k: &str, default: f64| {
            e.attribute(k).map_or(default, |s| xpath_number(s.trim().strip_prefix('+').unwrap_or(s.trim())))
        };
        let geodesics = local == "camera" && n.attribute("geodesics") == Some("true");
        match local {
            "blackHole" | "accretionDisk" => {}
            "camera" if n.has_attribute("geodesics") => {}
            _ => return,
        }
        self.check(root.attribute("version") == Some("1.3"), n, "BH1", || {
            "black holes require version=\"1.3\".".into()
        });
        match local {
            "blackHole" => {
                self.check(all("blackHole").count() == 1, n, "BH2", || {
                    "version 1.3 has one blackHole per scene.".into()
                });
            }
            "accretionDisk" => {
                let hole =
                    n.attribute("blackHole").and_then(|id| all("blackHole").find(|h| h.attribute("id") == Some(id)));
                self.check(hole.is_some(), n, "BH3", || "an accretionDisk must name a blackHole.".into());
                if let Some(hole) = hole {
                    let (mass, outer) = (number(hole, "mass", f64::NAN), number(n, "outerRadius", f64::NAN));
                    let inner = n.has_attribute("innerRadius").then(|| number(n, "innerRadius", f64::NAN));
                    let ok = inner.is_none_or(|i| i >= 6.0 * mass) && outer > inner.unwrap_or(6.0 * mass);
                    self.check(ok, n, "BH4", || {
                        "the inner radius of an accretionDisk is at least 6 times the mass of its blackHole (the \
                         innermost stable circular orbit), and the outer radius is beyond the inner one."
                            .into()
                    });
                }
            }
            _ if geodesics => {
                let hole = all("blackHole").next();
                self.check(hole.is_some(), n, "BH5", || "a camera with geodesics=\"true\" needs a blackHole.".into());
                const OTHER: [&str; 10] = [
                    "object3D",
                    "particles3D",
                    "particleEmitter",
                    "ocean",
                    "fluid",
                    "flock",
                    "slime",
                    "erosion",
                    "pyro",
                    "medium",
                ];
                self.check(!OTHER.iter().any(|name| all(name).next().is_some()), n, "BH6", || {
                    "a camera with geodesics=\"true\" renders only the black hole, its disk and the 2D layers: no \
                     object3D, particles3D, particleEmitter, ocean, fluid, flock, slime, erosion, pyro or medium may \
                     be in the scene."
                        .into()
                });
                if let Some(hole) = hole {
                    let mass = number(hole, "mass", f64::NAN);
                    let d2: f64 =
                        ["x", "y", "z"].iter().map(|k| (number(n, k, 0.0) - number(hole, k, 0.0)).powi(2)).sum();
                    self.check(d2 > 9.0 * mass * mass, n, "BH7", || {
                        "a camera with geodesics=\"true\" must be farther than 3 times the mass of the blackHole from \
                         it, the photon sphere."
                            .into()
                    });
                }
                self.check(
                    all("camera").filter(|c| c.attribute("geodesics") == Some("true")).count() <= 1,
                    n,
                    "BH8",
                    || "a scene has at most one camera with geodesics=\"true\".".into(),
                );
                if n.attribute("denoise") == Some("true") {
                    self.warn(
                        n,
                        "W04",
                        "a camera with geodesics=\"true\" does not denoise: pathSamples are antialiasing samples."
                            .into(),
                    );
                }
                if all("light").next().is_some() {
                    self.warn(n, "W05", "lights are not used by a camera with geodesics=\"true\".".into());
                }
            }
            _ => {}
        }
        if matches!(local, "blackHole" | "accretionDisk")
            && !all("camera").any(|c| c.attribute("geodesics") == Some("true"))
        {
            self.warn(
                n,
                "W03",
                "no camera has geodesics=\"true\", so the black hole and its disk are not rendered.".into(),
            );
        }
    }

    /// p1: what version="1.0" documents cannot use.
    fn version_1_0(&mut self, n: Node) {
        self.check(!V1_SECTIONS.iter().any(|s| has_kid(n, s)), n, "V1", || {
            "version=\"1.0\" documents cannot use 1.1 sections; set version=\"1.1\".".into()
        });
        self.check(kids(n, "output").count() <= 1, n, "V2", || "version=\"1.0\" allows one output element.".into());
        let v3 = n.descendants().skip(1).any(|d| V3_ELEMENTS.iter().any(|e| is(d, e)));
        self.check(!v3, n, "V3", || "version=\"1.0\" documents cannot use 1.1 node or animation elements.".into());
        let v4 = kids(n, "assets").any(|s| s.children().any(|c| V4_ASSETS.iter().any(|e| is(c, e))));
        self.check(!v4, n, "V4", || "version=\"1.0\" documents cannot use 1.1 asset kinds.".into());
        // p50
        let v6 = n.descendants().skip(1).any(|d| V6_ELEMENTS.iter().any(|e| is(d, e)));
        self.check(!v6, n, "V6", || {
            "version=\"1.0\" documents cannot use 1.1 elements (simulation nodes, geo and map assets, clay \
             blobs, audio buses and effects, output posters, thumbnails and destinations, camera shake, \
             representations, text spans, effect params); set version=\"1.1\"."
                .into()
        });
        let v7 = n
            .descendants()
            .any(|d| is(d, "object3D") && d.attribute("primitive").is_some_and(|p| V7_PRIMITIVES.contains(&p)));
        self.check(!v7, n, "V7", || {
            "version=\"1.0\" documents cannot use the 1.1 object3D primitives (capsule, clay, cone, \
             cylinder, extrude, text, torus); set version=\"1.1\"."
                .into()
        });
    }

    fn element(&mut self, n: Node) {
        let a = |k: &str| n.attribute(k);
        let has = |k: &str| n.attribute(k).is_some();
        let v = |k: &str| n.attribute(k).unwrap_or("").to_string();
        let local = if n.tag_name().namespace().is_none() { n.tag_name().name() } else { "" };
        let parent_is = |name: &str| n.parent_element().is_some_and(|p| is(p, name));

        if local == "scene" && n.parent_element().is_none() && a("version") != Some("1.3") {
            let uses_volume = n.descendants().any(|d| {
                is(d, "volume")
                    || is(d, "medium")
                    || is(d, "pyro")
                    || (is(d, "object3D")
                        && (d.attribute("primitive") == Some("volume") || d.attribute("volume").is_some()))
            });
            self.check(!uses_volume, n, "V8", || "volumetric assets and media require version=\"1.3\".".into());
        }

        if matches!(local, "object3D" | "pyro" | "particles3D") {
            let own = local == "object3D"
                && a("primitive") == Some("mesh")
                && kids(n, "rigidBody").next().is_some()
                && !(kids(n, "fracture").next().is_some()
                    && kids(n, "rigidBody").any(|b| matches!(b.attribute("type"), Some("static" | "kinematic"))))
                && contains(&self.sets.mesh_sequences, a("mesh"));
            let reference = a("colliders")
                .is_some_and(|ids| ids.split_whitespace().any(|id| self.sets.sequence_colliders.contains(id)));
            self.check(!own && !reference, n, "MSQ4", || "mesh sequences are changing render geometry; rigid collider consumers require an explicit static proxy.".into());
        }
        match local {
            "fracture" => {
                self.check(n.document().root_element().attribute("version") == Some("1.3"), n, "FRX1", || {
                    "fracture requires version=\"1.3\".".into()
                });
                self.check(
                    n.parent_element().is_some_and(|o| {
                        is(o, "object3D")
                            && !matches!(o.attribute("primitive"), Some("volume" | "plane" | "map"))
                            && kids(o, "fracture").count() == 1
                            && kids(o, "rigidBody").count() == 1
                    }),
                    n,
                    "FRX2",
                    || "fracture requires one closed surface object3D owner and exactly one rigidBody.".into(),
                );
                self.check(contains(&self.sets.materials, a("interiorMaterial")), n, "FRX3", || {
                    "fracture interiorMaterial must reference a declared material.".into()
                });
                let finite = n.attributes().filter(|a| a.name() != "interiorMaterial").all(|a| {
                    let s = a.value().trim();
                    xpath_number(s.strip_prefix('+').unwrap_or(s)).is_finite()
                });
                self.check(finite, n, "FRX4", || "fracture numeric values must be finite.".into());
            }
            "crater" => {
                let number = |k, default| {
                    a(k).map(|s| xpath_number(s.trim().strip_prefix('+').unwrap_or(s.trim()))).unwrap_or(default)
                };
                self.check(n.document().root_element().attribute("version") == Some("1.3"), n, "CRT1", || {
                    "crater deformation requires version=\"1.3\".".into()
                });
                let owner = n.parent_element();
                self.check(
                    owner.is_some_and(|o| {
                        is(o, "object3D")
                            && o.attribute("primitive") != Some("volume")
                            && kids(o, "crater").count() == 1
                    }),
                    n,
                    "CRT2",
                    || "one crater belongs to a surface object3D, not a volume.".into(),
                );
                self.check(number("end", 1.) > number("start", 0.), n, "CRT3", || {
                    "crater end must be greater than start in the object's local clock.".into()
                });
                let finite = n
                    .attributes()
                    .filter(|a| !matches!(a.name(), "curve" | "id" | "source" | "capture" | "targetMaterial"))
                    .all(|a| number(a.name(), 0.).is_finite());
                let direction = [number("normalX", 0.), number("normalY", 0.), number("normalZ", -1.)];
                let envelope = !has("influenceDepth")
                    || has("source")
                    || (number("influenceDepth", 0.) * 0.5 >= number("depth", 10.).max(number("rimHeight", 2.)));
                self.check(finite && direction.iter().any(|&x|x!=0.) && number("rimWidth",10.)<=number("radius",50.) && envelope,n,"CRT4",||"crater values must be finite, its normal nonzero, rimWidth <= radius and, for a crater without a source, influenceDepth >= twice max(depth,rimHeight); a crater with a source has its size from the impact, and its envelope is checked when it is made.".into());
                let derived = ["radius", "depth", "rimHeight", "rimWidth", "start", "end"]
                    .iter()
                    .chain(["centerX", "centerY", "centerZ", "normalX", "normalY", "normalZ"].iter());
                self.check(a("source").is_none() || !derived.into_iter().any(|k| a(k).is_some()), n, "CRT6", || {
                    "a crater that grows from a source derives its size, timing, centre and axis, so none of them may be given."
                        .into()
                });
                let target = ["targetMaterial", "targetDensity", "strength", "gravity"];
                self.check(
                    if a("source").is_some() {
                        a("targetMaterial").is_some()
                    } else {
                        !target.iter().any(|k| a(k).is_some())
                    },
                    n,
                    "CRT7",
                    || {
                        "crater source requires targetMaterial, and targetMaterial, targetDensity, strength and gravity belong to a crater with a source."
                            .into()
                    },
                );
                self.check(
                    a("source").is_none_or(|id| {
                        n.document().descendants().any(|o| {
                            is(o, "object3D")
                                && o.attribute("id") == Some(id)
                                && Some(o) != owner
                                && kids(o, "rigidBody").any(|b| matches!(b.attribute("type"), None | Some("dynamic")))
                        })
                    }),
                    n,
                    "CRT8",
                    || "crater source must name another object3D whose rigidBody is dynamic.".into(),
                );
                self.check(!has("capture") || has("source"), n, "CRT9", || {
                    "crater capture belongs to a crater that grows from a source.".into()
                });
                self.check(
                    owner.is_none_or(|o| {
                        kids(o, "rigidBody").all(|b| {
                            matches!(b.attribute("type"), Some("static" | "kinematic"))
                                && matches!(b.attribute("shape"), None | Some("auto" | "trimesh"))
                        })
                    }),
                    n,
                    "CRT5",
                    || {
                        "crater rigid bodies require static/kinematic type with auto or trimesh collision geometry."
                            .into()
                    },
                );
            }
            // p1 — version gate
            "scene" if n.parent_element().is_none() && matches!(a("version"), Some("1.0" | "1.1")) => {
                if a("version") == Some("1.0") {
                    self.version_1_0(n);
                }
                // p1b
                let v5 = kids(n, "assets").any(|s| s.children().any(|c| is(c, "tiles")))
                    || n.descendants().any(|d| {
                        is(d, "basemap")
                            || (is(d, "rigidBody") && d.parent_element().is_some_and(|p| is(p, "object3D")))
                            || (is(d, "object3D") && matches!(d.attribute("primitive"), Some("map" | "globe")))
                    })
                    || kids(n, "output").any(|o| {
                        o.children().any(|c| is(c, "segment") || is(c, "audioTrack") || is(c, "captionTrack"))
                    });
                self.check(!v5, n, "V5", || {
                    "documents before version=\"1.2\" cannot use 1.2 elements or asset kinds; set version=\"1.2\"."
                        .into()
                });
            }
            // p2
            "vector" => {
                self.check(a("shape") != Some("path") || has("path"), n, "C1", || {
                    "vector shape=\"path\" requires @path.".into()
                });
                self.check(a("shape") != Some("svg") || has("src"), n, "C2", || {
                    "vector shape=\"svg\" requires @src.".into()
                });
            }
            // p3
            "shape" => {
                self.check(a("shape") != Some("path") || has("path"), n, "C3", || {
                    "shape shape=\"path\" requires @path.".into()
                });
                // p66
                let marked = ["markerStart", "markerEnd"].iter().any(|k| a(k).is_some_and(|v| v != "none"));
                self.check(!marked || matches!(a("shape"), Some("path" | "line")), n, "C65", || {
                    "markers need an open outline: shape=\"path\" or \"line\".".into()
                });
            }
            // p4
            "mask" => {
                self.check(a("type") != Some("path") || has("path"), n, "C4", || {
                    "mask type=\"path\" requires @path.".into()
                });
                self.check(a("type") == Some("path") || (has("width") && has("height")), n, "C5", || {
                    format!("mask type=\"{}\" requires @width and @height.", v("type"))
                });
            }
            "ocean" => {
                self.check(
                    a("splash").is_none_or(|list| {
                        let ids: Vec<_> = list.split_whitespace().collect();
                        let unique: HashSet<_> = ids.iter().copied().collect();
                        !ids.is_empty()
                            && ids.len() == unique.len()
                            && ids.iter().all(|id| self.sets.ejecta_emitters.contains(id))
                    }),
                    n,
                    "OCN13",
                    || "ocean splash must name distinct particles3D emitters that throw out the ejecta of a crater, whose particles have a mass.".into(),
                );
                self.check(a("bodyCoupling").is_none_or(|c| c == "none") || has("colliders"), n, "OCN8", || {
                    "ocean bodyCoupling needs colliders that list the bodies.".into()
                });
                self.check(!has("bodyDrag") || has("colliders"), n, "OCN9", || {
                    "ocean bodyDrag belongs to an ocean with colliders.".into()
                });
                self.check(n.document().root_element().attribute("version") == Some("1.3"), n, "OCN1", || {
                    "ocean requires version=\"1.3\".".into()
                });
                let asset = !has("bathymetry")
                    || (contains(&self.sets.image_assets, a("bathymetry"))
                        && !contains(&self.sets.layered_images, a("bathymetry")))
                    || contains(&self.sets.mesh_assets, a("bathymetry"));
                let encoding = a("bathymetryEncoding").is_none_or(|s| s == "red")
                    || contains(&self.sets.image_assets, a("bathymetry"));
                let options = has("bathymetry")
                    || !(has("bathymetryScale") || has("bathymetryOffset") || has("bathymetryEncoding"));
                self.check(asset && encoding && options && (!has("material") || contains(&self.sets.materials,a("material"))),n,"OCN2",||"ocean bathymetry must name a primary-raster image (no layer selection) or mesh, material must name a material, and bathymetry options require the corresponding asset kind.".into());
                let number = |k, default| a(k).map(xpath_number).unwrap_or(default);
                let cell = number("cellSize", 1.);
                let dims = [number("width", 64.) / cell, number("depth", 64.) / cell];
                let mut valid =
                    dims.iter().all(|v| v.is_finite() && *v >= 1. && (*v - v.round()).abs() <= 1e-9 * v.abs().max(1.))
                        && dims[0].round() * dims[1].round() <= 4_000_000.
                        && number("dt", 1. / 60.) >= 1e-6;
                valid &= ["waterLevel", "bathymetryScale", "bathymetryOffset", "initialVelocityX", "initialVelocityZ"]
                    .iter()
                    .all(|k| number(k, 0.).is_finite());
                let mut waves = 0;
                let mut impulses = 0;
                for c in n.children().filter(|c| c.is_element()) {
                    let v = |k, default| c.attribute(k).map(xpath_number).unwrap_or(default);
                    match c.tag_name().name() {
                        "wave" => {
                            waves += 1;
                            valid &= v("wavelength", 16.) >= 2. * cell
                                && ["direction", "phase"].iter().all(|k| v(k, 0.).is_finite());
                        }
                        "waterImpulse" => {
                            impulses += 1;
                            valid &= (c.attribute("type") != Some("add-water") || v("amplitude", 1.) >= 0.)
                                && ["x", "z", "amplitude", "velocityX", "velocityZ"]
                                    .iter()
                                    .all(|k| v(k, 0.).is_finite());
                        }
                        _ => {}
                    }
                }
                self.check(valid&&waves<=64&&impulses<=16384,n,"OCN3",||"ocean requires finite inputs, integral grid dimensions (at most 4000000 cells), dt >= 0.000001, resolved wavelengths, at most 64 waves/16384 impulses and nonnegative added water.".into());
                self.check(
                    !n.children().any(|c| {
                        c.attribute("property").is_some_and(|k| {
                            !matches!(
                                k,
                                "x" | "y"
                                    | "z"
                                    | "rotation"
                                    | "rotationX"
                                    | "rotationY"
                                    | "scaleX"
                                    | "scaleY"
                                    | "scaleZ"
                                    | "opacity"
                            )
                        })
                    }),
                    n,
                    "OCN4",
                    || "ocean solver configuration is static; animate its pose or opacity instead.".into(),
                );
                let sources: Vec<_> =
                    n.children().filter(|c| c.is_element() && c.tag_name().name() == "whitewater").collect();
                if let Some(list) = a("colliders") {
                    let ids: Vec<_> = list.split_whitespace().collect();
                    let unique: HashSet<_> = ids.iter().copied().collect();
                    let valid = ids.len() <= 4096
                        && ids.len() == unique.len()
                        && ids.iter().all(|id| self.sets.ocean_colliders.contains(id));
                    self.check(valid, n, "OCN6", || "ocean colliders must name at most 4096 distinct objects: a plane or mesh with a crater, or a closed body without one.".into());
                    self.check(!ids.iter().any(|id| self.sets.dynamic_pyro_geometry.contains(id)), n, "OCN7", || "ocean collider geometry is static; animate its position, rotation or scale instead of shape parameters.".into());
                }
                self.check(
                    sources.len() <= 1
                        && sources.iter().all(|s| {
                            let start = s.attribute("start").map(xpath_number).unwrap_or(0.);
                            s.attribute("end").is_none_or(|end| xpath_number(end) >= start)
                                && ["foamMaterial", "sprayMaterial"]
                                    .iter()
                                    .all(|k| s.attribute(*k).is_none_or(|id| self.sets.materials.contains(id)))
                        }),
                    n,
                    "OCN5",
                    || "ocean accepts one whitewater source with end >= start and valid foam/spray materials.".into(),
                );
            }
            "particles3D" => {
                self.check(n.document().root_element().attribute("version") == Some("1.3"), n, "P3D1", || {
                    "particles3D requires version=\"1.3\".".into()
                });
                let mesh = (a("emitterShape") == Some("mesh")) == has("emitterMesh")
                    && (!has("emitterMesh") || contains(&self.sets.mesh_assets, a("emitterMesh")))
                    && (a("shape") == Some("mesh")) == has("mesh")
                    && (!has("mesh") || contains(&self.sets.mesh_assets, a("mesh")))
                    && (!has("sprite")
                        || (a("shape") == Some("billboard") && contains(&self.sets.image_assets, a("sprite"))))
                    && (!has("material") || contains(&self.sets.materials, a("material")));
                self.check(mesh,n,"P3D2",|| "particles3D mesh, emitterMesh, sprite and material must name the appropriate assets and match the selected shapes.".into());
                let number = |k, default| a(k).map(xpath_number).unwrap_or(default);
                let start = number("emissionStart", 0.);
                let direction = [number("directionX", 0.), number("directionY", -1.), number("directionZ", 0.)];
                let valid = number("lifetimeVariance", 0.) < number("lifetime", 2.)
                    && number("speedVariance", 0.) <= number("speed", 0.)
                    && direction.iter().all(|v| v.is_finite())
                    && direction.iter().any(|v| *v != 0.)
                    && (!has("emissionEnd") || number("emissionEnd", 0.) >= start)
                    && kids(n, "burst").all(|b| {
                        b.has_attribute("crater") || b.attribute("time").map(xpath_number).is_some_and(|t| t >= start)
                    });
                self.check(valid,n,"P3D3",|| "particles3D requires bounded variances, a nonzero direction, an ordered emission window and bursts at or after emissionStart.".into());
                let dynamic = [
                    "x",
                    "y",
                    "z",
                    "rotation",
                    "rotationX",
                    "rotationY",
                    "scaleX",
                    "scaleY",
                    "scaleZ",
                    "rate",
                    "opacity",
                    "size",
                    "sizeEnd",
                    "color",
                    "colorEnd",
                    "opacityEnd",
                    "trail",
                ];
                self.check(
                    !n.children().any(|c| c.attribute("property").is_some_and(|p| !dynamic.contains(&p))),
                    n,
                    "P3D4",
                    || "particles3D solver configuration is static; animate pose, rate or appearance instead.".into(),
                );
                self.check(a("gas").is_none_or(|id| self.sets.pyro_volumes.contains(id)), n, "P3D11", || {
                    "particles3D gas must name an object3D that holds a native pyro volume; a baked or asset volume has no velocity grid."
                        .into()
                });
                if let Some(list) = a("colliders") {
                    let ids: Vec<_> = list.split_whitespace().collect();
                    let unique: HashSet<_> = ids.iter().copied().collect();
                    self.check(
                        ids.len() <= 4096
                            && ids.len() == unique.len()
                            && ids.iter().all(|id| self.sets.pyro_colliders.contains(id)),
                        n,
                        "P3D5",
                        || {
                            "particles3D colliders must name at most 4096 distinct supported rigid surface objects."
                                .into()
                        },
                    );
                    self.check(!ids.iter().any(|id| self.sets.dynamic_pyro_geometry.contains(id)), n, "P3D6", || {
                        "particles3D collider geometry must be static; animate its rigid pose instead.".into()
                    });
                }
            }
            "pyro" => {
                if let Some(list) = a("colliders") {
                    let ids: Vec<_> = list.split_whitespace().collect();
                    let unique: HashSet<_> = ids.iter().copied().collect();
                    let valid = ids.len() <= 4096
                        && ids.len() == unique.len()
                        && ids.iter().all(|id| self.sets.pyro_colliders.contains(id));
                    self.check(valid, n, "PYRO7", || "pyro colliders must name at most 4096 distinct supported surface objects, including text, extrude and clay solids.".into());
                    self.check(!ids.iter().any(|id| self.sets.dynamic_pyro_geometry.contains(id)), n, "PYRO8", || "pyro collider geometry is static; animate its position, rotation or scale instead of shape parameters.".into());
                }
                let h = a("voxelSize").and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0);
                let grid = ["width", "height", "depth"].iter().all(|name| {
                    let cells = a(name).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0) / h;
                    cells.is_finite() && (2.0..=1024.0).contains(&cells) && (cells - cells.round()).abs() <= 1e-8
                });
                self.check(grid, n, "PYRO1", || {
                    "pyro dimensions must be integer multiples of voxelSize, with 2..1024 cells per axis.".into()
                });
            }
            "burst" if n.parent_element().is_some_and(|p| is(p, "particles3D")) => {
                let from_crater = has("crater");
                self.check(
                    if from_crater { !(has("time") || has("repeat") || has("interval")) } else { has("time") },
                    n,
                    "P3D7",
                    || "a burst requires time unless it comes from a crater, and a burst from a crater derives its instants, so time, repeat and interval may not be given.".into(),
                );
                self.check(
                    a("crater").is_none_or(|id| {
                        n.document().descendants().any(|c| {
                            is(c, "crater") && c.attribute("id") == Some(id) && c.attribute("source").is_some()
                        })
                    }),
                    n,
                    "P3D8",
                    || "a burst from a crater must name a crater that grows from an impact.".into(),
                );
                self.check(from_crater || !(has("angle") || has("angleSpread")), n, "P3D9", || {
                    "angle and angleSpread belong to a burst from a crater.".into()
                });
                let degrees = |k: &str, default: f64| a(k).map(xpath_number).unwrap_or(default);
                let (angle, spread) = (degrees("angle", 45.), degrees("angleSpread", 15.));
                self.check(!from_crater || (angle - spread >= 0. && angle + spread <= 90.), n, "P3D10", || {
                    "the launch angle of a burst from a crater (default 45) and its spread (default 15) must stay between 0 and 90 degrees."
                        .into()
                });
            }
            "waterImpulse" if n.parent_element().is_some_and(|p| is(p, "ocean")) => {
                let source = a("source");
                self.check(
                    source.is_none()
                        || !["time", "x", "z", "radius", "amplitude", "velocityX", "velocityZ", "type"]
                            .iter()
                            .any(|k| has(k)),
                    n,
                    "OCN10",
                    || "a water impulse from a body derives its instant, place, size and shape, so time, x, z, radius, amplitude, velocityX, velocityZ and type may not be given.".into(),
                );
                let ocean = n.parent_element();
                self.check(
                    source.is_none_or(|id| {
                        ocean.and_then(|o| o.attribute("colliders")).is_some_and(|l| l.split_whitespace().any(|c| c == id))
                            && n.document().descendants().any(|o| {
                                is(o, "object3D")
                                    && o.attribute("id") == Some(id)
                                    && !kids(o, "crater").any(|_| true)
                                    && kids(o, "rigidBody").any(|b| matches!(b.attribute("type"), None | Some("dynamic")))
                            })
                    }),
                    n,
                    "OCN11",
                    || "the source of a water impulse must be a dynamic rigid body, without a crater, that the ocean lists in colliders.".into(),
                );
                self.check(
                    source.is_none_or(|id| {
                        ocean.is_some_and(|o| {
                            kids(o, "waterImpulse").filter(|w| w.attribute("source") == Some(id)).count() == 1
                        })
                    }),
                    n,
                    "OCN12",
                    || "a body makes one cavity: at most one water impulse names it.".into(),
                );
            }
            "pyroSource" | "pyroImpulse" => {
                self.pyro_source_near_open_face(n);
                let mesh = if a("shape") == Some("mesh") {
                    has("mesh") && contains(&self.sets.mesh_assets, a("mesh"))
                } else {
                    !has("mesh")
                };
                self.check(mesh, n, "PYRO5", || {
                    "a mesh pyro source must name a mesh asset; @mesh is valid only for shape=mesh.".into()
                });
                self.check(
                    !n.children().any(|c| matches!(c.attribute("property"), Some("shape" | "mesh"))),
                    n,
                    "PYRO6",
                    || "pyro shape and mesh selection are static; animate the source transform instead.".into(),
                );
                let start = a("start").and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0);
                let end = a("end").and_then(|v| v.trim().parse::<f64>().ok());
                self.check(end.is_none_or(|end| end > start), n, "PYRO2", || {
                    "pyro source end must be greater than start.".into()
                });
                self.check(
                    a("shape") != Some("box") || ["width", "height", "depth"].iter().all(|k| has(k)),
                    n,
                    "PYRO3",
                    || "box pyro sources require width, height and depth.".into(),
                );
                self.check(
                    ["scaleX", "scaleY", "scaleZ"]
                        .iter()
                        .all(|k| a(k).and_then(|v| v.trim().parse::<f64>().ok()).is_none_or(|v| v != 0.0)),
                    n,
                    "PYRO4",
                    || "pyro source transforms must be invertible (nonzero scales).".into(),
                );
                const DERIVED: [&str; 22] = [
                    "start",
                    "end",
                    "time",
                    "densityRate",
                    "temperatureRate",
                    "density",
                    "temperature",
                    "expansion",
                    "shape",
                    "mesh",
                    "radius",
                    "width",
                    "height",
                    "depth",
                    "x",
                    "y",
                    "z",
                    "rotation",
                    "rotationX",
                    "rotationY",
                    "scaleX",
                    "scaleY",
                ];
                self.check(
                    !has("crater") || !(DERIVED.iter().any(|k| has(k)) || has("scaleZ")),
                    n,
                    "PYC1",
                    || "a pyro source or impulse from a crater derives its shape, place, timing, density, temperature and expansion, so none of them may be given.".into(),
                );
                self.check(
                    has("crater") || !["heatFraction", "dustFraction", "specificHeat", "maxTemperature"].iter().any(|k| has(k)),
                    n,
                    "PYC2",
                    || "heatFraction, dustFraction, specificHeat and maxTemperature belong to a pyro source or impulse from a crater.".into(),
                );
                self.check(n.tag_name().name() != "pyroImpulse" || has("time") || has("crater"), n, "PYC3", || {
                    "a pyro impulse requires time unless it comes from a crater.".into()
                });
                self.check(
                    a("crater").is_none_or(|id| {
                        n.document().descendants().any(|c| {
                            is(c, "crater") && c.attribute("id") == Some(id) && c.attribute("source").is_some()
                        })
                    }),
                    n,
                    "PYC4",
                    || "a pyro source or impulse from a crater must name a crater that grows from an impact.".into(),
                );
            }
            // p5, p26
            "object3D" => {
                let static_inputs = a("primitive") != Some("globe")
                    || !n.children().any(|c| {
                        matches!(
                            c.attribute("property"),
                            Some(
                                "terrain"
                                    | "terrainEncoding"
                                    | "planetRadius"
                                    | "terrainTileSize"
                                    | "terrainZoom"
                                    | "terrainMissing"
                                    | "terrainMemoryMiB"
                            )
                        )
                    });
                let static_collider = !(a("primitive") == Some("globe")
                    && has("terrain")
                    && kids(n, "rigidBody").next().is_some()
                    && n.children()
                        .any(|c| matches!(c.attribute("property"), Some("radius" | "segments" | "exaggeration"))));
                self.check(static_inputs && static_collider,n,"GEO3",||"globe terrain sampling and planetRadius are static; globe rigid collider relief cannot animate radius, segments or exaggeration.".into());

                let options = ["terrainTileSize", "terrainZoom", "terrainMissing", "terrainMemoryMiB"];
                let feature = (a("primitive") == Some("globe") && has("terrain"))
                    || has("planetRadius")
                    || options.iter().any(|k| has(k));
                self.check(
                    !feature || n.document().root_element().attribute("version") == Some("1.3"),
                    n,
                    "GEO1",
                    || "globe elevation and its physical-scale/budget options require version=\"1.3\".".into(),
                );
                let size = a("terrainTileSize").and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(256);
                self.check((!has("planetRadius") || a("primitive") == Some("globe"))
                    && (!options.iter().any(|k| has(k)) || (a("primitive") == Some("globe") && has("terrain")))
                    && size.is_power_of_two(), n, "GEO2", || "planetRadius belongs to globes; terrain sampling options require a globe with terrain and a power-of-two tile size.".into());

                let pyro_count = kids(n, "pyro").count();
                self.check(
                    a("primitive") != Some("volume") || (usize::from(has("volume")) + pyro_count == 1),
                    n,
                    "VOL1",
                    || "volume primitive requires exactly one source: @volume or a pyro child.".into(),
                );
                let target = !has("volume") || contains(&self.sets.volume_assets, a("volume"));
                self.check(target, n, "VOL2", || "object3D/@volume must name a volume asset.".into());
                let thermal = kids(n, "medium").any(|m| matches!(m.attribute("blackbody"), Some("true" | "1")));
                let temperature = n
                    .document()
                    .root_element()
                    .children()
                    .filter(|c| c.has_tag_name("assets"))
                    .flat_map(|c| c.children())
                    .any(|c| {
                        c.has_tag_name("volume")
                            && c.attribute("id") == a("volume")
                            && c.attribute("temperatureGrid").is_some()
                    });
                self.check(!thermal || temperature || pyro_count == 1, n, "VOL5", || {
                    "blackbody emission requires a declared temperatureGrid on the volume asset.".into()
                });
                let medium_count = kids(n, "medium").count();
                self.check(
                    medium_count <= 1
                        && (a("primitive") == Some("volume")
                            || (medium_count == 0 && pyro_count == 0 && !has("volume"))),
                    n,
                    "VOL3",
                    || "one medium child and @volume are permitted only on a volume primitive.".into(),
                );
                let grid_only = kids(n, "medium").all(|m| {
                    m.attribute("lighting") == Some("grid")
                        || ["lightGridCell", "lightGridDomeDirections", "lightGridMemoryMiB"]
                            .iter()
                            .all(|k| m.attribute(*k).is_none())
                });
                self.check(grid_only, n, "VOL10", || {
                    "the lightGrid* attributes of a medium apply only with lighting=\"grid\".".into()
                });
                self.check(a("primitive") != Some("mesh") || has("mesh"), n, "C6", || {
                    "object3D primitive=\"mesh\" requires @mesh.".into()
                });
                self.check(a("primitive") != Some("text") || has("text"), n, "C7", || {
                    "object3D primitive=\"text\" requires @text.".into()
                });
                self.check(a("primitive") != Some("extrude") || has("path"), n, "C8", || {
                    "object3D primitive=\"extrude\" requires @path.".into()
                });
                let c47 = !matches!(a("primitive"), Some("map" | "globe")) || has("map");
                self.check(c47, n, "C47", || "object3D primitive=\"map\" or \"globe\" requires @map.".into());
                let r28 = !has("map") || self.sets.map_assets.contains(&v("map").as_str());
                self.check(r28, n, "R28", || "object3D/@map must name a map asset.".into());
                let r29 = !has("terrain") || self.sets.tiles_assets.contains(&v("terrain").as_str());
                self.check(r29, n, "R29", || "object3D/@terrain must name a tiles asset.".into());
                let r4 = !has("material") || contains(&self.sets.materials, a("material"));
                self.check(r4, n, "R4", || "object3D/@material must name a material.".into());
                let r5 = !has("mesh")
                    || contains(&self.sets.mesh_assets, a("mesh"))
                    || contains(&self.sets.mesh_sequences, a("mesh"));
                self.check(r5, n, "R5", || "object3D/@mesh must name a mesh or meshSequence asset.".into());
            }
            "meshSequence" if parent_is("assets") => {
                self.check(n.document().root_element().attribute("version") == Some("1.3"), n, "MSQ1", || {
                    "mesh sequences require version=\"1.3\".".into()
                });
                let first = a("first").and_then(|v| v.trim().parse::<i64>().ok());
                let last = a("last").and_then(|v| v.trim().parse::<i64>().ok());
                let range = first.zip(last).is_some_and(|(first, last)| {
                    last.checked_sub(first).is_some_and(|n| (0..1_000_000).contains(&n))
                        && a("src").is_some_and(|s| crate::assets::sequence_frame(s, first).is_some())
                });
                self.check(range, n, "MSQ2", || {
                    "mesh sequences require 1..1000000 ordered frames and a numbered src pattern.".into()
                });
                self.check(!has("sha256"), n, "MSQ3", || {
                    "mesh sequence sha256 cannot identify multiple numbered files.".into()
                });
            }
            // p42
            "volume" if parent_is("assets") => {
                let baked = a("format") == Some("srvseq");
                let velocities = ["velocityGridX", "velocityGridY", "velocityGridZ"].iter().filter(|k| has(k)).count();
                self.check(
                    if a("interpolation") == Some("advect") { velocities == 3 } else { velocities == 0 },
                    n,
                    "VOL9",
                    || {
                        "advect requires all three velocity channels; velocity channels require advect interpolation."
                            .into()
                    },
                );
                self.check(!baked || (has("sha256") && !has("first") && !has("last") && !has("fps") && a("missingFrame").is_none_or(|s|s=="error")),n,"VOL8",|| {
                    "srvseq requires sha256 and owns its frame range and composition clock; first, last, fps and missing-frame substitution are not permitted.".into()
                });
                let sequence = has("first") || has("last");
                let first = a("first").and_then(|s| s.trim().parse::<i64>().ok());
                let last = a("last").and_then(|s| s.trim().parse::<i64>().ok());
                let range = first.zip(last).is_some_and(|(first, last)| {
                    last.checked_sub(first).is_some_and(|n| (0..1_000_000).contains(&n))
                        && a("src").is_some_and(|s| crate::assets::sequence_frame(s, first).is_some())
                });
                self.check(baked || if sequence { range } else { !has("fps") && a("interpolation").is_none_or(|s| s=="hold") && a("missingFrame").is_none_or(|s| s=="error") }, n,"VOL6",|| {
                    "volume sequences require first/last, 1..1000000 ordered frames and a numbered src pattern; sequence options require a sequence.".into()
                });
                self.check(!sequence || !has("sha256"), n, "VOL7", || {
                    "volume sha256 applies to a single cache file, not a numbered sequence.".into()
                });
                let bounds = ["boundsMinX", "boundsMinY", "boundsMinZ", "boundsMaxX", "boundsMaxY", "boundsMaxZ"];
                let count = bounds.iter().filter(|k| has(k)).count();
                let ordered = count == 6
                    && (0..3).all(|i| {
                        let lo = a(bounds[i]).and_then(|s| s.parse::<f64>().ok());
                        let hi = a(bounds[i + 3]).and_then(|s| s.parse::<f64>().ok());
                        matches!((lo, hi), (Some(lo), Some(hi)) if lo.is_finite() && hi.is_finite() && lo < hi)
                    });
                self.check(count == 0 || ordered, n, "VOL4", || {
                    "volume bounds require all six finite coordinates with each minimum below its maximum.".into()
                });
            }
            // p42
            "geoLayer" => {
                let r36 = self.sets.geo_assets.contains(&v("geo").as_str());
                self.check(r36, n, "R36", || "geoLayer/@geo must name a geo asset.".into());
            }
            "route" => {
                let r37 = !has("geo") || self.sets.geo_assets.contains(&v("geo").as_str());
                self.check(r37, n, "R37", || "route/@geo must name a geo asset.".into());
                self.check(has("points") || has("geo"), n, "C45", || "route needs @points or @geo.".into());
            }
            "basemap" => {
                let r27 = self.sets.tiles_assets.contains(&v("tiles").as_str());
                self.check(r27, n, "R27", || "basemap/@tiles must name a tiles asset.".into());
            }
            // p43
            "rigidBody" if parent_is("object3D") => {
                let ok = a("shape") != Some("trimesh") || matches!(a("type"), Some("static" | "kinematic"));
                self.check(ok, n, "C48", || "a trimesh rigidBody must be static or kinematic.".into());
            }
            "tiles" => {
                let ok = has("src") || (has("url") && has("cache") && has("cacheSha256"));
                self.check(ok, n, "C46", || "tiles need @src, or @url with @cache and @cacheSha256.".into());
            }
            "map" => {
                let r26 = a("fit").is_none_or(|list| every_token_names(list, &self.sets.geo_assets));
                self.check(r26, n, "R26", || "every id in map/@fit must name a geo asset.".into());
            }
            // p6, p7
            "constraint" => {
                if a("type") == Some("pin") {
                    self.check(!has("b"), n, "C9", || {
                        "pin constraints tie body a to a world point; @b is not allowed.".into()
                    });
                    self.check(has("x") && has("y"), n, "C10", || "pin constraints require @x and @y.".into());
                } else if has("type") {
                    self.check(has("b"), n, "C11", || format!("constraint type=\"{}\" requires @b.", v("type")));
                }
            }
            // p8
            "text" if parent_is("assets") => {
                let span = has_kid(n, "span");
                self.check((has("text") && !span) || (!has("text") && span), n, "C12", || {
                    format!("text asset \"{}\" needs exactly one of @text or span children.", v("id"))
                });
                let c13 = !(has("minSize") && has("maxSize")) || num(n, "minSize") <= num(n, "maxSize");
                self.check(c13, n, "C13", || "minSize must not exceed maxSize.".into());
                let (ls, size) = (num(n, "letterSpacing"), num(n, "size"));
                let c14 = !has("letterSpacing") || (ls >= -size && ls <= 4.0 * size);
                self.check(c14, n, "C14", || "letterSpacing must lie in [-size, 4 x size].".into());
            }
            // p11
            "repeat" => {
                self.check(has("count") != has("over"), n, "C17", || {
                    "repeat needs exactly one of @count or @over.".into()
                });
                let c18 =
                    !has("over") || contains(&self.sets.data, a("over")) || contains(&self.sets.list_params, a("over"));
                self.check(c18, n, "C18", || "repeat/@over must name a data source or a list parameter.".into());
            }
            // p13
            "transition" => {
                self.check(has("from") || has("to") || parent_is("segment"), n, "C20", || {
                    "transition needs @from, @to or both.".into()
                });
                self.check(a("type") != Some("shader") || has("shader"), n, "C21", || {
                    "transition type=\"shader\" requires @shader.".into()
                });
                self.check(a("type") != Some("luma") || has("matte"), n, "C22", || {
                    "transition type=\"luma\" requires @matte.".into()
                });
                if parent_is("segment") {
                    let c59 = !(has("from") || has("to") || matches!(a("type"), Some("morph" | "luma")));
                    self.check(c59, n, "C59", || {
                        "a segment transition joins two rendered pictures: no from, no to, not morph and not luma."
                            .into()
                    });
                }
                let siblings = |id: &str| match n.parent() {
                    Some(p) => p.children().filter(|c| c.is_element() && c.attribute("id") == Some(id)).count(),
                    None => 0,
                };
                let c23 = a("from").is_none_or(|f| siblings(f) == 1);
                self.check(c23, n, "C23", || "transition/@from must be a sibling of the transition.".into());
                let c24 = a("to").is_none_or(|t| siblings(t) == 1);
                self.check(c24, n, "C24", || "transition/@to must be a sibling of the transition.".into());
            }
            // p14
            "modifier" => {
                self.check(a("type") != Some("skin") || has("skeleton"), n, "C25", || {
                    "modifier type=\"skin\" requires @skeleton.".into()
                });
                let c26 = a("type") != Some("corner-pin") || tokens(a("corners").unwrap_or("")) == 8;
                self.check(c26, n, "C26", || "modifier type=\"corner-pin\" requires 8 numbers in @corners.".into());
            }
            // p15
            "transformConstraint" => {
                self.check(a("type") != Some("follow-path") || has("path"), n, "C27", || {
                    "follow-path requires @path.".into()
                });
                self.check(a("type") == Some("follow-path") || has("target"), n, "C28", || {
                    format!("transformConstraint type=\"{}\" requires @target.", v("type"))
                });
                let c29 = a("type") != Some("track") || contains(&self.sets.track_data, a("target"));
                self.check(c29, n, "C29", || "track constraints target trackData.".into());
            }
            // p16
            "safeArea" if a("preset").is_none_or(|p| p == "custom") => {
                let c30 = has("top") && has("right") && has("bottom") && has("left");
                self.check(c30, n, "C30", || "custom safe areas require top, right, bottom and left.".into());
            }
            // p18
            "audioMix" => {
                self.check(kids(n, "master").count() <= 1, n, "C34", || "audioMix allows at most one master.".into());
                self.check(has_kid(n, "audioTrack"), n, "C35", || "audioMix needs at least one audioTrack.".into());
            }
            // p19
            "linearGradient" | "radialGradient" | "conicGradient" => {
                self.check(kids(n, "stop").count() >= 2, n, "C36", || {
                    format!("gradient \"{}\" needs at least two stops.", v("id"))
                });
                self.check(non_decreasing(n, "stop", "offset"), n, "C37", || {
                    "gradient stop offsets must be non-decreasing.".into()
                });
            }
            // p21, p22
            "key" => {
                if a("interpolation") == Some("steps") {
                    self.check(has("steps"), n, "C39", || "interpolation=\"steps\" requires @steps.".into());
                }
                if a("interpolation") == Some("cubic-bezier") {
                    let next_key = n.next_siblings().skip(1).find(|s| is(*s, "key"));
                    let ok =
                        has("bezier") || has("easeOut") || next_key.is_some_and(|k| k.attribute("easeIn").is_some());
                    self.check(ok, n, "C40", || "cubic-bezier keys need @bezier or easeOut/easeIn handles.".into());
                }
            }
            // p23
            "generated" => {
                self.check(has("prompt") || a("kind") == Some("speech"), n, "C41", || {
                    "generated media other than speech requires @prompt.".into()
                });
            }
            // p61
            "segment" => {
                let c55 =
                    has_kid(n, "timeRemap") || ((has("from") || has("fromMarker")) && (has("to") || has("toMarker")));
                self.check(c55, n, "C55", || {
                    "a segment needs from (or fromMarker) and to (or toMarker), or a timeRemap.".into()
                });
                let duration = n
                    .ancestors()
                    .find(|x| is(*x, "scene"))
                    .and_then(|sc| kids(sc, "project").next())
                    .and_then(|pr| pr.attribute("duration"))
                    .map(xpath_number)
                    .unwrap_or(f64::NAN);
                let (from, to) = (num(n, "from"), num(n, "to"));
                let c56 = !(has("from") && has("to")) || (from >= 0.0 && to > from && to <= duration);
                self.check(c56, n, "C56", || {
                    "segment from and to must satisfy 0 <= from < to <= project/@duration.".into()
                });
                let c57 = !(has("from") && has("fromMarker") || has("to") && has("toMarker"));
                self.check(c57, n, "C57", || "a segment gives each end as a time or as a marker, not both.".into());
                let c58 = kids(n, "timeRemap").count() <= 1 && kids(n, "transition").count() <= 1;
                self.check(c58, n, "C58", || "a segment has at most one timeRemap and one transition.".into());
                let r38 = (!has("fromMarker") || contains(&self.sets.markers, a("fromMarker")))
                    && (!has("toMarker") || contains(&self.sets.markers, a("toMarker")));
                self.check(r38, n, "R38", || "segment markers must name markers.".into());
            }
            // p24, p32
            "output" => {
                self.check(!has("proresProfile") || a("codec") == Some("prores"), n, "C42", || {
                    "proresProfile applies only to codec=\"prores\".".into()
                });
                let c43 = a("alpha") != Some("true") || a("codec").is_some_and(|c| ALPHA_CODECS.contains(&c));
                self.check(c43, n, "C43", || "alpha=\"true\" needs a codec that carries alpha.".into());
                let c44 = !has("end") || (!has("start") && num(n, "end") > 0.0) || num(n, "end") > num(n, "start");
                self.check(c44, n, "C44", || "output end must be after start.".into());
                // p61
                // a start of 0 is the default, which validators that fill in defaults supply
                let c54 = !has_kid(n, "segment") || ((!has("start") || num(n, "start") == 0.0) && !has("end"));
                self.check(c54, n, "C54", || {
                    "an output with segments cannot also set start or end; put the range in a segment instead.".into()
                });
                let tracks_ok =
                    a("audioTracks").is_none_or(|l| l.split_whitespace().all(|t| self.sets.mix_tracks.contains(&t)));
                let buses_ok =
                    a("audioBuses").is_none_or(|l| l.split_whitespace().all(|b| self.sets.buses.contains(b)));
                self.check(tracks_ok && buses_ok, n, "R39", || {
                    "every id in output/@audioTracks must name an audioMix track, and every id in output/@audioBuses a bus."
                        .into()
                });
                self.check(!has("overlay") || contains(&self.sets.symbols, a("overlay")), n, "R40", || {
                    "output/@overlay must name a symbol.".into()
                });
                self.check(!has("layout") || contains(&self.sets.layouts, a("layout")), n, "R12", || {
                    "output/@layout must name a layout.".into()
                });
                self.check(!has("variant") || contains(&self.sets.variants, a("variant")), n, "R13", || {
                    "output/@variant must name a variant.".into()
                });
                let own_caption = |id: &str| kids(n, "captionTrack").any(|c| c.attribute("id") == Some(id));
                let r14 = !has("burnCaptions")
                    || contains(&self.sets.caption_tracks, a("burnCaptions"))
                    || a("burnCaptions").is_some_and(own_caption);
                self.check(r14, n, "R14", || "output/@burnCaptions must name a captionTrack.".into());
            }
            // p31
            "instance" => {
                self.check(contains(&self.sets.symbols, a("symbol")), n, "R11", || {
                    "instance/@symbol must name a symbol.".into()
                });
            }
            // p33
            "audioTrack" => {
                let r15 = contains(&self.sets.audio_assets, a("asset"))
                    || contains(&self.sets.generated_audio, a("asset"))
                    || contains(&self.sets.video_with_audio, a("asset"));
                self.check(r15, n, "R15", || {
                    "audioTrack/@asset must name audio, generated audio, or video with hasAudio=\"true\".".into()
                });
                self.check(!has("bus") || contains(&self.sets.buses, a("bus")), n, "R16", || {
                    "audioTrack/@bus must name a bus.".into()
                });
                let own = format!(" {} ", v("id"));
                let r17 = a("duckUnder").is_none_or(|d| !format!(" {} ", normalize_space(d)).contains(&own));
                self.check(r17, n, "R17", || "a track cannot duck under itself.".into());
            }
            // p34
            "bind" => {
                self.check(contains(&self.sets.params, a("param")), n, "R18", || {
                    "bind/@param must name a param.".into()
                });
            }
            // p36
            "camera" if has("focusTarget") || has("target") => {
                let ok = (!has("focusTarget") || contains(&self.sets.composition_desc, a("focusTarget")))
                    && (!has("target") || contains(&self.sets.composition_desc, a("target")));
                self.check(ok, n, "R20", || "camera targets must name composition nodes.".into());
            }
            _ => {}
        }

        self.black_hole(n, local);

        // p9, p25 — layer
        if local == "layer" {
            if has_kid(n, "timeRemap") {
                let c15 = (!has("speed") || num(n, "speed") == 1.0)
                    && a("reverse").is_none_or(|r| r == "false")
                    && a("loop").is_none_or(|l| l == "0");
                self.check(c15, n, "C15", || {
                    format!("layer \"{}\": timeRemap replaces speed, reverse and loop; remove them.", v("id"))
                });
            }
            self.check(contains(&self.sets.assets, a("asset")), n, "R1", || {
                format!("layer \"{}\": @asset must reference an element of assets.", v("id"))
            });
            let r2 =
                !(has_kid(n, "textAnimator") || has_kid(n, "textPath")) || contains(&self.sets.text_assets, a("asset"));
            self.check(r2, n, "R2", || "textAnimator and textPath need a text asset.".into());
            self.check(!has("audioBus") || contains(&self.sets.buses, a("audioBus")), n, "R3", || {
                "layer/@audioBus must name a bus.".into()
            });
        }
        // p10
        if (local == "layer" || local == "instance") && a("fit").is_some_and(|f| f != "none") {
            self.check(has("boxWidth") && has("boxHeight"), n, "C16", || {
                format!("fit=\"{}\" requires @boxWidth and @boxHeight.", v("fit"))
            });
        }
        // p12
        if local == "effect" && matches!(a("type"), Some("lut" | "shader" | "displacement-map" | "gradient-map")) {
            self.check(has("src") || has("source") || has("paint"), n, "C19", || {
                format!("effect type=\"{}\" requires @src, @source or @paint.", v("type"))
            });
        }
        // p17
        if local == "captionTrack" {
            let sources = usize::from(has_kid(n, "cue")) + usize::from(has("src")) + usize::from(has("transcribe"));
            self.check(sources == 1, n, "C31", || {
                format!("captionTrack \"{}\" needs exactly one source: cue children, @src or @transcribe.", v("id"))
            });
            self.check(!has("transcribe") || (has("cache") && has("cacheSha256")), n, "C32", || {
                "transcribed captions require @cache and @cacheSha256 (deterministic renders).".into()
            });
            let c33 = !has("transcribe") || parent_is("output") || contains(&self.sets.audio_tracks, a("transcribe"));
            self.check(c33, n, "C33", || "captionTrack/@transcribe must name an audioTrack.".into());
            if has("transcribe") && parent_is("output") {
                let r41 = n
                    .parent_element()
                    .is_some_and(|o| kids(o, "audioTrack").any(|t| t.attribute("id") == a("transcribe")));
                self.check(r41, n, "R41", || {
                    "an output caption track transcribes one of that output's own audio tracks.".into()
                });
            }
        }
        // p20
        if local == "animate" || local == "timeRemap" {
            self.check(non_decreasing(n, "key", "time"), n, "C38", || "key times must be non-decreasing.".into());
        }
        // p27
        if let Some(list) = a("effects") {
            let ok = every_token_names(list, &self.sets.effects);
            self.check(ok, n, "R6", || format!("every id in @effects of \"{}\" must name an effect.", v("id")));
        }
        // p28
        if local == "effect" {
            if let Some(list) = a("lights") {
                let ok = every_token_names(list, &self.sets.lights);
                self.check(ok, n, "R7", || "every id in effect/@lights must name a light.".into());
            }
        }
        // p29
        if has("matte") && local != "transition" {
            let r8 = contains(&self.sets.composition_desc, a("matte")) || contains(&self.sets.symbols_desc, a("matte"));
            self.check(r8, n, "R8", || "@matte must name a composition node.".into());
            self.check(has("id") && a("matte") != a("id"), n, "R9", || "a node cannot be its own matte.".into());
        }
        // p30
        if has("parent") && has("id") {
            self.check(a("parent") != a("id"), n, "R10", || "a node cannot parent itself.".into());
        }
        // p35
        if matches!(local, "project" | "layout" | "captionTrack") && has("safeArea") {
            self.check(contains(&self.sets.safe_areas, a("safeArea")), n, "R19", || {
                "@safeArea must name a safeArea.".into()
            });
        }
        // p37
        if has("startMarker") || has("endMarker") || (local == "key" && has("marker")) {
            let m = &self.sets.markers;
            let ok = (!has("startMarker") || contains(m, a("startMarker")))
                && (!has("endMarker") || contains(m, a("endMarker")))
                && (!has("marker") || contains(m, a("marker")));
            self.check(ok, n, "R21", || "marker references must name markers.".into());
        }
        // p38
        if (local == "textStyle" && has("basedOn")) || (has("style") && local != "audiogram") {
            let ts = &self.sets.text_styles;
            let ok = (!has("basedOn") || contains(ts, a("basedOn"))) && (!has("style") || contains(ts, a("style")));
            self.check(ok, n, "R22", || "text style references must name textStyle elements.".into());
        }
        // p39
        if has("fontAsset") {
            self.check(contains(&self.sets.font_assets, a("fontAsset")), n, "R23", || {
                "@fontAsset must name a font asset.".into()
            });
        }
        // p40, p41
        if PAINT_ATTRS.iter().any(|p| has(p)) {
            for attr in PAINT_ATTRS {
                let val = v(attr);
                if val.starts_with("url(#") {
                    let ok = self.sets.paints.contains(between(&val, "url(#").as_str());
                    self.check(ok, n, &format!("R24-{attr}"), || {
                        format!("@{attr}: url(#id) must name an element of paints.")
                    });
                }
            }
            for attr in PAINT_ATTRS {
                let val = v(attr);
                if val.starts_with("var(--") {
                    let ok = self.sets.token_names.contains(between(&val, "var(--").as_str());
                    self.check(ok, n, &format!("R25-{attr}"), || {
                        format!("@{attr}: var(--name) must name a style token.")
                    });
                }
            }
        }
        // p51
        if PAINT_REF_ATTRS.iter().any(|p| has(p)) {
            for attr in PAINT_REF_ATTRS {
                let val = v(attr);
                if val.starts_with("url(#") {
                    let ok = self.sets.paints.contains(between(&val, "url(#").as_str());
                    self.check(ok, n, &format!("R30-{attr}"), || {
                        format!("@{attr}: url(#id) must name an element of paints.")
                    });
                }
            }
        }
        // p52
        if TOKEN_REF_ATTRS.iter().any(|p| has(p)) {
            for attr in TOKEN_REF_ATTRS {
                let val = v(attr);
                if val.starts_with("var(--") {
                    let ok = self.sets.token_names.contains(between(&val, "var(--").as_str());
                    self.check(ok, n, &format!("R31-{attr}"), || {
                        format!("@{attr}: var(--name) must name a styles/token.")
                    });
                }
            }
        }
        let flock_or_emitter = matches!(local, "flock" | "particleEmitter");
        // p53
        if flock_or_emitter && a("shape") == Some("sprite") {
            self.check(has("sprite"), n, "C50", || "shape=\"sprite\" requires @sprite.".into());
        }
        // p54
        if flock_or_emitter && has("sprite") {
            self.check(contains(&self.sets.sprite_assets, a("sprite")), n, "R32", || {
                "@sprite must name an image, image sequence, video or generator asset.".into()
            });
        }
        // p55
        if local == "erosion" && has("heightmap") {
            self.check(contains(&self.sets.image_assets, a("heightmap")), n, "R33", || {
                "erosion/@heightmap must name an image asset.".into()
            });
        }
        // p62
        if local == "pattern" && has("asset") {
            self.check(contains(&self.sets.image_assets, a("asset")), n, "R42", || {
                "pattern/@asset must name an image asset: a pattern tiles an image.".into()
            });
        }
        // p63
        if has("emitterAsset") {
            self.check(contains(&self.sets.image_assets, a("emitterAsset")), n, "R43", || {
                "@emitterAsset must name an image asset: particles are emitted from its opaque pixels.".into()
            });
        }
        // p65
        if local == "generator" && has("lineWidth") {
            self.check(a("kind") == Some("grid"), n, "R45", || {
                "@lineWidth is the line width of a grid generator.".into()
            });
        }
        // p64
        if local == "effect"
            && has("source")
            && matches!(a("type"), Some("displacement-map" | "difference-key" | "shader"))
        {
            let ok =
                contains(&self.sets.composition_desc, a("source")) || contains(&self.sets.symbols_desc, a("source"));
            self.check(ok, n, "R44", || {
                "effect @source must name a composition node; an asset is placed on a (hidden) layer, and the layer named.".into()
            });
        }
        // p56
        if let Some(list) = a("forceFields") {
            let ok = every_token_names(list, &self.sets.force_fields);
            self.check(ok, n, "R34", || {
                format!("every id in @forceFields of \"{}\" must name a physics/forceField.", v("id"))
            });
        }
        // p57
        if has("textStyle") {
            self.check(contains(&self.sets.text_styles, a("textStyle")), n, "R35", || {
                "@textStyle must name a styles/textStyle.".into()
            });
        }
        // p58
        if local == "object3D" && a("primitive") == Some("clay") {
            self.check(has_kid(n, "blob"), n, "C51", || "object3D primitive=\"clay\" needs at least one blob.".into());
        }
        // p59
        if local == "fluidSource" && has("start") && has("end") {
            let ok = xpath_number(&v("end")) > xpath_number(&v("start"));
            self.check(ok, n, "C52", || "fluidSource end must be after start.".into());
        }
        // p60
        if local == "geoLayer" && has("domain") {
            let d: Vec<f64> = normalize_space(&v("domain")).split(' ').map(xpath_number).collect();
            let ok = !d.windows(2).any(|w| w[1] <= w[0]);
            self.check(ok, n, "C53", || "@domain values must increase.".into());
        }
    }
}

/// Evaluates every Schematron rule and appends one diagnostic per failed assert.
pub fn validate(doc: &Document<'_>, out: &mut Vec<Diagnostic>) {
    let root = doc.root_element();
    let scene = is(root, "scene").then_some(root);
    let mut e = Eval { sets: build_sets(scene), out: Vec::new() };
    for n in root.descendants().filter(|n| n.is_element()) {
        e.element(n);
    }
    out.append(&mut e.out);
    physics_limits(root, out);
    count_limits(root, out);
    symbol_cycles(root, out);
    matte_cycles(root, out);
    expression_depths(root, out);
}

/// Nodes whose matte edges participate in a cycle through mattes or contained children.
/// Shared mattes and a container using its own descendant as a matte are valid DAGs.
pub fn cyclic_mattes(children: &[Vec<usize>], mattes: &[Option<usize>]) -> Vec<usize> {
    if mattes.iter().all(Option::is_none) {
        return Vec::new();
    }
    let mut next = children.to_vec();
    for (edges, matte) in next.iter_mut().zip(mattes) {
        edges.extend(matte);
    }
    let group = components(&next);
    mattes.iter().enumerate().filter_map(|(i, m)| m.filter(|&m| group[i] == group[m]).map(|_| i)).collect()
}

fn matte_cycles(root: Node, out: &mut Vec<Diagnostic>) {
    let nodes: Vec<_> = root.descendants().filter(|n| n.is_element()).collect();
    let indices: HashMap<_, _> = nodes.iter().enumerate().map(|(i, n)| (n.id(), i)).collect();
    let ids: HashMap<_, _> =
        nodes.iter().enumerate().filter_map(|(i, n)| n.attribute("id").map(|id| (id, i))).collect();
    let children: Vec<Vec<usize>> =
        nodes.iter().map(|n| n.children().filter_map(|c| indices.get(&c.id()).copied()).collect()).collect();
    let mattes: Vec<_> = nodes
        .iter()
        .map(|n| n.attribute("matte").filter(|_| !is(*n, "transition")).and_then(|m| ids.get(m).copied()))
        .collect();
    for i in cyclic_mattes(&children, &mattes) {
        let n = nodes[i];
        // Direct self references already have the Schematron R9 diagnostic.
        if mattes[i] == Some(i) {
            continue;
        }
        out.push(Diagnostic::error(
            "P05",
            "matte dependencies form a cycle through mattes or contained children",
            attr_loc(n, "matte"),
            element_path(n),
        ));
    }
}

fn attr_loc(n: Node, attr: &str) -> Loc {
    n.attributes()
        .find(|a| a.name() == attr && a.namespace().is_none())
        .map(|a| Loc::at(a.range().start))
        .unwrap_or_else(|| Loc::of(n))
}

/// Largest accepted values of the counts that become that many copies, points or particles and
/// that the schema leaves unbounded: (elements, attribute, maximum).
pub const COUNT_LIMITS: &[(&[&str], &str, f64)] = &[
    (&["shapeModifier"], "copies", 10_000.0),
    (&["shapeModifier"], "ridges", 10_000.0),
    (&["shape", "vector", "mask"], "points", 10_000.0),
    (&["burst"], "count", 10_000_000.0),
    (&["burst"], "repeat", 100_000.0),
    (&["repeat"], "count", 100_000.0),
    (&["object3D"], "instances", 100_000.0),
];

/// P02: a count above its entry in [`COUNT_LIMITS`].
fn count_limits(root: Node, out: &mut Vec<Diagnostic>) {
    for n in root.descendants().filter(|n| n.is_element() && n.tag_name().namespace().is_none()) {
        let name = n.tag_name().name();
        for (_, attr, max) in COUNT_LIMITS.iter().filter(|(elements, ..)| elements.contains(&name)) {
            let Some(v) = n.attribute(*attr) else { continue };
            if v.trim().parse::<f64>().is_ok_and(|x| x > *max) {
                out.push(Diagnostic::error(
                    "P02",
                    format!("@{attr} of <{name}> is {}; at most {max} is supported", v.trim()),
                    attr_loc(n, attr),
                    element_path(n),
                ));
            }
        }
    }
}

/// P03: an instance inside a symbol that makes the symbol contain itself, directly or through
/// other symbols.
fn symbol_cycles(root: Node, out: &mut Vec<Diagnostic>) {
    let symbols: Vec<Node> = kids(root, "symbols").flat_map(|s| s.children().filter(|c| is(*c, "symbol"))).collect();
    let index: HashMap<&str, usize> =
        symbols.iter().enumerate().filter_map(|(i, s)| Some((s.attribute("id")?, i))).collect();
    // (containing symbol, instanced symbol, instance)
    let mut uses: Vec<(usize, usize, Node)> = Vec::new();
    for (i, s) in symbols.iter().enumerate() {
        for d in s.descendants().filter(|d| is(*d, "instance")) {
            if let Some(&t) = d.attribute("symbol").and_then(|t| index.get(t)) {
                uses.push((i, t, d));
            }
        }
    }
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); symbols.len()];
    uses.iter().for_each(|(a, b, _)| next[*a].push(*b));
    let group = components(&next);
    for (a, b, inst) in uses {
        if group[a] != group[b] {
            continue;
        }
        let id = |i: usize| symbols[i].attribute("id").unwrap_or("");
        let message = if a == b {
            format!("symbol {:?} contains an instance of itself", id(a))
        } else {
            format!("symbol {:?} contains itself through its instance of symbol {:?}", id(a), id(b))
        };
        out.push(Diagnostic::error("P03", message, Loc::of(inst), element_path(inst)));
    }
}

/// The strongly connected component of every node of a graph (Tarjan, without recursion).
fn components(next: &[Vec<usize>]) -> Vec<usize> {
    const NONE: usize = usize::MAX;
    let n = next.len();
    let (mut order, mut low, mut group) = (vec![NONE; n], vec![0; n], vec![NONE; n]);
    let (mut stack, mut work): (Vec<usize>, Vec<(usize, usize)>) = (Vec::new(), Vec::new());
    let (mut visited, mut groups) = (0, 0);
    for start in 0..n {
        if order[start] != NONE {
            continue;
        }
        work.push((start, 0));
        while let Some(&(v, edge)) = work.last() {
            if edge == 0 {
                order[v] = visited;
                low[v] = visited;
                visited += 1;
                stack.push(v);
            }
            if let Some(&w) = next[v].get(edge) {
                work.last_mut().expect("non-empty").1 += 1;
                if order[w] == NONE {
                    work.push((w, 0));
                } else if group[w] == NONE {
                    low[v] = low[v].min(order[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == order[v] {
                while let Some(w) = stack.pop() {
                    group[w] = groups;
                    if w == v {
                        break;
                    }
                }
                groups += 1;
            }
        }
    }
    group
}

/// Deepest bracket nesting accepted in an expression.
pub const MAX_EXPRESSION_DEPTH: usize = 62;

/// Deepest nesting of `()`, `[]` and `{}` in an expression, outside string literals.
fn expression_depth(s: &str) -> usize {
    let (mut depth, mut max) = (0usize, 0usize);
    let mut quote = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(_), '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'' | '`') => quote = Some(c),
            (None, '(' | '[' | '{') => {
                depth += 1;
                max = max.max(depth);
            }
            (None, ')' | ']' | '}') => depth = depth.saturating_sub(1),
            (None, _) => {}
        }
    }
    max
}

/// P04: an `<expression>` or `@condition` nested deeper than [`MAX_EXPRESSION_DEPTH`].
fn expression_depths(root: Node, out: &mut Vec<Diagnostic>) {
    let mut check = |n: Node, what: &str, text: &str, loc: Loc| {
        let depth = expression_depth(text);
        if depth > MAX_EXPRESSION_DEPTH {
            out.push(Diagnostic::error(
                "P04",
                format!("{what} nests brackets {depth} deep; at most {MAX_EXPRESSION_DEPTH} levels are supported"),
                loc,
                element_path(n),
            ));
        }
    };
    for n in root.descendants().filter(|n| n.is_element()) {
        if let Some(c) = n.attribute("condition") {
            check(n, "@condition", c, attr_loc(n, "condition"));
        }
        if is(n, "expression") {
            let text: String = n.children().filter_map(|c| c.text()).collect();
            check(n, "the expression", &text, Loc::of(n));
        }
    }
}

/// P01: a soft body whose stable integration needs more than 4096 substeps per
/// `physics@fixedStep` (spring-mass lattice: ⌈fixedStep · √(8 k / m)⌉ with m the mass per node;
/// jelly and cloth have (rows + 1) × (cols + 1) nodes, a rope `cols`).
fn physics_limits(root: Node, out: &mut Vec<Diagnostic>) {
    let num = |n: Node, a: &str, d: f64| n.attribute(a).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(d);
    let step =
        root.children().find(|c| is(*c, "physics")).map(|p| num(p, "fixedStep", 1.0 / 120.0)).unwrap_or(1.0 / 120.0);
    for n in root.descendants().filter(|n| is(*n, "softBody")) {
        let (rows, cols) = (num(n, "rows", 4.0).max(1.0), num(n, "cols", 4.0).max(1.0));
        let nodes = if n.attribute("kind") == Some("rope") { cols } else { (rows + 1.0) * (cols + 1.0) };
        let m = (num(n, "mass", 1.0) / nodes).max(1e-12);
        let k = num(n, "stiffness", 20.0);
        let sub = (step * (8.0 * k / m).sqrt()).ceil();
        if sub > 4096.0 {
            out.push(Diagnostic::error(
                "P01",
                format!("softBody needs {sub} integration substeps per fixedStep {step}; at most 4096 are allowed"),
                Loc::of(n),
                element_path(n),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xpath_numbers() {
        assert_eq!(xpath_number(" 1.5 "), 1.5);
        assert_eq!(xpath_number("-.5"), -0.5);
        assert_eq!(xpath_number("3."), 3.0);
        assert_eq!(xpath_number("1e3"), 1000.0);
        assert_eq!(xpath_number("-.5e1"), -5.0);
        assert_eq!(xpath_number("1e"), 1.0);
        assert!(xpath_number("+1").is_nan());
        assert!(xpath_number("INF").is_nan());
        assert!(xpath_number("e3").is_nan());
        assert!(xpath_number("").is_nan());
        assert!(xpath_number(".").is_nan());
    }

    #[test]
    fn token_lists() {
        assert!(every_token_names("a  b", &["a", "b", "c"]));
        assert!(!every_token_names("a a", &["a"]));
        assert!(!every_token_names("a x", &["a"]));
        assert!(every_token_names("", &[]));
    }
}
