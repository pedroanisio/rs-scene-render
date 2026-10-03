//! Frame fingerprints, so `render --changed-only` and `watch` render only the frames an edit
//! changed.
//!
//! A frame's fingerprint hashes what decides its pixels: the evaluated frame (every node's
//! animated values, transforms and timing), the source text of every element it draws (its
//! static attributes), the document outside the composition (assets, paints, effects,
//! symbols, project, colour management), the files those name, and the render settings. When
//! the scene uses motion blur or time effects a frame also shows other times, so the whole
//! composition joins the shared part and any edit renders every frame again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sr_eval::FrameGraph;
use sr_model::element::Element;

/// The source text of the element starting at byte `offset` of `text` (at its `<`): up to the
/// end of its start tag when it closes itself, else to the end of its matching end tag.
pub fn element_span(text: &str, offset: usize) -> Option<&str> {
    let b = text.as_bytes();
    if b.get(offset) != Some(&b'<') {
        return None;
    }
    let mut depth = 0usize;
    let mut i = offset;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->")? + 3;
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i += rest.find("]]>")? + 3;
            continue;
        }
        if rest.starts_with("<?") {
            i += rest.find("?>")? + 2;
            continue;
        }
        let closing = rest.starts_with("</");
        // the end of this tag, skipping quoted attribute values
        let mut j = i + 1;
        let mut quote = None;
        while j < b.len() {
            match (quote, b[j]) {
                (None, b'"') | (None, b'\'') => quote = Some(b[j]),
                (Some(q), c) if c == q => quote = None,
                (None, b'>') => break,
                _ => {}
            }
            j += 1;
        }
        if j >= b.len() {
            return None;
        }
        let self_closing = b[j - 1] == b'/';
        if closing {
            depth = depth.checked_sub(1)?;
        } else if !self_closing {
            depth += 1;
        }
        i = j + 1;
        if depth == 0 {
            return Some(&text[offset..i]);
        }
    }
    None
}

/// Byte range of the composition in `text`, when there is one.
fn composition_range(text: &str) -> Option<(usize, usize)> {
    let start = text.find("<composition")?;
    let span = element_span(text, start)?;
    Some((start, start + span.len()))
}

fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for &c in bytes {
        h ^= c as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

const SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// Effects that make a frame show the scene at other times.
const TIME_EFFECTS: &[&str] = &["posterize-time", "echo", "pixel-motion-blur"];

/// Local input files named by the document and by the documents it includes, including every declared
/// sequence frame.
pub fn files(path: &Path, doc: &sr_model::Document) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    named(path, doc, &mut files, &mut vec![path.canonicalize().unwrap_or_else(|_| path.to_path_buf())])?;
    files.sort();
    files.dedup();
    Ok(files)
}

/// Inputs before and after parameter bindings, variants, data rows and includes are resolved.
/// Keep the authored inputs too: changing a data file or included document can select new assets.
pub fn effective_files(path: &Path, doc: &sr_model::Document, p: &sr_eval::Program) -> std::io::Result<Vec<PathBuf>> {
    let mut files = files(path, doc)?;
    for (i, scene) in std::iter::once(&p.scene).chain(p.includes.iter().map(|(_, scene)| scene)).enumerate() {
        let base = p.base_dirs.get(i).map(PathBuf::as_path).unwrap_or_else(|| doc.base_dir());
        scene_inputs(scene, base, &mut files)?;
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// Adds the files `doc` at `path` names to `files`, then those of the documents it includes that are not in
/// `seen` yet: they are loaded when the scene is evaluated, so the main document does not name their files.
fn named(
    path: &Path,
    doc: &sr_model::Document,
    files: &mut Vec<PathBuf>,
    seen: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    let base = path.parent().unwrap_or(Path::new("."));
    for inc in scene_inputs(&doc.scene, base, files)? {
        // one file has many spellings (`parts/../main.xml`): documents that include each other must end
        let real = inc.canonicalize().unwrap_or_else(|_| inc.clone());
        if seen.contains(&real) {
            continue;
        }
        seen.push(real);
        if let Ok(doc) = sr_model::load_file(&inc, &sr_model::LoadOptions::without_assets()) {
            named(&inc, &doc, files, seen)?;
        }
    }
    Ok(())
}

struct NumberedInput<'a> {
    kind: &'static str,
    src: &'a str,
    first: i128,
    last: i128,
    step: i128,
    format: Option<String>,
}
fn numbered_input(e: &dyn Element) -> Option<NumberedInput<'_>> {
    if let Some(s) = e.as_any().downcast_ref::<sr_model::model::ImageSequenceAsset>() {
        Some(NumberedInput {
            kind: "image",
            src: &s.src,
            first: s.first as i128,
            last: s.last as i128,
            step: s.step.max(1) as i128,
            format: None,
        })
    } else if let Some(s) = e.as_any().downcast_ref::<sr_model::model::MeshSequenceAsset>() {
        Some(NumberedInput {
            kind: "mesh",
            src: &s.src,
            first: s.first.get() as i128,
            last: s.last.get() as i128,
            step: 1,
            format: s.format.map(|v| v.to_string()),
        })
    } else if let Some(s) = e.as_any().downcast_ref::<sr_model::model::VolumeAsset>() {
        Some(NumberedInput {
            kind: "volume",
            src: &s.src,
            first: s.first?.get() as i128,
            last: s.last?.get() as i128,
            step: 1,
            format: None,
        })
    } else {
        None
    }
}

fn scene_inputs(
    scene: &sr_model::model::Scene,
    base: &Path,
    files: &mut Vec<PathBuf>,
) -> std::io::Result<Vec<PathBuf>> {
    use sr_model::assets::{input_uri_attributes, resolve, sequence_frame, Resolved};
    // The validator skips huge sequences with a warning. Dependency collection cannot
    // silently skip them: edits to an omitted frame would leave incremental output stale.
    // Bound the total work before expanding this scene, including included/bound inputs.
    const MAX_SEQUENCE_INPUTS: i128 = 1_000_000;
    let mut count = files.len() as i128;
    let mut error = None;
    sr_model::element::walk(scene, &mut |e| {
        if error.is_some() {
            return;
        }
        if let Some(seq) = numbered_input(e) {
            let frames = if seq.last >= seq.first { (seq.last - seq.first) / seq.step + 1 } else { 0 };
            count += frames;
            if count > MAX_SEQUENCE_INPUTS {
                error=Some(std::io::Error::new(std::io::ErrorKind::InvalidInput,format!("{} sequence {:?} ({frames} frames) exceeds the dependency limit of {MAX_SEQUENCE_INPUTS} inputs; narrow its range",seq.kind,seq.src)));
            }
        }
    });
    if let Some(error) = error {
        return Err(error);
    }
    let mut includes = Vec::new();
    let mut imported = Vec::new();
    let mut add = |uri: &str| {
        if let Resolved::Local(path) = resolve(uri, base) {
            files.push(path);
        }
    };
    sr_model::element::walk(scene, &mut |e| {
        // Discover dependencies by asset role. A buffer or image named *.gltf is
        // still opaque data; an explicit mesh format overrides its extension.
        let mut discover = |uri: &str, format: Option<&str>| {
            if let Resolved::Local(path) = resolve(uri, base) {
                match sr_3d::import::dependencies_as(&path, format) {
                    Ok(dependencies) => imported.extend(dependencies),
                    Err(message) => {
                        error = Some(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("{}: {message}", path.display()),
                        ))
                    }
                }
            }
        };
        if let Some(mesh) = e.as_any().downcast_ref::<sr_model::model::MeshAsset>() {
            let format = mesh.format.map(|f| f.to_string());
            for uri in std::iter::once(&mesh.src).chain(mesh.proxy.iter()) {
                discover(uri, format.as_deref());
            }
        }
        if let Some(seq) = numbered_input(e).filter(|s| s.kind == "mesh") {
            for frame in seq.first..=seq.last {
                if let Some(uri) = sequence_frame(seq.src, frame as i64) {
                    discover(&uri, seq.format.as_deref());
                }
            }
        }
        if let Some(sr_model::element::AttrValue::Str(uri)) = e.get_attr("materialX") {
            discover(&uri, Some("mtlx"));
        }
        if let Some(inc) = e.as_any().downcast_ref::<sr_model::model::Include>() {
            if let Resolved::Local(path) = resolve(&inc.src, base) {
                includes.push(path);
            }
        }
        let sequence = numbered_input(e);
        if let Some(seq) = &sequence {
            // i128 also handles a step or a final increment beyond the i64 frame range.
            let mut frame = seq.first;
            while frame <= seq.last {
                if let Some(uri) = sequence_frame(seq.src, frame as i64) {
                    add(&uri);
                }
                frame += seq.step.max(1);
            }
        }
        // Share the validator's schema-derived inputs (including transition shaders,
        // proxies and colour configurations), rather than maintaining a second list.
        for &attr in input_uri_attributes() {
            if attr == "src" && sequence.is_some() {
                continue;
            }
            if let Some(sr_model::element::AttrValue::Str(s)) = e.get_attr(attr) {
                add(&s);
            }
        }
    });
    if let Some(error) = error {
        return Err(error);
    }
    files.extend(imported);
    Ok(includes)
}

/// A file's size, modification time and, on Unix, status-change time (seconds, nanoseconds): a file put
/// back with its old modification time (`cp -p`) still differs in the last.
pub type Stamp = (u64, Option<std::time::SystemTime>, (i64, i64));

/// A file's stamp, when it exists.
pub fn stamp(f: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(f).ok()?;
    #[cfg(unix)]
    let changed = {
        use std::os::unix::fs::MetadataExt;
        (m.ctime(), m.ctime_nsec())
    };
    #[cfg(not(unix))]
    let changed = (0, 0);
    Some((m.len(), m.modified().ok(), changed))
}

/// What a frame fingerprint keeps of a file: the hash of its content while it is small (a file replaced by
/// one of the same size and time is seen, a file merely touched is not), else its stamp.
fn file_print(f: &Path) -> String {
    const SMALL: u64 = 1 << 20;
    match std::fs::metadata(f) {
        Ok(m) if m.is_file() && m.len() <= SMALL => match std::fs::read(f) {
            Ok(b) => format!("{}:{:016x}", b.len(), fnv(&b, SEED)),
            Err(e) => e.to_string(),
        },
        _ => format!("{:?}", stamp(f)),
    }
}

/// Directories system fonts are installed in.
fn font_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = if cfg!(target_os = "macos") {
        ["/Library/Fonts", "/System/Library/Fonts", "/Network/Library/Fonts"].map(PathBuf::from).to_vec()
    } else if cfg!(windows) {
        std::env::var_os("WINDIR").map(|w| PathBuf::from(w).join("Fonts")).into_iter().collect()
    } else {
        ["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from).to_vec()
    };
    if let Some(home) = home {
        let own: &[&str] = if cfg!(target_os = "macos") {
            &["Library/Fonts"]
        } else if cfg!(windows) {
            &["AppData/Local/Microsoft/Windows/Fonts"]
        } else {
            &[".fonts", ".local/share/fonts"]
        };
        dirs.extend(own.iter().map(|d| home.join(d)));
    }
    dirs
}

/// Hash of the font files under `dirs` (path, size, modification time): text set in a system font changes
/// when the fonts installed do.
fn font_set(dirs: &[PathBuf]) -> u64 {
    fn walk(dir: &Path, depth: u32, found: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = e.path();
            match e.metadata() {
                Ok(m) if m.is_dir() && depth < 8 => walk(&path, depth + 1, found),
                Ok(m) if m.is_file() => found.push(format!("{}:{}:{:?}", path.display(), m.len(), m.modified().ok())),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    for d in dirs {
        walk(d, 0, &mut found);
    }
    found.sort();
    found.iter().fold(SEED, |h, f| fnv(f.as_bytes(), h))
}

/// The lock on a sidecar while it is rewritten; removed when dropped.
struct Lock(PathBuf);

impl Lock {
    /// Waits for the lock beside `sidecar`; one left by a run that died is taken over after a few seconds.
    fn take(sidecar: &Path) -> std::io::Result<Lock> {
        let path = sidecar.with_extension("lock");
        let started = std::time::Instant::now();
        loop {
            match std::fs::File::options().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Lock(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && started.elapsed().as_secs() < 15 => {
                    let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
                    if age.is_some_and(|a| a.as_secs() >= 5) {
                        let _ = std::fs::remove_file(&path);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Hash of what a renderer is built from (the project, colour management, styles and media dependencies): while it
/// stays the same, a renderer made for an earlier version of the document can render this one.
pub fn setup_key(text: &str, inputs: &[PathBuf]) -> u64 {
    let mut h = SEED;
    for tag in ["<project", "<colorManagement", "<styles"] {
        if let Some(at) = text.find(tag) {
            h = fnv(element_span(text, at).unwrap_or("").as_bytes(), h);
        }
    }
    for f in inputs {
        h = fnv(format!("{}:{:?}", f.display(), stamp(f)).as_bytes(), h);
    }
    h
}

/// The fingerprints of frames rendered with `--changed-only`, by output file, kept beside them.
pub struct Sidecar {
    path: PathBuf,
    pub frames: BTreeMap<String, String>,
    /// The frames as last read from or written to the file: what differs from them is this run's to save.
    saved: BTreeMap<String, String>,
}

impl Sidecar {
    /// The sidecar of outputs written into `dir`.
    pub fn load(dir: &Path) -> Sidecar {
        let path = dir.join(".scene-render-frames.json");
        let frames = Sidecar::read(&path);
        Sidecar { path, saved: frames.clone(), frames }
    }

    fn read(path: &Path) -> BTreeMap<String, String> {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| serde_json::from_value(v["frames"].clone()).ok())
            .unwrap_or_default()
    }

    /// Writes this run's changes into the file as it is now (another run, rendering other frames into the
    /// directory, may have written it since), whole or not at all.
    pub fn save(&mut self) -> std::io::Result<()> {
        if self.frames == self.saved {
            return Ok(());
        }
        let _lock = Lock::take(&self.path)?;
        let mut frames = Sidecar::read(&self.path);
        frames.retain(|k, _| self.frames.contains_key(k) || !self.saved.contains_key(k));
        for (k, v) in &self.frames {
            if self.saved.get(k) != Some(v) {
                frames.insert(k.clone(), v.clone());
            }
        }
        let doc = serde_json::json!({ "version": 1, "frames": frames });
        let tmp = self.path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&tmp, serde_json::to_string_pretty(&doc).expect("serialisable"))
            .and_then(|()| std::fs::rename(&tmp, &self.path))
            .inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            })?;
        self.saved = frames.clone();
        self.frames = frames;
        Ok(())
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.save();
    }
}

/// Fingerprints of frames of one document.
pub struct Fingerprints {
    /// What every frame depends on.
    shared: u64,
    /// Why frames are not told apart (any edit to the composition changes them all).
    pub whole: Option<String>,
}

impl Fingerprints {
    /// Fingerprints for `doc`, whose source is `text`, with resolved `inputs` and render `settings`
    /// (quality, bit depth, variant and anything else that changes pixels).
    pub fn new(text: &str, doc: &sr_model::Document, inputs: &[PathBuf], settings: &str) -> Fingerprints {
        let mut shared = fnv(settings.as_bytes(), SEED);
        shared = fnv(env!("CARGO_PKG_VERSION").as_bytes(), shared);
        // Invalidate fingerprints produced without the inputs selected by bindings and overrides.
        shared = fnv(b"frame-fingerprint-v7", shared);
        shared = fnv(&font_set(&font_dirs()).to_le_bytes(), shared);
        // the document outside the composition
        let (a, b) = composition_range(text).unwrap_or((text.len(), text.len()));
        shared = fnv(&text.as_bytes()[..a], shared);
        shared = fnv(&text.as_bytes()[b..], shared);
        // files the document and those it includes name
        for f in inputs {
            shared = fnv(format!("{}:{}", f.display(), file_print(f)).as_bytes(), shared);
        }
        let whole = if doc.scene.project.motion_blur {
            Some("the scene uses motion blur, so frames show other times".to_string())
        } else if doc
            .scene
            .effects
            .as_ref()
            .is_some_and(|fx| fx.effects.iter().any(|e| TIME_EFFECTS.contains(&e.r#type.as_str())))
        {
            Some("the scene uses time effects, so frames show other times".to_string())
        } else if text.contains("<include") {
            Some("the scene includes other documents".to_string())
        } else {
            None
        };
        if whole.is_some() {
            shared = fnv(&text.as_bytes()[a..b], shared);
        }
        Fingerprints { shared, whole }
    }

    /// The fingerprint of an evaluated frame.
    pub fn frame(&self, text: &str, g: &FrameGraph) -> u64 {
        // Captions and procedural content can change with time even without any nodes.
        let state = (g.time, g.frame, g.size, &g.background, g.camera);
        let mut h = fnv(serde_json::to_string(&state).unwrap_or_default().as_bytes(), self.shared);
        for n in &g.nodes {
            // what the evaluation decided, and the element's own attributes
            h = fnv(serde_json::to_string(n).unwrap_or_default().as_bytes(), h);
            let off = n.elem.loc().offset as usize;
            h = fnv(element_span(text, off).unwrap_or("").as_bytes(), h);
        }
        for tr in &g.transitions {
            h = fnv(serde_json::to_string(tr).unwrap_or_default().as_bytes(), h);
            if let Some(elem) = &tr.elem {
                h = fnv(element_span(text, elem.loc().offset as usize).unwrap_or("").as_bytes(), h);
            }
        }
        fnv(serde_json::to_string(&g.elements).unwrap_or_default().as_bytes(), h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_mesh_and_volume_frames_are_incremental_inputs() {
        let dir = scratch("mesh-volume-sequences");
        std::fs::write(dir.join("frame-0.obj"), "mtllib surface.mtl\nv 0 0 0\n").unwrap();
        std::fs::write(dir.join("surface.mtl"), "newmtl surface\nmap_Kd albedo.png\n").unwrap();
        let doc=sr_model::load_str(r#"<scene version="1.3"><project width="8" height="8" fps="1" duration="2"/><assets><meshSequence id="mesh" src="frame-%d.obj" first="0" last="1" fps="1"/><volume id="smoke" src="density-%d.srvol" first="0" last="1" fps="1"/></assets><composition/></scene>"#,&sr_model::LoadOptions::without_assets()).unwrap();
        let mut inputs = Vec::new();
        scene_inputs(&doc.scene, &dir, &mut inputs).unwrap();
        for name in ["frame-0.obj", "frame-1.obj", "surface.mtl", "albedo.png", "density-0.srvol", "density-1.srvol"] {
            assert!(inputs.contains(&dir.join(name)), "{name} missing from {inputs:?}");
        }
        assert!(!inputs.contains(&dir.join("frame-%d.obj")) && !inputs.contains(&dir.join("density-%d.srvol")));
    }

    #[test]
    fn imported_assets_include_external_buffers_and_textures() {
        let dir = scratch("transitive-assets");
        std::fs::write(dir.join("mesh.gltf"), r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh%20data.bin","byteLength":4}],"images":[{"uri":"colour.png"},{"uri":"data:image/png;base64,AA=="}]}"#).unwrap();
        std::fs::write(dir.join("mesh.obj"), "mtllib mesh.mtl\nv 0 0 0\n").unwrap();
        std::fs::write(dir.join("mesh.mtl"), "newmtl surface\nmap_Kd diffuse.png\nmap_Bump normal.png\n").unwrap();
        std::fs::write(dir.join("surface.mtlx"), r#"<materialx fileprefix="textures/"><image name="colour"><input name="file" type="filename" value="base.png"/></image><standard_surface name="surface"><input name="base_color" nodename="colour"/></standard_surface></materialx>"#).unwrap();
        let path = dir.join("scene.xml");
        let doc = load(
            &path,
            r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/><assets><mesh id="g" src="mesh.gltf"/><mesh id="o" src="mesh.obj"/></assets><materials><material id="m" materialX="surface.mtlx"/></materials><composition/></scene>"#,
        );
        let inputs = files(&path, &doc).unwrap();
        for file in ["mesh data.bin", "colour.png", "mesh.mtl", "diffuse.png", "normal.png", "textures/base.png"] {
            assert!(inputs.contains(&dir.join(file)), "{file} missing from {inputs:?}");
        }
        assert_eq!(inputs.len(), 9, "embedded data is not a file dependency");
    }

    #[test]
    fn explicit_mesh_formats_are_used_for_dependency_discovery() {
        let dir = scratch("explicit-format");
        std::fs::write(
            dir.join("mesh.data"),
            r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"vertices.bin","byteLength":4}]}"#,
        )
        .unwrap();
        let doc = sr_model::load_str(r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/><assets><mesh id="m" src="mesh.data" format="gltf"/></assets><composition/></scene>"#, &sr_model::LoadOptions::without_assets()).unwrap();
        let mut inputs = Vec::new();
        scene_inputs(&doc.scene, &dir, &mut inputs).unwrap();
        assert!(inputs.contains(&dir.join("vertices.bin")), "{inputs:?}");
    }

    #[test]
    fn explicit_format_overrides_conflicting_extension_in_full_discovery() {
        let dir = scratch("conflicting-format");
        std::fs::write(dir.join("mesh.gltf"), "mtllib mesh.mtl\nv 0 0 0\n").unwrap();
        std::fs::write(dir.join("mesh.mtl"), "newmtl surface\nmap_Kd colour.png\n").unwrap();
        let path = dir.join("scene.xml");
        let doc = load(
            &path,
            r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/><assets><mesh id="m" src="mesh.gltf" format="obj"/></assets><composition/></scene>"#,
        );
        let evaluator = sr_eval::Evaluator::new(&doc, &sr_eval::EvalOptions::default()).unwrap();
        let inputs = effective_files(&path, &doc, evaluator.program()).unwrap();
        assert!(inputs.contains(&dir.join("mesh.mtl")));
        assert!(inputs.contains(&dir.join("colour.png")));
    }

    #[test]
    fn spans_end_at_the_matching_end_tag() {
        let t = r#"<a x="1"><b/><!-- <c> --><a y=">">t</a></a><d/>"#;
        assert_eq!(element_span(t, 0), Some(r#"<a x="1"><b/><!-- <c> --><a y=">">t</a></a>"#));
        assert_eq!(element_span(t, 9), Some("<b/>"));
        let inner = t.find(r#"<a y"#).unwrap();
        assert_eq!(element_span(t, inner), Some(r#"<a y=">">t</a>"#));
        assert_eq!(element_span(t, t.find("<d").unwrap()), Some("<d/>"));
        assert_eq!(element_span(t, 1), None, "not at a tag");
    }

    #[test]
    fn element_offsets_point_at_their_tags() {
        let xml = r##"<scene version="1.1"><project width="8" height="8" fps="1" duration="1"/>
<composition><shape id="s" shape="rect" width="2" height="2" fill="#FF0000"><animate property="x"><key time="0" value="0"/><key time="1" value="4"/></animate></shape></composition></scene>"##;
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &sr_eval::EvalOptions::default()).unwrap();
        let g = ev.evaluate(0.0);
        let n = g.nodes.iter().find(|n| &*n.id == "s").unwrap();
        let span = element_span(xml, n.elem.loc().offset as usize).unwrap();
        assert!(span.starts_with("<shape id=\"s\"") && span.ends_with("</shape>"), "{span}");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sr-changes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn load(path: &Path, xml: &str) -> sr_model::Document {
        std::fs::write(path, xml).unwrap();
        sr_model::load_file(path, &sr_model::LoadOptions::without_assets()).unwrap()
    }

    #[test]
    fn files_of_included_documents_are_dependencies() {
        let dir = scratch("includes");
        std::fs::create_dir_all(dir.join("parts/deep")).unwrap();
        let scene = |body: &str| {
            format!(r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/>{body}</scene>"#)
        };
        load(
            &dir.join("parts/deep/leaf.xml"),
            &scene(r#"<assets><image id="i" src="leaf.png" width="8" height="8"/></assets><composition/>"#),
        );
        // includes its parent as well: a cycle must end
        load(
            &dir.join("parts/child.xml"),
            &scene(
                r#"<assets><image id="i" src="logo.png" width="8" height="8"/></assets>
            <composition><include id="a" src="deep/leaf.xml"/><include id="b" src="../main.xml"/></composition>"#,
            ),
        );
        let main = dir.join("main.xml");
        let doc = load(&main, &scene(r#"<composition><include id="c" src="parts/child.xml"/></composition>"#));
        let found = files(&main, &doc).unwrap();
        for f in ["parts/child.xml", "parts/logo.png", "parts/deep/leaf.xml", "parts/deep/leaf.png"] {
            assert!(found.contains(&dir.join(f)), "{f} is missing from {found:?}");
        }
    }

    #[test]
    fn sequence_dependencies_use_the_step_without_integer_overflow() {
        let dir = scratch("sequence-step");
        let main = dir.join("main.xml");
        let doc = load(
            &main,
            r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/>
          <assets><imageSequence id="s" src="f_%d.png" first="-9223372036854775808" last="9223372036854775807" step="9223372036854775807" fps="1" width="8" height="8"/></assets><composition/></scene>"#,
        );
        let found = files(&main, &doc).unwrap();
        assert_eq!(found.len(), 3);
        for frame in [i64::MIN, -1, i64::MAX - 1] {
            assert!(found.contains(&dir.join(format!("f_{frame}.png"))));
        }
    }

    #[test]
    fn bound_and_included_sequences_share_the_dependency_limit() {
        let dir = scratch("sequence-limit");
        let main = dir.join("main.xml");
        let doc = load(
            &main,
            r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/>
          <parameters><param id="end" type="string" default="9223372036854775807"/><bind param="end" target="s" property="last"/></parameters>
          <assets><imageSequence id="s" src="f_%d.png" first="0" last="0" fps="1" width="8" height="8"/></assets><composition/></scene>"#,
        );
        assert_eq!(files(&main, &doc).unwrap().len(), 1);
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        assert!(effective_files(&main, &doc, ev.program()).unwrap_err().to_string().contains("limit"));
        let child = dir.join("child.xml");
        load(&child, &std::fs::read_to_string(&main).unwrap().replace("last=\"0\"", "last=\"9223372036854775807\""));
        let doc = load(
            &main,
            r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/>
          <composition><include id="child" src="child.xml"/></composition></scene>"#,
        );
        assert!(files(&main, &doc).unwrap_err().to_string().contains("limit"));
    }

    #[test]
    fn a_same_size_file_put_back_with_its_old_time_is_a_change() {
        let dir = scratch("stamp");
        let (main, logo) = (dir.join("main.xml"), dir.join("logo.png"));
        let xml = r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/>
          <assets><image id="i" src="logo.png" width="8" height="8"/></assets><composition/></scene>"#;
        let doc = load(&main, xml);
        std::fs::write(&logo, b"aaaa").unwrap();
        let time = std::fs::metadata(&logo).unwrap().modified().unwrap();
        let inputs = files(&main, &doc).unwrap();
        let before = (Fingerprints::new(xml, &doc, &inputs, "").shared, setup_key(xml, &inputs));
        // as `cp -p` of another file of the same size does
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(&logo, b"bbbb").unwrap();
        std::fs::File::options().write(true).open(&logo).unwrap().set_modified(time).unwrap();
        assert_eq!(std::fs::metadata(&logo).unwrap().modified().unwrap(), time);
        assert_ne!(Fingerprints::new(xml, &doc, &inputs, "").shared, before.0);
        if cfg!(unix) {
            assert_ne!(setup_key(xml, &inputs), before.1, "a kept renderer would show the old image");
        }
    }

    #[test]
    fn the_font_set_changes_with_the_fonts_installed() {
        let dir = scratch("fonts");
        std::fs::create_dir_all(dir.join("truetype/a")).unwrap();
        std::fs::write(dir.join("truetype/a/A.ttf"), b"one").unwrap();
        let dirs = [dir.clone(), dir.join("absent")];
        let before = font_set(&dirs);
        assert_eq!(font_set(&dirs), before);
        std::fs::write(dir.join("truetype/B.ttf"), b"two").unwrap();
        let added = font_set(&dirs);
        assert_ne!(added, before);
        std::fs::write(dir.join("truetype/a/A.ttf"), b"another").unwrap();
        assert_ne!(font_set(&dirs), added);
    }

    #[test]
    fn sidecars_of_two_shards_keep_each_others_frames() {
        let dir = scratch("shards");
        let (mut a, mut b) = (Sidecar::load(&dir), Sidecar::load(&dir));
        a.frames.insert("f_000.png".into(), "a0".into());
        b.frames.insert("f_050.png".into(), "b0".into());
        a.save().unwrap();
        b.save().unwrap();
        a.frames.insert("f_001.png".into(), "a1".into());
        a.frames.remove("f_000.png");
        a.save().unwrap();
        b.frames.insert("f_051.png".into(), "b1".into());
        drop(a);
        drop(b);
        let kept: Vec<String> = Sidecar::load(&dir).frames.keys().cloned().collect();
        assert_eq!(kept, ["f_001.png", "f_050.png", "f_051.png"]);
        // written whole or not at all: nothing but the sidecar is left
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, [".scene-render-frames.json"]);
    }
}
