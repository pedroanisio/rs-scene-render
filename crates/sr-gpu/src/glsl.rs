//! GLSL custom shaders in the conventions of the Python engine (`effects/shader.py`,
//! `transitions/shader.py`), compiled through naga.
//!
//! Effects: a Shadertoy program (`void mainImage(out vec4, in vec2)`), an ISF program
//! (a leading `/*{ JSON }*/` header: INPUTS, IMPORTED, PASSES) or a plain `void main()`
//! reading `inputTexture`/`uv`/`time`; transitions: gl-transitions (`vec4 transition(vec2)`).
//! Desktop GLSL (`#version 330`, loose uniforms, `sampler2D`, `texture2D`/`varying`/
//! `gl_FragColor`) becomes Vulkan GLSL 450: loose uniforms join one std140 block (binding 0),
//! each `sampler2D` becomes a texture (binding 2 + k) sampled through one sampler (binding 1).
//!
//! Orientation: programs run on textures stored bottom row first (GL order), so `uv`,
//! `gl_FragCoord` and texture lookups follow GL conventions unchanged; the passes around the
//! program flip rows on the way in and out.

use std::collections::HashMap;

use regex::Regex;

/// Which harness wraps the user code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Shadertoy,
    Isf,
    Plain,
    Transition,
}

/// Scalar type of a uniform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Float,
    Int,
    Uint,
    /// Stored as an int (naga has no host-shareable bool).
    Bool,
}

/// A uniform in the program's std140 block.
#[derive(Clone, Debug)]
pub struct Uniform {
    pub name: String,
    pub base: Base,
    /// Vector size (1 to 4); rows of a matrix.
    pub rows: u32,
    /// 1 for scalars and vectors; columns of a matrix.
    pub cols: u32,
    /// Array length (0: not an array).
    pub array: u32,
    /// Byte offset in the block.
    pub offset: u32,
    /// Byte stride between array elements (and matrix columns).
    pub stride: u32,
}

impl Uniform {
    /// Components of one element (rows × columns).
    pub fn dimension(&self) -> usize {
        (self.rows * self.cols) as usize
    }

    fn elements(&self) -> usize {
        self.array.max(1) as usize * self.cols as usize
    }
}

/// A compiled-ready program.
#[derive(Clone, Debug)]
pub struct Program {
    pub kind: Kind,
    /// Vulkan GLSL 450 fragment shader.
    pub glsl: String,
    pub uniforms: Vec<Uniform>,
    /// Size of the uniform block in bytes (at least 16).
    pub block_size: u32,
    /// Sampler names in binding order (binding 2 + index).
    pub samplers: Vec<String>,
    /// `// = value` defaults and ISF DEFAULTs.
    pub defaults: HashMap<String, String>,
    /// The ISF header.
    pub isf: Option<serde_json::Value>,
    /// Lines of harness before the user's line 1 (to translate compiler line numbers).
    pub prelude_lines: u32,
}

impl Program {
    pub fn uniform(&self, name: &str) -> Option<&Uniform> {
        self.uniforms.iter().find(|u| u.name == name)
    }

    /// An empty uniform block for this program.
    pub fn block(&self) -> Vec<u8> {
        vec![0u8; self.block_size as usize]
    }

    /// Writes `vals` into uniform `u` of `block` (Python's `write_uniform`: a scalar
    /// broadcasts, a vec4 given three values gets 1 as fourth, missing values are 0, int
    /// and bool uniforms round).
    pub fn write(&self, block: &mut [u8], u: &Uniform, vals: &[f64]) {
        if vals.is_empty() {
            return;
        }
        let dim = u.dimension();
        let n = dim * u.array.max(1) as usize;
        let mut v: Vec<f64> = vals.iter().map(|x| if x.is_finite() { *x } else { 0.0 }).collect();
        if v.len() == 1 {
            v = vec![v[0]; n];
        } else if v.len() < n {
            if dim == 4 && v.len() == 3 {
                v.push(1.0);
            }
            v.resize(n, 0.0);
        }
        // element-major: arrays of vectors/matrices, matrices column-major
        let mut k = 0;
        for e in 0..u.elements() {
            let base = (u.offset + e as u32 * u.stride) as usize;
            for r in 0..u.rows as usize {
                let at = base + r * 4;
                let x = v[k];
                k += 1;
                let bytes = match u.base {
                    Base::Float => (x as f32).to_le_bytes(),
                    Base::Int | Base::Bool => (x.round() as i32).to_le_bytes(),
                    Base::Uint => (x.round().max(0.0) as u32).to_le_bytes(),
                };
                if at + 4 <= block.len() {
                    block[at..at + 4].copy_from_slice(&bytes);
                }
            }
        }
    }
}

fn re(s: &str) -> Regex {
    Regex::new(s).expect("static regex")
}

/// `(#version line to keep, code without its #version line)` — a `#version` of 330 or later is
/// accepted (naga always receives 450); the line count is kept.
fn split_version(code: &str) -> String {
    let v = re(r"(?m)^[ \t]*#[ \t]*version[^\n]*");
    v.replace(code, "").into_owned()
}

/// `texture2D` → `texture`, `varying` → `in`, `gl_FragColor` → `_sr_fragColor`.
fn modernize(code: &str) -> String {
    let c = re(r"\btexture2D\b").replace_all(code, "texture");
    let c = re(r"\bvarying\b").replace_all(&c, "in");
    re(r"\bgl_FragColor\b").replace_all(&c, "_sr_fragColor").into_owned()
}

fn used(code: &str, name: &str) -> bool {
    Regex::new(&format!(r"\b{}\b", regex::escape(name))).map(|r| r.is_match(code)).unwrap_or(false)
}

/// gl-transitions defaults: `uniform vec2 direction; // = vec2(1.0, -1.0)`.
pub fn comment_defaults(code: &str) -> HashMap<String, String> {
    let r = re(
        r"(?m)^[ \t]*uniform[ \t]+(?:(?:lowp|mediump|highp)[ \t]+)?\w+[ \t]+(\w+)[ \t]*(?:\[[^\]]*\])?[ \t]*;[ \t]*//[ \t]*=[ \t]*(.+?)[ \t]*;?[ \t]*$",
    );
    r.captures_iter(code).map(|c| (c[1].to_string(), c[2].to_string())).collect()
}

/// A declared uniform before layout.
#[derive(Clone, Debug)]
struct Decl {
    ty: String,
    name: String,
    array: u32,
}

/// Blanks `m` in `s` keeping its newlines (line numbers of later code stay valid).
fn blank(s: &mut String, range: std::ops::Range<usize>) {
    let kept: String = s[range.clone()].chars().filter(|c| *c == '\n').collect();
    s.replace_range(range, &kept);
}

/// Removes the user's loose uniform declarations; returns them.
fn take_uniforms(code: &mut String) -> Result<Vec<Decl>, String> {
    // a declaration starts a line or follows `;`, `{` or `}` (several may share a line)
    let r = re(
        r"((?:\blayout[ \t]*\([^)]*\)[ \t]*)?\buniform[ \t]+(?:(?:lowp|mediump|highp)[ \t]+)?(\w+)[ \t]+([^;{}]+);)",
    );
    let mut out = Vec::new();
    let mut ranges = Vec::new();
    // match on a comment-blanked copy (`uniform vec3 color /* = ... */;`), edit the original
    let scan = uncommented(code);
    for c in r.captures_iter(&scan) {
        let at = c.get(1).unwrap().start();
        let before = scan[..at].trim_end_matches([' ', '\t']);
        if !(before.is_empty() || before.ends_with(['\n', ';', '{', '}'])) {
            continue;
        }
        let ty = c[2].to_string();
        for part in c[3].split(',') {
            let part = part.trim();
            let (name, array) = match part.find('[') {
                Some(i) => {
                    let n = part[i + 1..].trim_end_matches(']').trim();
                    let len = n.parse::<u32>().map_err(|_| format!("uniform {part}: array length must be a number"))?;
                    (part[..i].trim(), len)
                }
                None => (part.split('=').next().unwrap_or(part).trim(), 0),
            };
            if !name.is_empty() {
                out.push(Decl { ty: ty.clone(), name: name.to_string(), array });
            }
        }
        ranges.push(c.get(1).unwrap().range());
    }
    for rg in ranges.into_iter().rev() {
        blank(code, rg);
    }
    Ok(out)
}

/// Removes `in vec2 NAME;` inputs (every input is the GL uv); returns their names.
fn take_inputs(code: &mut String) -> Vec<String> {
    let r = re(
        r"(?m)^[ \t]*(?:layout[ \t]*\([^)]*\)[ \t]*)?in[ \t]+(?:(?:lowp|mediump|highp)[ \t]+)?vec2[ \t]+(\w+)[ \t]*;",
    );
    let names: Vec<(String, std::ops::Range<usize>)> =
        r.captures_iter(code).map(|c| (c[1].to_string(), c.get(0).unwrap().range())).collect();
    for (_, rg) in names.iter().rev() {
        blank(code, rg.clone());
    }
    names.into_iter().map(|(n, _)| n).collect()
}

/// Turns `out vec4 NAME;` into a global; returns NAME.
fn take_output(code: &mut String) -> Option<String> {
    let r = re(
        r"(?m)^[ \t]*(?:layout[ \t]*\([^)]*\)[ \t]*)?out[ \t]+(?:(?:lowp|mediump|highp)[ \t]+)?vec4[ \t]+(\w+)[ \t]*;",
    );
    let (name, rg) = r.captures(code).map(|c| (c[1].to_string(), c.get(0).unwrap().range()))?;
    code.replace_range(rg, &format!("vec4 {name};"));
    Some(name)
}

fn strip_precision(code: &str) -> String {
    re(r"(?m)^[ \t]*precision[ \t]+\w+[ \t]+\w+[ \t]*;").replace_all(code, "").into_owned()
}

/// `code` with comments blanked (same length and newlines), for scanning.
fn uncommented(code: &str) -> String {
    let b = code.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            while i < b.len() && !(b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/') {
                if b[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            let end = (i + 2).min(b.len());
            out[i..end].fill(b' ');
            i += 2;
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| code.to_string())
}

/// Moves non-constant initializers of global variables (`float d = min(progress, 0.5);`,
/// accepted by GL drivers, rejected by naga) into `sr_globals()`, run first in `main`.
fn hoist_globals(code: &mut String) -> String {
    let scan = uncommented(code);
    let decl = re(
        r"(?s)^\s*(?:(?:highp|mediump|lowp)\s+)?((?:float|int|uint|bool|[biu]?vec[234]|mat[234])(?:\s*\[\s*\d*\s*\])?)\s+(\w+)\s*(\[\s*\d*\s*\])?\s*=\s*(.+)$",
    );
    let (mut depth, mut paren, mut start) = (0i32, 0i32, 0usize);
    let mut edits = Vec::new();
    for (i, c) in scan.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    start = i + 1;
                }
            }
            '(' => paren += 1,
            ')' => paren -= 1,
            ';' if depth == 0 && paren == 0 => {
                let stmt = &scan[start..i];
                let trimmed = stmt.trim_start();
                if !trimmed.starts_with("const") && !trimmed.starts_with('#') {
                    if let Some(c) = decl.captures(stmt) {
                        let (ty, name, arr, init) = (
                            c[1].to_string(),
                            c[2].to_string(),
                            c.get(3).map(|m| m.as_str().to_string()).unwrap_or_default(),
                            c[4].trim().to_string(),
                        );
                        edits.push((start..i, ty, name, arr, init));
                    }
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let mut inits = String::new();
    for (rg, ty, name, arr, _) in edits.iter().rev() {
        let kept: String = code[rg.clone()].chars().filter(|c| *c == '\n').collect();
        code.replace_range(rg.clone(), &format!("{kept}{ty} {name}{arr}"));
    }
    for (_, _, name, _, init) in &edits {
        inits.push_str(&format!("{name} = {init}; "));
    }
    if inits.is_empty() {
        String::new()
    } else {
        format!("\nvoid sr_globals() {{ {inits}}}\n")
    }
}

/// (base, rows, cols) of a GLSL type name.
fn type_shape(ty: &str) -> Option<(Base, u32, u32)> {
    Some(match ty {
        "float" => (Base::Float, 1, 1),
        "int" => (Base::Int, 1, 1),
        "uint" => (Base::Uint, 1, 1),
        "bool" => (Base::Bool, 1, 1),
        "vec2" => (Base::Float, 2, 1),
        "vec3" => (Base::Float, 3, 1),
        "vec4" => (Base::Float, 4, 1),
        "ivec2" => (Base::Int, 2, 1),
        "ivec3" => (Base::Int, 3, 1),
        "ivec4" => (Base::Int, 4, 1),
        "uvec2" => (Base::Uint, 2, 1),
        "uvec3" => (Base::Uint, 3, 1),
        "uvec4" => (Base::Uint, 4, 1),
        "mat2" => (Base::Float, 2, 2),
        "mat3" => (Base::Float, 3, 3),
        "mat4" => (Base::Float, 4, 4),
        _ => return None,
    })
}

fn is_sampler(ty: &str) -> bool {
    ty == "sampler2D"
}

/// Lays out the uniforms in std140 and emits the block and the bool/sampler macros.
fn layout(decls: &[Decl]) -> Result<(Vec<Uniform>, u32, String), String> {
    let mut uniforms = Vec::new();
    let mut members = String::new();
    let mut macros = String::new();
    let mut off = 0u32;
    for d in decls {
        let (base, rows, cols) =
            type_shape(&d.ty).ok_or_else(|| format!("uniform {} {}: type not supported", d.ty, d.name))?;
        if base == Base::Bool && (rows > 1 || d.array > 0) {
            return Err(format!("uniform {} {}: bool vectors and arrays are not supported", d.ty, d.name));
        }
        // std140: vec2 aligns to 8, vec3/vec4 to 16; arrays and matrix columns to 16
        let (align, size, stride) = if d.array > 0 || cols > 1 {
            let n = d.array.max(1) * cols;
            (16, 16 * n, 16)
        } else {
            let a = match rows {
                1 => 4,
                2 => 8,
                _ => 16,
            };
            (a, 4 * rows, 16)
        };
        off = off.div_ceil(align) * align;
        let member = if base == Base::Bool { format!("{}_srb", d.name) } else { d.name.clone() };
        let ty = if base == Base::Bool { "int".to_string() } else { d.ty.clone() };
        let arr = if d.array > 0 { format!("[{}]", d.array) } else { String::new() };
        members.push_str(&format!("    {ty} {member}{arr};\n"));
        if base == Base::Bool {
            macros.push_str(&format!("#define {} ({}_srb != 0)\n", d.name, d.name));
        }
        uniforms.push(Uniform { name: d.name.clone(), base, rows, cols, array: d.array, offset: off, stride });
        off += size;
    }
    if uniforms.is_empty() {
        members.push_str("    float sr_unused;\n");
        off = 4;
    }
    let size = off.div_ceil(16).max(1) * 16;
    let block = format!("layout(std140, set = 0, binding = 0) uniform SrUniforms {{\n{members}}};\n{macros}");
    Ok((uniforms, size, block))
}

fn samplers_block(names: &[String]) -> String {
    let mut s = String::from("layout(set = 0, binding = 1) uniform sampler sr_smp;\n");
    for (k, n) in names.iter().enumerate() {
        s.push_str(&format!(
            "layout(set = 0, binding = {}) uniform texture2D {n}_srt;\n#define {n} sampler2D({n}_srt, sr_smp)\n",
            2 + k
        ));
    }
    s
}

const ISF_MACROS: &str = "#define isf_FragNormCoord _sr_uv
#define vv_FragNormCoord _sr_uv
#define IMG_NORM_PIXEL(i, c) texture(i, c)
#define IMG_PIXEL(i, c) texture(i, (c) / vec2(textureSize(i, 0)))
#define IMG_THIS_PIXEL(i) texture(i, gl_FragCoord.xy / vec2(textureSize(i, 0)))
#define IMG_THIS_NORM_PIXEL(i) texture(i, _sr_uv)
#define IMG_NORM_THIS_PIXEL(i) texture(i, _sr_uv)
#define IMG_SIZE(i) vec2(textureSize(i, 0))
";

fn isf_type(t: &str) -> &'static str {
    match t {
        "bool" | "event" => "bool",
        "long" => "int",
        "color" => "vec4",
        "point2D" => "vec2",
        "image" | "audio" | "audioFFT" => "sampler2D",
        _ => "float",
    }
}

/// Builds an effect program from its source.
pub fn build_effect(code: &str) -> Result<Program, String> {
    let code = split_version(code);
    let mainimage = re(r"\bmainImage\s*\(").is_match(&code);
    let mut header = None;
    let mut code = if mainimage {
        code
    } else {
        let h = re(r"^\s*/\*\s*(\{(?s:.*?)\})\s*\*/");
        match h
            .captures(&code)
            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c[1]).ok().map(|v| (v, c.get(0).unwrap().range())))
        {
            Some((v, rg)) if v.is_object() => {
                header = Some(v);
                let mut c = code.clone();
                blank(&mut c, rg);
                c
            }
            _ => code,
        }
    };
    let legacy_out = used(&code, "gl_FragColor");
    code = strip_precision(&modernize(&code));
    // The earlier Rust-only convention: `vec4 effect(vec2 uv)` reading `getColor(uv)`.
    let effect_fn = !mainimage
        && header.is_none()
        && re(r"\bvec4\s+effect\s*\(").is_match(&code)
        && !re(r"\bvoid\s+main\s*\(").is_match(&code);
    if effect_fn {
        code.push_str("\nvoid main() { fragColor = effect(uv); }\n");
        if !code.contains("inputTexture") {
            code.push_str("// inputTexture\n");
        }
    }
    let kind = if mainimage {
        Kind::Shadertoy
    } else if header.is_some() {
        Kind::Isf
    } else {
        Kind::Plain
    };
    let defaults_comment = comment_defaults(&code);
    let mut decls = take_uniforms(&mut code)?;
    let inputs = take_inputs(&mut code);
    let globals = hoist_globals(&mut code);
    let user_out = if legacy_out { None } else { take_output(&mut code) };
    let declared = |decls: &[Decl], n: &str| decls.iter().any(|d| d.name == n);
    let mut defaults = HashMap::new();
    let push = |decls: &mut Vec<Decl>, ty: &str, name: &str, array: u32| {
        if !decls.iter().any(|d| d.name == name) {
            decls.push(Decl { ty: ty.into(), name: name.into(), array });
        }
    };
    match kind {
        Kind::Shadertoy => {
            for (ty, n, a) in [
                ("vec3", "iResolution", 0),
                ("float", "iTime", 0),
                ("float", "iTimeDelta", 0),
                ("float", "iFrameRate", 0),
                ("float", "iSampleRate", 0),
                ("int", "iFrame", 0),
                ("vec4", "iMouse", 0),
                ("vec4", "iDate", 0),
                ("vec3", "iChannelResolution", 4),
                ("float", "iChannelTime", 4),
                ("sampler2D", "iChannel0", 0),
                ("sampler2D", "iChannel1", 0),
                ("sampler2D", "iChannel2", 0),
                ("sampler2D", "iChannel3", 0),
                ("vec2", "iOffset", 0),
                ("vec2", "iFrameResolution", 0),
            ] {
                push(&mut decls, ty, n, a);
            }
        }
        Kind::Isf | Kind::Plain => {
            if let Some(h) = &header {
                for (ty, n) in [
                    ("vec2", "RENDERSIZE"),
                    ("float", "TIME"),
                    ("float", "TIMEDELTA"),
                    ("int", "FRAMEINDEX"),
                    ("int", "PASSINDEX"),
                    ("vec4", "DATE"),
                ] {
                    push(&mut decls, ty, n, 0);
                }
                for inp in h.get("INPUTS").and_then(|v| v.as_array()).into_iter().flatten() {
                    let (Some(name), t) = (
                        inp.get("NAME").and_then(|v| v.as_str()),
                        inp.get("TYPE").and_then(|v| v.as_str()).unwrap_or(""),
                    ) else {
                        continue;
                    };
                    let ty = isf_type(t);
                    push(&mut decls, ty, name, 0);
                    if ty != "sampler2D" {
                        if let Some(d) = inp.get("DEFAULT") {
                            let s = match d {
                                serde_json::Value::Array(a) => {
                                    a.iter().filter_map(json_num).map(|x| x.to_string()).collect::<Vec<_>>().join(",")
                                }
                                other => json_num(other).map(|x| x.to_string()).unwrap_or_default(),
                            };
                            defaults.insert(name.to_string(), s);
                        }
                    }
                }
                for name in h.get("IMPORTED").and_then(|v| v.as_object()).into_iter().flatten().map(|(k, _)| k) {
                    push(&mut decls, "sampler2D", name, 0);
                }
                for ps in h.get("PASSES").and_then(|v| v.as_array()).into_iter().flatten() {
                    if let Some(t) = ps.get("TARGET").and_then(|v| v.as_str()) {
                        push(&mut decls, "sampler2D", t, 0);
                    }
                }
            }
            for (ty, n) in [
                ("sampler2D", "inputTexture"),
                ("sampler2D", "sourceTexture"),
                ("vec2", "resolution"),
                ("vec2", "frameResolution"),
                ("vec2", "tileOffset"),
                ("float", "time"),
                ("float", "localTime"),
                ("float", "timeDelta"),
                ("float", "fps"),
                ("int", "frame"),
            ] {
                if used(&code, n) && !declared(&decls, n) {
                    push(&mut decls, ty, n, 0);
                }
            }
        }
        Kind::Transition => unreachable!(),
    }
    let mut defs = defaults_comment;
    defs.extend(defaults);
    let (samplers, loose): (Vec<Decl>, Vec<Decl>) = decls.into_iter().partition(|d| is_sampler(&d.ty));
    let (uniforms, size, block) = layout(&loose)?;
    let samplers: Vec<String> = samplers.into_iter().map(|d| d.name).collect();
    let mut pre =
        String::from("#version 450\nlayout(location = 0) in vec2 sr_vuv;\nlayout(location = 0) out vec4 sr_out;\n");
    pre.push_str(&block);
    pre.push_str(&samplers_block(&samplers));
    let (call, result) = match kind {
        Kind::Shadertoy => {
            let out = user_out.clone();
            pre.push_str(&out.as_ref().map(|_| String::new()).unwrap_or_default());
            ("vec4 sr_c = vec4(0.0); mainImage(sr_c, gl_FragCoord.xy);".to_string(), "sr_c".to_string())
        }
        _ => {
            code = re(r"\bvoid\s+main\s*\(\s*(?:void)?\s*\)").replace(&code, "void sr_user_main()").into_owned();
            let out = if legacy_out {
                pre.push_str("vec4 _sr_fragColor;\n");
                "_sr_fragColor".to_string()
            } else {
                match &user_out {
                    Some(n) => n.clone(),
                    None => {
                        pre.push_str("vec4 fragColor;\n");
                        "fragColor".to_string()
                    }
                }
            };
            ("sr_user_main();".to_string(), out)
        }
    };
    if effect_fn {
        pre.push_str("vec4 getColor(vec2 p) { return texture(inputTexture, p); }\n");
    }
    let mut uv_names: Vec<String> = inputs;
    if kind == Kind::Isf {
        uv_names.push("_sr_uv".into());
        pre.push_str(ISF_MACROS);
    } else if used(&code, "uv") && !uv_names.iter().any(|n| n == "uv") {
        uv_names.push("uv".into());
    }
    uv_names.sort();
    uv_names.dedup();
    for n in &uv_names {
        pre.push_str(&format!("vec2 {n};\n"));
    }
    let assign: String = uv_names.iter().map(|n| format!("{n} = sr_vuv; ")).collect();
    let prelude_lines = pre.lines().count() as u32 + 1;
    let init = if globals.is_empty() { "" } else { "sr_globals(); " };
    let glsl = format!("{pre}#line 1\n{code}{globals}\nvoid main() {{ {assign}{init}{call} sr_out = {result}; }}\n");
    Ok(Program { kind, glsl, uniforms, block_size: size, samplers, defaults: defs, isf: header, prelude_lines })
}

/// Builds a gl-transitions program.
pub fn build_transition(code: &str) -> Result<Program, String> {
    let code = split_version(code);
    let legacy = used(&code, "gl_FragColor");
    let mut code = strip_precision(&modernize(&code));
    let defaults = comment_defaults(&code);
    let mut decls = take_uniforms(&mut code)?;
    let _ = take_inputs(&mut code);
    let globals = hoist_globals(&mut code);
    for (ty, n) in [("sampler2D", "from"), ("sampler2D", "to"), ("float", "progress"), ("float", "ratio")] {
        if !decls.iter().any(|d| d.name == n) {
            decls.push(Decl { ty: ty.into(), name: n.into(), array: 0 });
        }
    }
    for (ty, n) in [("vec2", "resolution"), ("float", "time"), ("int", "frame"), ("sampler2D", "matte")] {
        if used(&code, n) && !decls.iter().any(|d| d.name == n) {
            decls.push(Decl { ty: ty.into(), name: n.into(), array: 0 });
        }
    }
    let (samplers, loose): (Vec<Decl>, Vec<Decl>) = decls.into_iter().partition(|d| is_sampler(&d.ty));
    let (uniforms, size, block) = layout(&loose)?;
    let samplers: Vec<String> = samplers.into_iter().map(|d| d.name).collect();
    let mut pre =
        String::from("#version 450\nlayout(location = 0) in vec2 sr_vuv;\nlayout(location = 0) out vec4 sr_out;\n");
    pre.push_str(&block);
    // `from`/`to` are often reused as local names: they are macros only when sampled directly
    let sampled = |n: &str| {
        Regex::new(&format!(r"\b(?:texture|texelFetch|textureSize|textureLod)\s*\(\s*{n}\b"))
            .map(|r| r.is_match(&code))
            .unwrap_or(false)
    };
    let mut sb = samplers_block(&samplers);
    for n in ["from", "to"] {
        if !sampled(n) {
            sb = sb.replace(&format!("#define {n} sampler2D({n}_srt, sr_smp)\n"), "");
        }
    }
    pre.push_str(&sb);
    pre.push_str("vec4 getFromColor(vec2 uv) { return texture(sampler2D(from_srt, sr_smp), uv); }\nvec4 getToColor(vec2 uv) { return texture(sampler2D(to_srt, sr_smp), uv); }\n");
    if legacy {
        pre.push_str("vec4 _sr_fragColor;\n");
    }
    let prelude_lines = pre.lines().count() as u32 + 1;
    let init = if globals.is_empty() { "" } else { "sr_globals(); " };
    let glsl = format!("{pre}#line 1\n{code}{globals}\nvoid main() {{ {init}sr_out = transition(sr_vuv); }}\n");
    Ok(Program {
        kind: Kind::Transition,
        glsl,
        uniforms,
        block_size: size,
        samplers,
        defaults,
        isf: None,
        prelude_lines,
    })
}

fn json_num(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::Bool(b) => Some(*b as u8 as f64),
        _ => None,
    }
}

/// Parses a param/default string as numbers: `0.5`, `1, 2`, `vec3(1.0, 0.5, 0.25)`,
/// `true`/`false`, `2.0f`. `None` when it holds none (colours are resolved by the caller).
pub fn parse_numbers(s: &str) -> Option<Vec<f64>> {
    let s = s.trim();
    match s {
        "true" => return Some(vec![1.0]),
        "false" => return Some(vec![0.0]),
        _ => {}
    }
    let body = match (s.find('('), s.rfind(')')) {
        (Some(a), Some(b))
            if b > a && s[..a].chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c.is_whitespace()) =>
        {
            &s[a + 1..b]
        }
        _ => s,
    };
    let mut out = Vec::new();
    for tok in body.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()) {
        let t = tok.trim_end_matches(['f', 'F']);
        out.push(t.parse::<f64>().ok()?);
    }
    (!out.is_empty()).then_some(out)
}

/// A shader source: a file path or a `data:` URI (`data:,<url-encoded>` or `data:<mime>;base64,<b64>`).
pub fn load_source(uri: &str, base: &std::path::Path) -> Result<(String, std::path::PathBuf), String> {
    let uri = uri.trim();
    if let Some(rest) = uri.strip_prefix("data:") {
        let (head, body) = rest.split_once(',').ok_or("data: URI without a comma")?;
        let bytes = if head.ends_with(";base64") { base64_decode(body)? } else { percent_decode(body) };
        let text = String::from_utf8(bytes).map_err(|e| format!("undecodable data: URI ({e})"))?;
        return Ok((text, base.to_path_buf()));
    }
    let p = uri.strip_prefix("file://").unwrap_or(uri);
    let path = match sr_model::assets::resolve(p, base) {
        sr_model::assets::Resolved::Local(p) => p,
        _ => return Err(format!("{uri}: only local shader files and data: URIs are supported")),
    };
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("shader file not found ({}): {e}", path.display()))?;
    let dir = path.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| base.to_path_buf());
    Ok((text, dir))
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | val(c).ok_or("invalid base64")?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// Translates compiler diagnostics to the user's line numbers (lines of the wrapped source
/// minus the harness prelude).
pub fn user_lines(msg: &str, prelude: u32) -> String {
    let r = re(r":(\d+):(\d+)");
    r.replace_all(msg, |c: &regex::Captures| {
        let l: u32 = c[1].parse().unwrap_or(0);
        format!(":{}:{}", l.saturating_sub(prelude).max(1), &c[2])
    })
    .into_owned()
}

// ------------------------------------------------------------------ ISF pass sizes

/// Evaluates an ISF WIDTH/HEIGHT expression (`$WIDTH/16.0`, `floor($HEIGHT*0.5)`, `max(1., $W)`).
pub fn isf_size(expr: Option<&serde_json::Value>, names: &HashMap<String, f64>, default: u32) -> u32 {
    let Some(e) = expr else { return default };
    let src = match e {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return default,
    };
    let mut p = ExprParser { s: src.as_bytes(), i: 0, names };
    match p.expr() {
        Some(v) if p.done() && v.is_finite() => (v.round().max(1.0)) as u32,
        _ => default,
    }
}

struct ExprParser<'a> {
    s: &'a [u8],
    i: usize,
    names: &'a HashMap<String, f64>,
}

impl ExprParser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn done(&mut self) -> bool {
        self.ws();
        self.i == self.s.len()
    }
    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.i < self.s.len() && self.s[self.i] == c {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat(b'+') {
                v += self.term()?;
            } else if self.eat(b'-') {
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        loop {
            if self.eat(b'*') {
                if self.eat(b'*') {
                    v = v.powf(self.unary()?);
                } else {
                    v *= self.power()?;
                }
            } else if self.eat(b'/') {
                let d = self.power()?;
                if d == 0.0 {
                    return None;
                }
                v /= d;
            } else if self.eat(b'%') {
                v %= self.power()?;
            } else {
                return Some(v);
            }
        }
    }
    fn power(&mut self) -> Option<f64> {
        self.unary()
    }
    fn unary(&mut self) -> Option<f64> {
        if self.eat(b'-') {
            return Some(-self.unary()?);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        self.atom()
    }
    fn atom(&mut self) -> Option<f64> {
        self.ws();
        if self.eat(b'(') {
            let v = self.expr()?;
            return self.eat(b')').then_some(v);
        }
        let start = self.i;
        if self.i < self.s.len() && self.s[self.i] == b'$' {
            self.i += 1;
            while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_') {
                self.i += 1;
            }
            let name = std::str::from_utf8(&self.s[start + 1..self.i]).ok()?;
            return Some(*self.names.get(name).unwrap_or(&0.0));
        }
        if self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
            while self.i < self.s.len() && self.s[self.i].is_ascii_alphanumeric() {
                self.i += 1;
            }
            let f = std::str::from_utf8(&self.s[start..self.i]).ok()?.to_string();
            if !self.eat(b'(') {
                return None;
            }
            let mut args = vec![self.expr()?];
            while self.eat(b',') {
                args.push(self.expr()?);
            }
            if !self.eat(b')') {
                return None;
            }
            return Some(match (f.as_str(), args.as_slice()) {
                ("floor", [a]) => a.floor(),
                ("ceil", [a]) => a.ceil(),
                ("round", [a]) => a.round(),
                ("abs", [a]) => a.abs(),
                ("sqrt", [a]) => a.sqrt(),
                ("min", [a, b]) => a.min(*b),
                ("max", [a, b]) => a.max(*b),
                ("pow", [a, b]) => a.powf(*b),
                _ => return None,
            });
        }
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || matches!(self.s[self.i], b'.' | b'e' | b'E'))
        {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn std140_offsets_and_packing() {
        let p = build_effect("uniform float a; uniform vec3 b; uniform vec2 c; uniform vec3 arr[2]; uniform int n; uniform bool f;\nvoid main() { fragColor = vec4(a + b.x + c.x + arr[1].y + float(n) + (f ? 1.0 : 0.0)); }").unwrap();
        let o = |n: &str| p.uniform(n).unwrap().offset;
        assert_eq!((o("a"), o("b"), o("c"), o("arr"), o("n"), o("f")), (0, 16, 32, 48, 80, 84));
        let mut blk = p.block();
        p.write(&mut blk, p.uniform("arr").unwrap(), &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let f = |b: &[u8], at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        assert_eq!(
            (f(&blk, 48), f(&blk, 52), f(&blk, 56), f(&blk, 64), f(&blk, 68), f(&blk, 72)),
            (1.0, 2.0, 3.0, 4.0, 5.0, 6.0)
        );
        p.write(&mut blk, p.uniform("b").unwrap(), &[0.5]);
        assert_eq!((f(&blk, 16), f(&blk, 20), f(&blk, 24)), (0.5, 0.5, 0.5));
        p.write(&mut blk, p.uniform("f").unwrap(), &[1.0]);
        assert_eq!(i32::from_le_bytes(blk[84..88].try_into().unwrap()), 1);
    }

    #[test]
    fn numbers_and_defaults() {
        assert_eq!(parse_numbers("vec2(1.0, -1.0)"), Some(vec![1.0, -1.0]));
        assert_eq!(parse_numbers(" 0.5 "), Some(vec![0.5]));
        assert_eq!(parse_numbers("2.0f"), Some(vec![2.0]));
        assert_eq!(parse_numbers("true"), Some(vec![1.0]));
        assert_eq!(parse_numbers("#ff0000"), None);
        let d = comment_defaults("uniform vec2 direction; // = vec2(1.0, -1.0)\nuniform float s; // = 0.5;\n");
        assert_eq!(d["direction"], "vec2(1.0, -1.0)");
        assert_eq!(d["s"], "0.5");
    }

    #[test]
    fn isf_sizes() {
        let names: HashMap<String, f64> = [("WIDTH".to_string(), 64.0), ("HEIGHT".to_string(), 32.0)].into();
        let s = |e: &str| isf_size(Some(&serde_json::Value::String(e.into())), &names, 7);
        assert_eq!(s("$WIDTH/16.0"), 4);
        assert_eq!(s("floor($HEIGHT*0.5)"), 16);
        assert_eq!(s("max(1.0, $WIDTH - 100)"), 1);
        assert_eq!(s("$WIDTH +"), 7);
    }

    #[test]
    fn data_uris() {
        let base = std::path::Path::new("/tmp");
        assert_eq!(load_source("data:,void%20main()", base).unwrap().0, "void main()");
        assert_eq!(load_source("data:text/plain;base64,dm9pZCBtYWluKCk=", base).unwrap().0, "void main()");
    }
}
