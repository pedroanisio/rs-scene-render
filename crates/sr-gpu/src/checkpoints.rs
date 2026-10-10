//! SREP 68, Semantics 3: the states of a document's stateful shader effects saved on disk after a frame, so that a seek
//! or a segment rendered in another process resumes from the nearest saved frame instead of replaying from frame 0.
//!
//! One file per frame holds the state of every stateful pair (effect and node) at the end of that frame. Its name is the
//! hex SHA-256 key of the frame. The key is a superset of the SREP's per-pair key: it covers
//!
//! * `"sr-checkpoint-1"`, the engine name and version, and the SHA-256 of the running executable (the build);
//! * the SHA-256 of every document's bytes (the main document and its includes), which holds the effect's XML, its node's
//!   subtree and everything that places that node;
//! * the SHA-256 of every file the documents name through an `xs:anyURI` attribute, in document order (shader sources and
//!   assets included); files above [`HASHED_BYTES`] enter by size and modification time instead of content;
//! * `project/@seed`, the frame rate and the frame.
//!
//! A change to any of them gives another key, so a checkpoint is never loaded into a document, a file set or a build it
//! was not made with. The price of the wider key is fewer hits after an edit, never a wrong picture.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sr_model::element::{AttrValue, Element};

/// Files up to this size enter the key by content; larger ones (video) by size and modification time.
pub const HASHED_BYTES: u64 = 256 << 20;

const MAGIC: &[u8; 8] = b"SRCKPT01";

/// The state of one stateful pair at the end of a frame.
#[derive(Debug, Clone, PartialEq)]
pub struct PairState {
    /// The pair's key in the renderer (effect id, node id, source hash).
    pub key: String,
    /// The frame the state was recorded at.
    pub frame: i64,
    /// Steps the pair has taken.
    pub steps: u64,
    /// Its persistent buffers: name, size and RGBA texels (exact as half floats).
    pub targets: Vec<(String, [u32; 2], Vec<[f32; 4]>)>,
}

/// A checkpoint directory for one document.
pub struct Store {
    dir: PathBuf,
    /// The key's bytes common to every frame.
    prefix: Vec<u8>,
}

impl Store {
    /// The store for `p` (whose relative files resolve against `base_dirs`) in `dir`.
    pub fn new(dir: &Path, p: &sr_eval::Program) -> std::io::Result<Store> {
        std::fs::create_dir_all(dir)?;
        let mut prefix = Vec::new();
        let mut field = |bytes: &[u8]| {
            prefix.extend_from_slice(bytes);
            prefix.push(0);
        };
        field(b"sr-checkpoint-1");
        field(b"rs-scene-render");
        field(env!("CARGO_PKG_VERSION").as_bytes());
        field(hex(&executable_digest()).as_bytes());
        for d in &p.source_digests {
            field(hex(d).as_bytes());
        }
        for f in referenced_files(p) {
            field(hex(&file_digest(&f)).as_bytes());
        }
        field(p.scene.project.seed.to_string().as_bytes());
        let fps = &p.scene.project.fps;
        field(format!("{}/{}", fps.num, fps.den).as_bytes());
        Ok(Store { dir: dir.to_path_buf(), prefix })
    }

    /// The file of frame `frame`.
    pub fn path(&self, frame: i64) -> PathBuf {
        let mut h = Sha256::new();
        h.update(&self.prefix);
        h.update(frame.to_string().as_bytes());
        self.dir.join(format!("{}.srckpt", hex(&h.finalize().into())))
    }

    /// Whether frame `frame` is saved.
    pub fn has(&self, frame: i64) -> bool {
        self.path(frame).is_file()
    }

    /// Saves the states at the end of frame `frame` (atomically: a reader sees the whole file or none).
    pub fn save(&self, frame: i64, pairs: &[PairState]) -> std::io::Result<()> {
        let path = self.path(frame);
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(pairs.len() as u64).to_le_bytes());
        for s in pairs {
            put_str(&mut out, &s.key);
            out.extend_from_slice(&s.frame.to_le_bytes());
            out.extend_from_slice(&s.steps.to_le_bytes());
            out.extend_from_slice(&(s.targets.len() as u64).to_le_bytes());
            for (name, size, px) in &s.targets {
                put_str(&mut out, name);
                out.extend_from_slice(&size[0].to_le_bytes());
                out.extend_from_slice(&size[1].to_le_bytes());
                for p in px {
                    for c in p {
                        out.extend_from_slice(&half::f16::from_f32(*c).to_bits().to_le_bytes());
                    }
                }
            }
        }
        let tmp = path.with_extension(format!("part-{}", std::process::id()));
        std::fs::File::create(&tmp)?.write_all(&out)?;
        std::fs::rename(&tmp, &path)
    }

    /// The states saved at the end of frame `frame`, when there is a whole file for it.
    pub fn load(&self, frame: i64) -> Option<Vec<PairState>> {
        let mut bytes = Vec::new();
        std::fs::File::open(self.path(frame)).ok()?.read_to_end(&mut bytes).ok()?;
        parse(&bytes)
    }
}

fn parse(bytes: &[u8]) -> Option<Vec<PairState>> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.take(8)? != MAGIC {
        return None;
    }
    let n = r.u64()?;
    let mut pairs = Vec::new();
    for _ in 0..n {
        let key = r.str()?;
        let frame = r.u64()? as i64;
        let steps = r.u64()?;
        let mut targets = Vec::new();
        for _ in 0..r.u64()? {
            let name = r.str()?;
            let (w, h) = (r.u32()?, r.u32()?);
            let texels = (w as usize).checked_mul(h as usize)?;
            let raw = r.take(texels.checked_mul(8)?)?;
            let px = raw
                .chunks_exact(8)
                .map(|t| {
                    std::array::from_fn(|c| half::f16::from_bits(u16::from_le_bytes([t[2 * c], t[2 * c + 1]])).to_f32())
                })
                .collect();
            targets.push((name, [w, h], px));
        }
        pairs.push(PairState { key, frame, steps, targets });
    }
    (r.at == bytes.len()).then_some(pairs)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u64()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u64).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of the running executable: the identity of the build (zero when it cannot be read).
fn executable_digest() -> [u8; 32] {
    static D: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::current_exe().ok().map(|p| file_digest(&p)).unwrap_or([0; 32]))
}

/// SHA-256 of a file's bytes, or of its size and modification time above [`HASHED_BYTES`], or of its path when it cannot
/// be read (a missing file).
fn file_digest(path: &Path) -> [u8; 32] {
    let mut h = Sha256::new();
    match std::fs::metadata(path) {
        Ok(m) if m.len() > HASHED_BYTES => {
            let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
            h.update(format!("size {} mtime {:?}", m.len(), mtime).as_bytes());
        }
        Ok(_) => match std::fs::File::open(path) {
            Ok(mut f) => {
                let mut buf = vec![0u8; 1 << 20];
                while let Ok(n) = f.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    h.update(&buf[..n]);
                }
            }
            Err(_) => h.update(format!("unreadable {}", path.display()).as_bytes()),
        },
        Err(_) => h.update(format!("missing {}", path.display()).as_bytes()),
    }
    h.finalize().into()
}

/// Every local file that an `xs:anyURI` attribute of the documents names, resolved against its document, in document
/// order (the main document's, then each include's).
fn referenced_files(p: &sr_eval::Program) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut scan = |e: &dyn Element, base: &Path| {
        for a in sr_model::xsd::COMPLEX_TYPES[e.xsd_type()].attrs {
            if !is_uri(a.ty) {
                continue;
            }
            if let Some(AttrValue::Str(v)) = e.get_attr(a.name) {
                if let sr_model::assets::Resolved::Local(path) = sr_model::assets::resolve(&v, base) {
                    out.push(path);
                }
            }
        }
    };
    let base = p.base_dirs.first().cloned().unwrap_or_default();
    sr_model::element::walk(&p.scene as &dyn Element, &mut |e| scan(e, &base));
    out
}

fn is_uri(ty: usize) -> bool {
    use sr_model::xsd::{Builtin, SimpleKind, SIMPLE_TYPES};
    match SIMPLE_TYPES[ty].kind {
        SimpleKind::Builtin(b) => b == Builtin::AnyUri,
        SimpleKind::Restriction(r) => is_uri(r.base),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_state_reads_back_exactly_and_a_damaged_file_does_not() {
        let px = vec![[0.0, 1.0, 65504.0, -2.5], [0.000_061_035_156, 0.5, 0.25, 1.0]];
        let pairs = vec![PairState {
            key: "f|a|7".into(),
            frame: 29,
            steps: 87,
            targets: vec![("acc".into(), [2, 1], px.clone())],
        }];
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&1u64.to_le_bytes());
        put_str(&mut out, "f|a|7");
        out.extend_from_slice(&29i64.to_le_bytes());
        out.extend_from_slice(&87u64.to_le_bytes());
        out.extend_from_slice(&1u64.to_le_bytes());
        put_str(&mut out, "acc");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for p in &px {
            for c in p {
                out.extend_from_slice(&half::f16::from_f32(*c).to_bits().to_le_bytes());
            }
        }
        assert_eq!(parse(&out), Some(pairs));
        assert_eq!(parse(&out[..out.len() - 1]), None, "a truncated file is no checkpoint");
        let mut longer = out.clone();
        longer.push(0);
        assert_eq!(parse(&longer), None, "trailing bytes are no checkpoint");
        assert_eq!(parse(b"SRCKPT02"), None);
    }
}
