//! # sr-resolve
//!
//! Fills the caches that rendering reads. A scene-render document declares
//! provider-made media as `<generated>` assets and transcribed captions as
//! `<captionTrack transcribe="…">`. The renderer never calls a provider: it reads
//! the cache and refuses unless its SHA-256 equals `cacheSha256`, which keeps
//! renders deterministic. [`resolve`] is the separate step that makes those
//! caches and pins their digests:
//!
//! 1. Each target becomes a [`protocol::Request`] whose key hashes everything
//!    that decides the result (provider, model, prompt, voice, language, seed,
//!    size, the timeline and, for transcriptions, the input audio). Paths and ids
//!    are left out, so the same request finds the same result anywhere.
//! 2. A target is up to date when the sidecar `<cache>.resolve.json` records
//!    the same key and the cache's digest, and `cacheSha256` matches. Otherwise
//!    the result comes from the store (`SR_RESOLVE_STORE`, default
//!    `~/.cache/scene-render/resolve`) when an earlier run made it, and from the
//!    provider when none did.
//! 3. Generated media resolve first, so a transcription can take its audio from
//!    speech made in the same run. A transcriber hears the track as it plays in
//!    the scene's mix (placement, trims, speed, volume and effects), from
//!    composition time 0, so its word times are composition times.
//! 4. `cacheSha256` is written into the document text in place; nothing else
//!    in the file changes.
//!
//! With `check`, nothing is made or written: the report says which targets are
//! stale, for CI.
//!
//! Caches are written only inside the document's folder (or the folder named by
//! `SR_RESOLVE_ROOT`), so a document cannot direct a provider's output elsewhere.

pub mod doc;
pub mod protocol;
pub mod providers;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sr_model::model as m;

use protocol::{file_sha256, Request, Response, Task, Timeline, PROTOCOL};

/// What to do.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Report stale targets; make and write nothing.
    pub check: bool,
    /// Make every target again, ignoring caches and the store.
    pub force: bool,
    /// Only these ids (all when empty).
    pub only: Vec<String>,
    /// Allow providers that send content to a cloud service.
    pub allow_cloud: bool,
    /// Result store (`None`: `SR_RESOLVE_STORE`, else the user cache folder; `Some("")`: none).
    pub store: Option<PathBuf>,
}

/// The outcome of one target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Cache, sidecar and digest agree with the request.
    UpToDate,
    /// The cache was right; only `cacheSha256` was written.
    Pinned,
    /// Copied from the store.
    Restored,
    /// Made by the provider.
    Made,
    /// Needs work (check mode).
    Stale,
    /// Failed.
    Error,
}

/// One resolved target.
#[derive(Clone, Debug, Serialize)]
pub struct Resolution {
    /// Element id.
    pub id: String,
    /// `generated` or `captionTrack`.
    pub element: &'static str,
    pub provider: String,
    pub cache: String,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// The record kept next to each cache.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sidecar {
    pub key: String,
    pub sha256: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub request: Request,
}

fn sidecar_path(cache: &Path) -> PathBuf {
    let mut s = cache.as_os_str().to_owned();
    s.push(".resolve.json");
    PathBuf::from(s)
}

fn read_sidecar(cache: &Path) -> Option<Sidecar> {
    serde_json::from_slice(&std::fs::read(sidecar_path(cache)).ok()?).ok()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    replace_atomic(path, |f| f.write_all(bytes)).map_err(|e| format!("{}: {e}", path.display()))
}

/// Never follow an existing temporary-file or destination symlink while writing.
fn replace_atomic(path: &Path, write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".part-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let tmp = PathBuf::from(tmp);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let result = write(&mut f);
    drop(f);
    let result = result.and_then(|_| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn copy_atomic(src: &Path, path: &Path) -> std::io::Result<()> {
    let mut input = std::fs::File::open(src)?;
    replace_atomic(path, |f| std::io::copy(&mut input, f).map(|_| ()))
}

/// The result store.
fn store_dir(o: &Options) -> Option<PathBuf> {
    match &o.store {
        Some(p) if p.as_os_str().is_empty() => None,
        Some(p) => Some(p.clone()),
        None => match std::env::var_os("SR_RESOLVE_STORE") {
            Some(p) if p.is_empty() => None,
            Some(p) => Some(PathBuf::from(p)),
            None => std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
                .map(|c| c.join("scene-render").join("resolve")),
        },
    }
}

fn store_entry(store: &Path, req: &Request, key: &str) -> PathBuf {
    let ext = Path::new(&req.output).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    store.join(format!("{key}.{ext}"))
}

/// A scratch folder removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Result<Scratch, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "scene-render-resolve-{}-{}-{tag}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        Ok(Scratch(d))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A path made absolute with its `.` and `..` steps applied, without reading the file system.
fn lexical(p: &Path) -> Result<PathBuf, String> {
    use std::path::Component;
    let abs = std::path::absolute(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    Ok(out)
}

/// Resolve existing ancestors, including symlinks, while allowing new cache directories.
/// A dangling symlink is an error rather than a missing directory we may create through.
fn physical(p: &Path) -> Result<PathBuf, String> {
    match std::fs::symlink_metadata(p) {
        Ok(_) => p.canonicalize().map_err(|e| format!("{}: {e}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = p.parent().ok_or_else(|| format!("{}: {e}", p.display()))?;
            let name = p.file_name().ok_or_else(|| format!("{}: {e}", p.display()))?;
            Ok(physical(parent)?.join(name))
        }
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

/// The file a cache attribute names. Caches are written, so they must lie inside `root` (the
/// project folder): absolute paths and `..` steps that leave it are refused.
fn local_in(src: &str, base: &Path, root: &Path) -> Result<PathBuf, String> {
    let p = match sr_model::assets::resolve(src, base) {
        sr_model::assets::Resolved::Local(p) => p,
        sr_model::assets::Resolved::Remote(u) => return Err(format!("{u}: a remote cache cannot be written")),
    };
    let (file, root) = (lexical(&p)?, lexical(root)?);
    if file == root || !file.starts_with(&root) {
        return Err(format!(
            "the cache is outside the project folder {}; keep caches inside it, or name the folder they may be \
             written in with SR_RESOLVE_ROOT",
            root.display()
        ));
    }
    let (file, real_root) = (physical(&file)?, physical(&root)?);
    if file == real_root || !file.starts_with(&real_root) {
        return Err(format!("the cache resolves outside the project folder {}", root.display()));
    }
    Ok(file)
}

/// [`local_in`] the document's folder, or `SR_RESOLVE_ROOT` when it is set.
fn local(src: &str, base: &Path) -> Result<PathBuf, String> {
    match std::env::var_os("SR_RESOLVE_ROOT").filter(|r| !r.is_empty()) {
        Some(root) => local_in(src, base, Path::new(&root)),
        None => local_in(src, base, base),
    }
}

/// Where a generated asset plays: composition time of its start in the first audio track that
/// plays it (`start − clipIn`).
fn track_start(scene: &m::Scene, asset: &str) -> Option<f64> {
    scene.audio_mix.as_ref()?.children.iter().find_map(|c| match c {
        m::AudioMixChild::AudioTrack(t) if t.asset == asset => Some(t.start.get() - t.clip_in.get()),
        _ => None,
    })
}

struct Target {
    element: &'static str,
    id: String,
    cache: PathBuf,
    cache_attr: String,
    pinned: Option<String>,
    req: Request,
}

/// Brings one target up to date (or, in check mode, reports whether it is).
fn settle(t: &Target, o: &Options, work: &Path) -> (Resolution, Option<String>) {
    let mut r = Resolution {
        id: t.id.clone(),
        element: t.element,
        provider: t.req.provider.clone(),
        cache: t.cache_attr.clone(),
        status: Status::Error,
        sha256: None,
        message: String::new(),
        notes: Vec::new(),
    };
    // The extension selects the provider's output format and must participate in both
    // the request key and store path, before any cache or sidecar is consulted.
    let req = Request { output: t.cache.display().to_string(), ..t.req.clone() };
    let key = req.key();
    let pinned_ok = |sha: &str| t.pinned.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(sha));
    // 1. cache and sidecar agree with the request
    if !o.force {
        if let (Some(sc), true) = (read_sidecar(&t.cache), t.cache.is_file()) {
            if sc.key == key && file_sha256(&t.cache).ok().as_deref() == Some(sc.sha256.as_str()) {
                r.sha256 = Some(sc.sha256.clone());
                if pinned_ok(&sc.sha256) {
                    r.status = Status::UpToDate;
                    return (r, None);
                }
                if o.check {
                    r.status = Status::Stale;
                    r.message = "cacheSha256 differs from the cache".into();
                    return (r, None);
                }
                r.status = Status::Pinned;
                return (r, Some(sc.sha256));
            }
        }
    }
    if o.check {
        r.status = Status::Stale;
        r.message = if t.cache.is_file() {
            "the cache was made from a different request".into()
        } else {
            "the cache is missing".into()
        };
        return (r, None);
    }
    let store = store_dir(o);
    let fail = |mut r: Resolution, m: String| {
        r.status = Status::Error;
        r.message = m;
        (r, None)
    };
    if let Some(dir) = t.cache.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return fail(r, format!("{}: {e}", dir.display()));
        }
    }
    // 2. the store
    let version;
    let stored = store.as_ref().filter(|_| !o.force).map(|s| store_entry(s, &req, &key)).and_then(|path| {
        // A shared result has the same integrity requirements as a project cache.
        // Missing, stale or damaged entries are misses, never new content to pin.
        read_sidecar(&path)
            .filter(|side| side.key == key && file_sha256(&path).ok().as_deref() == Some(side.sha256.as_str()))
            .map(|side| (path, side))
    });
    if let Some((src, side)) = stored {
        if let Err(e) = copy_atomic(&src, &t.cache) {
            return fail(r, format!("{}: {e}", t.cache.display()));
        }
        version = side.version;
        r.status = Status::Restored;
    } else {
        // 3. the provider
        let provider = match providers::find(&t.req.provider) {
            Ok(p) => p,
            Err(e) => return fail(r, e),
        };
        if provider.cloud() && !o.allow_cloud {
            return fail(
                r,
                format!(
                    "provider {} sends the prompt to a cloud service; run with --allow-cloud to allow it",
                    t.req.provider
                ),
            );
        }
        let ext = t.cache.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "bin".into());
        let out = work.join(format!("result.{ext}"));
        let provider_req =
            Request { output: out.display().to_string(), workdir: work.display().to_string(), ..req.clone() };
        let resp: Response = match provider.run(&provider_req) {
            Ok(r) => r,
            Err(e) => return fail(r, format!("{}: {e}", t.req.provider)),
        };
        if !out.is_file() {
            return fail(r, format!("{} reported success but wrote nothing", t.req.provider));
        }
        if let Err(e) = copy_atomic(&out, &t.cache) {
            return fail(r, format!("{}: {e}", t.cache.display()));
        }
        version = resp.version;
        r.notes = resp.notes;
        r.status = Status::Made;
    }
    let sha = match file_sha256(&t.cache) {
        Ok(s) => s,
        Err(e) => return fail(r, format!("{}: {e}", t.cache.display())),
    };
    let side = Sidecar { key, sha256: sha.clone(), provider: t.req.provider.clone(), version, request: req.clone() };
    let side_json = serde_json::to_vec_pretty(&side).expect("json");
    if let Err(e) = write_atomic(&sidecar_path(&t.cache), &side_json) {
        return fail(r, e);
    }
    if let (Some(s), Status::Made) = (&store, r.status) {
        let entry = store_entry(s, &req, &side.key);
        let _ = std::fs::create_dir_all(s)
            .and_then(|_| copy_atomic(&t.cache, &entry))
            .and_then(|_| replace_atomic(&sidecar_path(&entry), |f| std::io::Write::write_all(f, &side_json)));
    }
    r.sha256 = Some(sha.clone());
    let pin = (!pinned_ok(&sha)).then_some(sha);
    (r, pin)
}

/// The scene's audio format.
fn audio_format(scene: &m::Scene) -> (u32, u16) {
    scene
        .audio_mix
        .as_ref()
        .map(|a| (a.sample_rate as u32, a.bit_depth.to_string().parse().unwrap_or(24)))
        .unwrap_or((48000, 24))
}

struct DocumentLock(std::fs::File);
impl Drop for DocumentLock {
    fn drop(&mut self) {
        // Explicit unlock also releases the lock if a concurrent fork briefly
        // inherited this descriptor before closing it on exec.
        let _ = self.0.unlock();
    }
}

/// Resolves a document's generated media and transcriptions, rewriting its `cacheSha256`
/// attributes in place.
pub fn resolve(path: &Path, o: &Options) -> Result<Vec<Resolution>, String> {
    // Keep the lock file in place: unlinking it would let another writer lock a
    // different inode while a waiter still holds the old one.
    let _lock = if o.check {
        None
    } else {
        let canonical = path.canonicalize().map_err(|e| e.to_string())?;
        let mut name = canonical.into_os_string();
        name.push(".resolve.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(name)
            .map_err(|e| format!("cannot lock {}: {e}", path.display()))?;
        file.try_lock().map_err(|e| format!("{}: another resolver holds the document lock: {e}", path.display()))?;
        Some(DocumentLock(file))
    };
    let mut text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let original = text.clone();
    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let base = if base.as_os_str().is_empty() { PathBuf::from(".") } else { base };
    let load = |text: &str| {
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(base.clone()) };
        sr_model::load_str(text, &opts).map_err(|e| format!("{}: {e}", path.display()))
    };
    let wanted = |id: &str| o.only.is_empty() || o.only.iter().any(|x| x == id);
    let mut out = Vec::new();

    // generated media
    let doc = load(&text)?;
    let scene = &doc.scene;
    let (rate, bits) = audio_format(scene);
    let timeline = |start| Timeline { project_duration: scene.project.duration.get(), start };
    let mut targets = Vec::new();
    for a in scene.assets.iter().flat_map(|a| a.children.iter()) {
        let m::AssetsChild::Generated(g) = a else { continue };
        if !wanted(&g.id) {
            continue;
        }
        let cache = match local(&g.cache, &base) {
            Ok(c) => c,
            Err(e) => {
                out.push(error_row(&g.id, "generated", &g.provider, &g.cache, e));
                continue;
            }
        };
        targets.push(Target {
            element: "generated",
            id: g.id.clone(),
            cache,
            cache_attr: g.cache.clone(),
            pinned: Some(g.cache_sha256.to_string()),
            req: Request {
                protocol: PROTOCOL,
                task: Some(Task::Generate),
                kind: g.kind.to_string(),
                id: g.id.clone(),
                provider: g.provider.clone(),
                model: g.model.clone(),
                prompt: g.prompt.clone(),
                voice: g.voice.clone(),
                language: g.language.as_ref().map(|l| l.to_string()),
                seed: g.seed,
                width: g.width,
                height: g.height,
                fps: g.fps.map(|f| f.as_f64()),
                duration: g.duration.map(|d| d.get()),
                sample_rate: rate,
                bit_depth: bits,
                timeline: timeline(track_start(scene, &g.id)),
                base_dir: base.display().to_string(),
                ..Default::default()
            },
        });
    }
    for t in &targets {
        let work = Scratch::new(&t.id)?;
        let (r, pin) = settle(t, o, &work.0);
        if let Some(sha) = pin {
            text = doc::set_attr(&text, "generated", &t.id, "cacheSha256", &sha)?;
        }
        out.push(r);
    }

    // map tiles from online services, for every view the maps show
    let doc = load(&text)?;
    let tile_assets: Vec<&m::TilesAsset> = doc
        .scene
        .assets
        .iter()
        .flat_map(|a| a.children.iter())
        .filter_map(|c| match c {
            m::AssetsChild::Tiles(t) if t.url.is_some() && wanted(&t.id) => Some(t),
            _ => None,
        })
        .collect();
    for t in tile_assets {
        let url = t.url.clone().unwrap_or_default();
        let Some(cache_attr) = t.cache.clone() else {
            out.push(error_row(&t.id, "tiles", "tiles", "", "tiles with @url need @cache".into()));
            continue;
        };
        let cache = match local(&cache_attr, &base) {
            Ok(c) => c,
            Err(e) => {
                out.push(error_row(&t.id, "tiles", "tiles", &cache_attr, e));
                continue;
            }
        };
        let set = match tile_set(&doc, t) {
            Ok(s) => s,
            Err(e) => {
                out.push(error_row(&t.id, "tiles", "tiles", &cache_attr, e));
                continue;
            }
        };
        let list: Vec<String> = set.iter().map(|(z, x, y)| format!("{z}/{x}/{y}")).collect();
        let target = Target {
            element: "tiles",
            id: t.id.clone(),
            cache,
            cache_attr,
            pinned: t.cache_sha256.as_ref().map(|s| s.to_string()),
            req: Request {
                protocol: PROTOCOL,
                task: Some(Task::Generate),
                kind: "tiles".into(),
                id: t.id.clone(),
                provider: "tiles".into(),
                model: url,
                prompt: Some(list.join(" ")),
                sample_rate: rate,
                bit_depth: bits,
                timeline: Timeline { project_duration: doc.scene.project.duration.get(), start: None },
                base_dir: base.display().to_string(),
                ..Default::default()
            },
        };
        let work = Scratch::new(&t.id)?;
        let (r, pin) = settle(&target, o, &work.0);
        if let Some(sha) = pin {
            text = doc::set_attr(&text, "tiles", &target.id, "cacheSha256", &sha)?;
        }
        out.push(r);
    }

    // transcriptions, from the mix as it now plays: the composition's tracks in composition time, an
    // output's own tracks alone in output time
    let doc = load(&text)?;
    let scene = &doc.scene;
    let mut tracks: Vec<(&m::CaptionTrack, Option<&m::Output>)> = scene
        .captions
        .iter()
        .flat_map(|c| c.caption_tracks.iter())
        .filter(|t| t.transcribe.is_some() && wanted(&t.id))
        .map(|t| (t, None))
        .collect();
    for o in &scene.outputs {
        tracks.extend(o.children.iter().filter_map(|c| match c {
            m::OutputChild::CaptionTrack(t) if t.transcribe.is_some() && wanted(&t.id) => Some((t, Some(o))),
            _ => None,
        }));
    }
    if !tracks.is_empty() {
        let scene_mix = if tracks.iter().any(|(_, o)| o.is_none()) { mix(&doc) } else { Err(String::new()) };
        for (tr, owner) in tracks {
            let own_mix;
            let (mixed, duration) = match owner {
                None => (&scene_mix, scene.project.duration.get()),
                Some(o) => {
                    own_mix =
                        sr_deliver::segment_audio::own_tracks(&doc, o).map_err(|e| format!("output {}: {e}", o.path));
                    let d = own_mix
                        .as_ref()
                        .ok()
                        .and_then(|(r, n)| n.values().next().map(|b| b[0].len() as f64 / *r as f64));
                    (&own_mix, d.unwrap_or(0.0))
                }
            };
            let source = tr.transcribe.clone().unwrap_or_default();
            let provider = tr.provider.clone();
            let Some(cache_attr) = tr.cache.clone() else {
                out.push(error_row(&tr.id, "captionTrack", &provider, "", "transcribe needs @cache".into()));
                continue;
            };
            let cache = match local(&cache_attr, &base) {
                Ok(c) => c,
                Err(e) => {
                    out.push(error_row(&tr.id, "captionTrack", &provider, &cache_attr, e));
                    continue;
                }
            };
            let work = Scratch::new(&tr.id)?;
            let input = match mixed {
                Ok((rate, nodes)) => match nodes.get(&source) {
                    Some(buf) => {
                        let wav = work.0.join("input.wav");
                        let mono: Vec<f32> = mono(buf);
                        if let Err(e) =
                            sr_audio::wav::write(&wav, &vec![mono], *rate, 24, sr_audio::layout::Layout::Mono, false)
                        {
                            out.push(error_row(&tr.id, "captionTrack", &provider, &cache_attr, e.to_string()));
                            continue;
                        }
                        wav
                    }
                    None => {
                        let msg = format!("audio track {source:?} is not in the mix");
                        out.push(error_row(&tr.id, "captionTrack", &provider, &cache_attr, msg));
                        continue;
                    }
                },
                Err(e) => {
                    out.push(error_row(&tr.id, "captionTrack", &provider, &cache_attr, e.clone()));
                    continue;
                }
            };
            let t = Target {
                element: "captionTrack",
                id: tr.id.clone(),
                cache,
                cache_attr,
                pinned: tr.cache_sha256.as_ref().map(|s| s.to_string()),
                req: Request {
                    protocol: PROTOCOL,
                    task: Some(Task::Transcribe),
                    kind: "captions".into(),
                    id: tr.id.clone(),
                    provider,
                    model: tr.model.clone(),
                    prompt: tr.prompt.clone(),
                    language: Some(tr.language.to_string()),
                    sample_rate: rate,
                    bit_depth: bits,
                    timeline: Timeline { project_duration: duration, start: None },
                    input_sha256: file_sha256(&input).ok(),
                    input: Some(input.display().to_string()),
                    base_dir: base.display().to_string(),
                    ..Default::default()
                },
            };
            let (r, pin) = settle(&t, o, &work.0);
            if let Some(sha) = pin {
                text = doc::set_attr(&text, "captionTrack", &t.id, "cacheSha256", &sha)?;
            }
            out.push(r);
        }
    }
    if !o.check {
        let current = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        if current != original {
            return Err(format!(
                "{} changed during resolution; generated caches were kept, but the document was not overwritten",
                path.display()
            ));
        }
        if original != text {
            write_atomic(path, text.as_bytes())?;
        }
    }
    for id in &o.only {
        if !out.iter().any(|r| &r.id == id) {
            return Err(format!("{id}: no generated asset or transcribed caption track has this id"));
        }
    }
    Ok(out)
}

/// Every tile a document's maps show from the tiles asset `t` (frame by frame, at the zoom the
/// renderer will draw), within its zoom limits.
fn tile_set(doc: &sr_model::Document, t: &m::TilesAsset) -> Result<std::collections::BTreeSet<(u8, u32, u32)>, String> {
    let ev = sr_eval::Evaluator::new(doc, &sr_eval::EvalOptions::default()).map_err(|e| format!("{e:?}"))?;
    let p = ev.program();
    let url = t.url.as_deref().unwrap_or("").to_ascii_lowercase();
    let raster = [".png", ".jpg", ".jpeg", ".webp"].iter().any(|e| url.split('?').next().unwrap_or("").ends_with(e));
    let size = t.tile_size.map(|s| s as f64).unwrap_or(if raster { 256.0 } else { 512.0 });
    let (zmin, zmax) = (t.min_zoom.get().min(24) as u8, t.max_zoom.get().min(24) as u8);
    if zmin > zmax {
        return Err(format!("tiles {}: minZoom {zmin} is above maxZoom {zmax}", t.id));
    }
    // the maps drawing these tiles, with each basemap's detail
    let mut maps: Vec<(&m::MapAsset, f64)> = Vec::new();
    for a in doc.scene.assets.iter().flat_map(|a| a.children.iter()) {
        if let m::AssetsChild::Map(mp) = a {
            for c in &mp.children {
                if let m::MapAssetChild::Basemap(b) = c {
                    if b.tiles == t.id {
                        maps.push((mp, b.detail));
                    }
                }
            }
        }
    }
    // 3D maps (SREP 10): elevation tiles for a ground's terrain, over each frame's view at the DEM zoom
    // the renderer draws; a globe's basemaps over the whole world, at the zoom its drape draws them
    let map_of = |id: &str| {
        doc.scene.assets.iter().flat_map(|a| a.children.iter()).find_map(|a| match a {
            m::AssetsChild::Map(mp) if mp.id == id => Some(mp),
            _ => None,
        })
    };
    let mut terrains: Vec<(&m::MapAsset, u64)> = Vec::new();
    let mut set = std::collections::BTreeSet::new();
    // the budget holds while the tiles are listed: no view is walked beyond it
    let max = providers::tiles::max_tiles();
    let mut add = |proj: &sr_geo::project::Projection, z: u8| -> Result<(), String> {
        let tiles = sr_geo::tiles::visible_within(proj, z, max).map_err(|_| providers::tiles::over_budget(max))?;
        set.extend(tiles.iter().map(|t| (t.z, t.x, t.y)));
        if set.len() > max {
            return Err(providers::tiles::over_budget(max));
        }
        Ok(())
    };
    for (_, n) in doc.composition_nodes() {
        let m::Node::Object3D(o) = n else { continue };
        let Some(mp) = o.map.as_deref().and_then(map_of) else { continue };
        if o.terrain.as_deref() == Some(t.id.as_str()) {
            terrains.push((mp, o.resolution.clamp(8, 256)));
        }
        if o.primitive == m::Object3DPrimitive::Globe {
            // the drape's frame: the whole world, 2H × H map pixels
            let h = mp.height as f64;
            let (world, c) = sr_geo::view::Map::new(
                sr_geo::view::Kind::Equirectangular,
                None,
                [2.0 * h, h],
                &[],
                0.0,
                Some([0.0, 0.0]),
            );
            let proj = world.projection(&sr_geo::view::View { lon: c[0], lat: c[1], zoom: 0.0, rotation: 0.0 });
            for c in &mp.children {
                if let m::MapAssetChild::Basemap(b) = c {
                    if b.tiles == t.id {
                        let z = sr_eval::geo::basemap_zoom(&proj, size, b.detail, raster).clamp(zmin, zmax);
                        add(&proj, z)?;
                    }
                }
            }
        }
    }
    if maps.is_empty() && terrains.is_empty() {
        return Ok(set);
    }
    let fps = doc.scene.project.fps.as_f64().max(1.0);
    let frames = (doc.scene.project.duration.get() * fps).ceil() as usize;
    for f in 0..=frames {
        let time = f as f64 / fps;
        let g = ev.evaluate(time);
        for (mp, detail) in &maps {
            let cam = sr_eval::geo::camera(p, mp)?;
            let props = g.elements.iter().find(|e| *e.key == *mp.id).map(|e| &e.props);
            let animated = |name: &str| props.and_then(|p| p.get(name)).and_then(sr_eval::Value::as_num);
            let view = sr_eval::geo::view(&cam, mp, &animated, time);
            let proj = cam.map.projection(&view);
            let z = sr_eval::geo::basemap_zoom(&proj, size, *detail, raster).clamp(zmin, zmax);
            add(&proj, z)?;
        }
        for (mp, res) in &terrains {
            let cam = sr_eval::geo::camera(p, mp)?;
            let props = g.elements.iter().find(|e| *e.key == *mp.id).map(|e| &e.props);
            let animated = |name: &str| props.and_then(|p| p.get(name)).and_then(sr_eval::Value::as_num);
            let proj = cam.map.projection(&sr_eval::geo::view(&cam, mp, &animated, time));
            // a DEM pixel per grid cell: 256-pixel tiles at the map zoom less log2(cell), as the renderer
            let cell = (mp.width.max(mp.height) as f64) / *res as f64;
            let zd = (sr_geo::tiles::map_zoom(&proj) + 1.0 - cell.log2()).round().clamp(zmin as f64, zmax as f64) as u8;
            add(&proj, zd)?;
        }
    }
    Ok(set)
}

fn error_row(id: &str, element: &'static str, provider: &str, cache: &str, message: String) -> Resolution {
    Resolution {
        id: id.into(),
        element,
        provider: provider.into(),
        cache: cache.into(),
        status: Status::Error,
        sha256: None,
        message,
        notes: Vec::new(),
    }
}

fn mono(buf: &[Vec<f32>]) -> Vec<f32> {
    let n = buf.iter().map(Vec::len).max().unwrap_or(0);
    let k = buf.len().max(1) as f32;
    (0..n).map(|i| buf.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>() / k).collect()
}

type TrackSignals = (u32, std::collections::HashMap<String, Vec<Vec<f32>>>);

/// Every audio track's post-fader signal in composition time, and the rate.
fn mix(doc: &sr_model::Document) -> Result<TrackSignals, String> {
    let ev = sr_eval::Evaluator::new(doc, &sr_eval::EvalOptions::default()).map_err(|e| format!("{e:?}"))?;
    let fps = doc.scene.project.fps.as_f64();
    let a = sr_deliver::audio::mix_scene(&ev, fps, None)
        .map_err(|e| format!("mixing the scene's audio: {e}"))?
        .ok_or("the scene has no audio to transcribe")?;
    Ok((a.mix.rate, a.mixed.nodes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_stay_inside_the_root() {
        let base = Path::new("/proj/scenes");
        let inside = |src: &str, root: &str| local_in(src, base, Path::new(root));
        // as the platform spells them: a drive is added on Windows
        let at = |p: &str| lexical(Path::new(p)).unwrap();
        assert_eq!(inside("gen/vo.wav", "/proj/scenes").unwrap(), at("/proj/scenes/gen/vo.wav"));
        assert_eq!(inside("./a/../vo%20x.wav", "/proj/scenes").unwrap(), at("/proj/scenes/vo x.wav"));
        assert_eq!(inside("file:///proj/scenes/vo.wav", "/proj/scenes/").unwrap(), at("/proj/scenes/vo.wav"));
        for out in
            ["../media/vo.wav", "/etc/passwd", "file:///etc/passwd", "gen/../../x", "a/../..", ".", "../scenes-2/x"]
        {
            assert!(inside(out, "/proj/scenes").is_err_and(|e| e.contains("outside")), "{out}");
        }
        // a wider root admits a media folder beside the scenes
        assert_eq!(inside("../media/vo.wav", "/proj").unwrap(), at("/proj/media/vo.wav"));
        assert!(inside("../../etc/passwd", "/proj").is_err());
        assert!(inside("https://example.com/vo.wav", "/proj").is_err_and(|e| e.contains("remote")));
    }
}
