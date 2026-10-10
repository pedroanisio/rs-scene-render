//! Catalogue of every diagnostic code, used by `scene-render explain`.

include!(concat!(env!("OUT_DIR"), "/sch_asserts.rs"));

/// One diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Code {
    /// The code as it appears in diagnostics.
    pub code: &'static str,
    /// Stage that emits it: `xml`, `structure`, `rules`, `assets`, `model` or `inert`.
    pub stage: &'static str,
    /// One-line description.
    pub summary: &'static str,
    /// For Schematron asserts: the rule context and the XPath test.
    pub rule: Option<(&'static str, &'static str)>,
}

const FIXED: &[(&str, &str, &str)] = &[
    ("XML", "xml", "The file is not well-formed XML."),
    ("S01", "structure", "The root element is not <scene>."),
    ("S02", "structure", "An element is not allowed where it appears: unknown, out of order, or repeated too often."),
    ("S03", "structure", "A required child element is missing."),
    ("S04", "structure", "An attribute is not declared for its element."),
    ("S05", "structure", "A required attribute is missing."),
    ("S06", "structure", "An attribute value violates its type: lexical form, enumeration, bounds, pattern or length."),
    ("S07", "structure", "Text appears inside an element that only takes child elements."),
    ("S08", "structure", "Text content violates its type."),
    ("S09", "structure", "Two elements share the same xs:ID value; ids are unique across the document."),
    ("S10", "structure", "An xs:IDREF value names no element id."),
    ("S11", "structure", "An element or attribute is in a foreign namespace; scene-render uses no namespace."),
    ("S12", "structure", "Elements are nested deeper than the supported limit of 256 levels."),
    (
        "W01",
        "structure",
        "A numeric attribute holds INF, -INF, NaN or a number too large for a double (such as 1e400), also inside lengths and number lists; the schema accepts them but the renderer needs finite values.",
    ),
    (
        "W02",
        "rules",
        "A pyro source or impulse in a volume with open faces is less than 12 cells from one of them, where its position is in the document and the volume has room to put it further in; the open face changes the flow of the cloud. The number comes from one plume and one face; the side faces were not measured.",
    ),
    (
        "W03",
        "rules",
        "A blackHole or an accretionDisk is in a scene where no camera has geodesics=\"true\": nothing draws them, and the lens is not rendered. Also used by canonical generated-marker validation when an explicit marker shadows a generated beat or bar marker.",
    ),
    (
        "W04",
        "rules",
        "A camera with geodesics=\"true\" has denoise=\"true\": it does not denoise; pathSamples are antialiasing samples. Also used by canonical generated-marker validation when a poster or thumbnail names no marker.",
    ),
    ("W05", "rules", "A camera with geodesics=\"true\" is in a scene with lights: they are not used."),
    (
        "W06",
        "rules",
        "A whitewater with foamMode=\"albedo\" names a foamMaterial: it is not used, the foam is the water's own.",
    ),
    (
        "W08",
        "rules",
        "A whitewater with foamMode=\"albedo\" is in an ocean whose material is not opaque, is unlit or is emissive: the foam mix needs an opaque, lit material without emission, and a renderer reports an error.",
    ),
    (
        "W07",
        "rules",
        "A whitewater with foamMode=\"albedo\" is in a scene where no camera has renderer=\"pathtrace\": the mix is the path tracer's, and the raster renderer reports an error.",
    ),
    (
        "W09",
        "rules",
        "The cells of an object3D of primitive=\"voxels\" are smaller than 0.5 in the scene (cellSize of the object or of its asset times the smallest side of its scale): the shadow the raster renderer casts from such small cells is displaced or lost (measured: 23 of 51 pixels differ at 0.25, all at 0.1 and below); the path tracer's holds down to 0.05.",
    ),
    (
        "W10",
        "rules",
        "A medium has scatterBounces above 1 and an albedo of black: it scatters no light, so the extra bounces have nothing to do.",
    ),
    ("A01", "assets", "A referenced input file does not exist."),
    ("A02", "assets", "A file's SHA-256 digest differs from the declared sha256 or cacheSha256."),
    ("A03", "assets", "A remote input (http, https, s3, …) could not be verified offline."),
    ("A04", "assets", "Frames of an image sequence are missing; an error when missingFrame=\"error\"."),
    ("A05", "assets", "A referenced file exists but could not be read."),
    ("A06", "assets", "A physics cache is declared but absent; the simulation will be recomputed."),
    ("A07", "assets", "An image's declared width and height differ from its local source file. Layout uses the declared dimensions, so a different aspect ratio distorts the image. Set the asset dimensions to the file's size and use the layer's boxWidth, boxHeight and fit for layout. Proxy representations are not compared."),
    ("CMP10", "assets", "A compute node's WGSL source cannot be read, or its SHA-256 differs from compute/@sha256 (SREP 67)."),
    ("CMP11", "model", "A compute node's WGSL source does not compile against the engine's prelude, defines no fn sr_point(i: u32), or declares its own @group(0) bindings (SREP 67)."),
    ("CMP12", "model", "The device cannot hold a compute node's histogram: width x height x channels cells of 8 bytes (SREP 67)."),
    ("PRG10", "assets", "A program's WebAssembly module cannot be read, or its SHA-256 differs from @sha256 (SREP 66)."),
    ("PRG11", "model", "A program's module is not usable: invalid WebAssembly, a forbidden feature (threads, shared memory, atomics, relaxed SIMD), an import other than sr.rand_u64, sr.rand_f64, sr.param_f64 and sr.param_str, a missing or mistyped export (memory, generate; init, step and frame in step mode), or a declared memory minimum above memoryLimit (SREP 66, 69)."),
    ("PRG12", "model", "A program spent more fuel (executed instructions) than its @fuel allows, in one call of generate, init, step or frame (SREP 66, 69)."),
    ("PRG13", "model", "A program trapped: unreachable, an out-of-bounds access, a division by zero, stack exhaustion, or the host could not provide memory below memoryLimit (SREP 66)."),
    ("PRG14", "model", "A program's output is unusable: not UTF-8, not well-formed, invalid in place, colliding with another id, over 1,000,000 elements, containing a program, not the JSON a data source accepts, or (step mode) not width × height × 4 bytes (SREP 66, 69)."),
    ("PRG15", "model", "A program's output differs from its @outputSha256 (SREP 66)."),
    ("PRG16", "model", "sr.param_f64 read a parameter whose value is not a number (SREP 66)."),
    ("P01", "rules","A soft body is too stiff for its mass: stable integration needs more than 4096 substeps per physics@fixedStep."),
    ("P02", "rules", "A count (repeater copies, star points, zig-zag ridges, burst count or repeat, repeat count, object3D instances) is far above what a renderer can draw."),
    ("P03", "rules", "A symbol contains an instance of itself, directly or through other symbols."),
    ("P04", "rules", "An expression or condition nests brackets deeper than the supported limit of 62 levels, the depth the evaluator compiles."),
    ("P05", "rules", "Matte dependencies form a cycle through mattes or contained children, including after composition expansion."),
    ("SA01", "rules", "Text, a burned caption, or a node tagged cta or logo reaches outside the safe region its safeArea leaves, at some time of the timeline. A warning when safeArea@enforce is warn, an error when it is error. Move the content inside the region or lower enforce. Validation and encode audit every frame of a short range and at most 240 frames of a long one, and say how many they sampled."),
    ("OUT1", "rules", "An H.264 or H.265 output has a frame of odd width or height where its pixel format halves the chroma (4:2:0: both sides; 4:2:2: the width): libx264 and libx265 refuse it when the encode starts, after the first frame is rendered. The frame is the output's width and height, else scene360's, else its layout's, else the project's. Make the size even, on the output or where it comes from; encode -o outputs are checked before rendering."),
    ("M01", "model", "Internal error: the validated document could not be converted to the typed model."),
];

/// Every code: fixed codes first, then the inert-attribute rules (SREP 18), then the Schematron asserts in schema
/// order.
pub fn all() -> Vec<Code> {
    let mut v: Vec<Code> =
        FIXED.iter().map(|(code, stage, summary)| Code { code, stage, summary, rule: None }).collect();
    v.extend(crate::inert::RULES.iter().map(|(code, summary)| Code { code, stage: "inert", summary, rule: None }));
    v.extend(SCH_ASSERTS.iter().map(|(id, _p, ctx, test, msg)| Code {
        code: id,
        stage: "rules",
        summary: msg,
        rule: Some((ctx, test)),
    }));
    v
}

/// Looks up a code, case-insensitively.
pub fn lookup(code: &str) -> Option<Code> {
    all().into_iter().find(|c| c.code.eq_ignore_ascii_case(code))
}
