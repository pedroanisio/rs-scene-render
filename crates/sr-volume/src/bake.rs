//! Immutable, content-addressed render bakes. SRVSEQ stores composition-time
//! samples, not resumable solver state. A manifest is published only after every
//! frame is complete. SHA-256 names bind frame bytes without accepting file paths
//! from the manifest. The caller can additionally pin the manifest's digest.

use crate::{
    sequence::{FramePair, Interpolation, MissingFrame, Sequence, TimedFramePair},
    CacheLimits, Error, Volume,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAGIC: &[u8; 8] = b"SRVSEQ\r\n";
const VERSION: u32 = 1;
const MAX_FRAMES: u32 = 100_000;
const HEADER: u64 = 32;
const ENTRY: u64 = 40;

#[derive(Debug, Clone, Copy)]
pub struct BakeLimits {
    pub max_frames: u32,
    /// Unique frame files plus the manifest, rather than repeated sample references.
    pub max_total_bytes: u64,
    pub frame: CacheLimits,
}
impl Default for BakeLimits {
    fn default() -> Self {
        Self { max_frames: MAX_FRAMES, max_total_bytes: 64 << 30, frame: CacheLimits::default() }
    }
}

#[derive(Debug, Clone)]
pub struct BakeReceipt {
    pub manifest: PathBuf,
    pub sha256: String,
    pub frames: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct BakedFrame {
    digest: [u8; 32],
    bytes: u64,
}
impl BakedFrame {
    pub fn filename(&self) -> String {
        format!("{}.srvol", hex(&self.digest))
    }
    pub fn sha256(&self) -> String {
        hex(&self.digest)
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    /// Validates size, bounded SRVOL structure, and the digest of the bytes actually decoded.
    pub fn read(&self, directory: &Path, limits: CacheLimits) -> Result<Arc<Volume>, Error> {
        let file = File::open(directory.join(self.filename()))?;
        if file.metadata()?.len() != self.bytes {
            return Err(Error::Invalid("baked frame size mismatch"));
        }
        let mut reader = HashReader { input: BufReader::new(file), hash: Sha256::new() };
        let volume = Volume::read(&mut reader, limits)?;
        let digest: [u8; 32] = reader.hash.finalize().into();
        if digest != self.digest {
            return Err(Error::Invalid("baked frame SHA-256 mismatch"));
        }
        Ok(Arc::new(volume))
    }
}

#[derive(Debug)]
pub struct BakedSequence {
    directory: PathBuf,
    start: f64,
    fps: f64,
    entries: Vec<Option<BakedFrame>>,
    limits: BakeLimits,
}
impl BakedSequence {
    pub fn open(path: &Path, expected_sha: Option<&str>, limits: BakeLimits) -> Result<Self, Error> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();
        if size > HEADER + ENTRY * u64::from(limits.max_frames.min(MAX_FRAMES)) {
            return Err(Error::Limit("bake manifest bytes"));
        }
        let mut r = HashReader { input: BufReader::new(file), hash: Sha256::new() };
        if &read::<8>(&mut r)? != MAGIC || u32::from_le_bytes(read(&mut r)?) != VERSION {
            return Err(Error::Invalid("bake magic or version"));
        }
        let start = f64::from_le_bytes(read(&mut r)?);
        let fps = f64::from_le_bytes(read(&mut r)?);
        let count = u32::from_le_bytes(read(&mut r)?);
        validate_time(start, fps, count, limits)?;
        if size != HEADER + ENTRY * u64::from(count) {
            return Err(Error::Invalid("bake manifest length"));
        }
        let mut entries = Vec::with_capacity(count as usize);
        let mut unique = HashMap::new();
        let mut total = size;
        for _ in 0..count {
            let digest = read(&mut r)?;
            let bytes = u64::from_le_bytes(read(&mut r)?);
            if bytes == 0 {
                if digest != [0; 32] {
                    return Err(Error::Invalid("empty bake frame digest"));
                }
                entries.push(None);
            } else {
                if bytes > limits.frame.max_bytes {
                    return Err(Error::Limit("baked frame bytes"));
                }
                match unique.insert(digest, bytes) {
                    Some(previous) if previous != bytes => {
                        return Err(Error::Invalid("repeated baked frame has conflicting lengths"))
                    }
                    None => total = total.checked_add(bytes).ok_or(Error::Limit("bake size overflow"))?,
                    _ => {}
                }
                entries.push(Some(BakedFrame { digest, bytes }));
            }
        }
        if total > limits.max_total_bytes {
            return Err(Error::Limit("bake total bytes"));
        }
        eof(&mut r)?;
        if expected_sha.is_some_and(|s| !s.eq_ignore_ascii_case(&hex(&r.hash.finalize().into()))) {
            return Err(Error::Invalid("bake manifest SHA-256 mismatch"));
        }
        Ok(Self { directory: path.parent().unwrap_or(Path::new("")).to_owned(), start, fps, entries, limits })
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn frame_count(&self) -> usize {
        self.entries.len()
    }
    pub fn start(&self) -> f64 {
        self.start
    }
    pub fn fps(&self) -> f64 {
        self.fps
    }
    pub fn manifest_bytes(&self) -> u64 {
        HEADER + ENTRY * self.entries.len() as u64
    }
    pub fn frames(&self) -> impl Iterator<Item = &BakedFrame> {
        self.entries.iter().flatten()
    }
    pub fn load(&self, time: f64, interpolation: Interpolation, max_bytes: usize) -> Result<FramePair, Error> {
        self.load_with(time, interpolation, max_bytes, |frame| frame.read(&self.directory, self.limits.frame))
    }
    /// The loader may cache immutable frames by their digest. It must verify the
    /// bytes at least once, using [`BakedFrame::read`], before admitting a frame.
    pub fn load_with(
        &self,
        time: f64,
        interpolation: Interpolation,
        max_bytes: usize,
        loader: impl FnMut(&BakedFrame) -> Result<Arc<Volume>, Error>,
    ) -> Result<FramePair, Error> {
        self.load_inner(time, interpolation, max_bytes, loader, false).map(|t| t.frames)
    }

    /// Loads motion-aware endpoint times on the bake's composition clock.
    pub fn load_timed(
        &self,
        time: f64,
        interpolation: Interpolation,
        max_bytes: usize,
    ) -> Result<TimedFramePair, Error> {
        self.load_timed_with(time, interpolation, max_bytes, |frame| frame.read(&self.directory, self.limits.frame))
    }

    /// Cached variant of `load_timed`; the integrity requirements of `load_with` apply.
    pub fn load_timed_with(
        &self,
        time: f64,
        interpolation: Interpolation,
        max_bytes: usize,
        loader: impl FnMut(&BakedFrame) -> Result<Arc<Volume>, Error>,
    ) -> Result<TimedFramePair, Error> {
        self.load_inner(time, interpolation, max_bytes, loader, true)
    }

    fn load_inner(
        &self,
        time: f64,
        interpolation: Interpolation,
        max_bytes: usize,
        mut loader: impl FnMut(&BakedFrame) -> Result<Arc<Volume>, Error>,
        timed: bool,
    ) -> Result<TimedFramePair, Error> {
        let offset = (time - self.start) * self.fps;
        // Subtracting an absolute start can lose more precision than multiplying
        // a small local time. Recover intended integral samples within a bounded
        // floating-point error, without blending their neighboring fields.
        let tolerance = (8. * f64::EPSILON * (time.abs() + self.start.abs() + 1.) * self.fps).min(1e-6);
        let snapped = (offset - offset.round()).abs() <= tolerance;
        let offset = if snapped { offset.round() } else { offset };
        if !time.is_finite() || !offset.is_finite() || offset < 0. || offset >= self.entries.len() as f64 {
            return Err(Error::Invalid("time outside baked composition range"));
        }
        if snapped {
            let volume = self.entries[offset as usize].as_ref().map(&mut loader).transpose()?;
            if volume.as_ref().is_some_and(|v| v.bytes() > max_bytes) {
                return Err(Error::Limit("volume sequence frame bytes"));
            }
            return Ok(TimedFramePair {
                frames: FramePair { first: volume.clone(), second: volume, blend: 0. },
                elapsed: [0.; 2],
            });
        }
        let sequence =
            Sequence::new(0, self.entries.len() as i64 - 1, self.fps, interpolation, MissingFrame::Transparent)?;
        let loader = |i: i64| self.entries[i as usize].as_ref().map(&mut loader).transpose();
        if timed {
            sequence.load_timed(offset / self.fps, max_bytes, loader)
        } else {
            sequence
                .load(offset / self.fps, max_bytes, loader)
                .map(|frames| TimedFramePair { frames, elapsed: [0.; 2] })
        }
    }
}

/// Exclusive, bounded transaction. An append error poisons the transaction;
/// dropping it removes its newly created directory. Existing paths are never replaced.
pub struct BakeWriter {
    directory: PathBuf,
    start: f64,
    fps: f64,
    limits: BakeLimits,
    entries: Vec<Option<BakedFrame>>,
    unique: HashSet<[u8; 32]>,
    bytes: u64,
    failed: bool,
    committed: bool,
}
impl BakeWriter {
    pub fn new(directory: &Path, start: f64, fps: f64, limits: BakeLimits) -> Result<Self, Error> {
        validate_time(start, fps, 1, limits)?;
        std::fs::create_dir(directory)?;
        Ok(Self {
            directory: directory.to_owned(),
            start,
            fps,
            limits,
            entries: Vec::new(),
            unique: HashSet::new(),
            bytes: 0,
            failed: false,
            committed: false,
        })
    }
    pub fn push(&mut self, volume: Option<&Volume>) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Invalid("bake transaction already failed"));
        }
        let result = self.append(volume);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn append(&mut self, volume: Option<&Volume>) -> Result<(), Error> {
        let count = self.entries.len() + 1;
        validate_time(self.start, self.fps, count as u32, self.limits)?;
        let manifest = HEADER + ENTRY * count as u64;
        let entry = if let Some(volume) = volume {
            let grids = volume.grids().count();
            let bricks: usize = volume.grids().map(|(_, g)| g.bricks().count()).sum();
            if grids > self.limits.frame.max_grids
                || bricks > self.limits.frame.max_bricks
                || volume.bytes() as u64 > self.limits.frame.max_bytes
            {
                return Err(Error::Limit("baked frame resident budget"));
            }
            let mut sink = HashSink { hash: Sha256::new(), bytes: 0, limit: self.limits.frame.max_bytes };
            volume.write(&mut sink)?;
            let digest = sink.hash.finalize().into();
            let bytes = sink.bytes;
            if !self.unique.contains(&digest) {
                let total = self
                    .bytes
                    .checked_add(bytes)
                    .and_then(|v| v.checked_add(manifest))
                    .ok_or(Error::Limit("bake size overflow"))?;
                if total > self.limits.max_total_bytes {
                    return Err(Error::Limit("bake total bytes"));
                }
                let path = self.directory.join(format!("{}.srvol", hex(&digest)));
                let file = File::create_new(path)?;
                let mut out = BufWriter::new(file);
                volume.write(&mut out)?;
                out.flush()?;
                out.get_ref().sync_all()?;
                self.bytes += bytes;
                self.unique.insert(digest);
            }
            Some(BakedFrame { digest, bytes })
        } else {
            None
        };
        if self.bytes.checked_add(manifest).is_none_or(|n| n > self.limits.max_total_bytes) {
            return Err(Error::Limit("bake total bytes"));
        }
        self.entries.push(entry);
        Ok(())
    }
    pub fn finish(mut self) -> Result<BakeReceipt, Error> {
        if self.failed || self.entries.is_empty() {
            return Err(Error::Invalid("empty or failed bake transaction"));
        }
        let mut data = Vec::with_capacity((HEADER + ENTRY * self.entries.len() as u64) as usize);
        data.extend_from_slice(MAGIC);
        data.extend_from_slice(&VERSION.to_le_bytes());
        data.extend_from_slice(&self.start.to_le_bytes());
        data.extend_from_slice(&self.fps.to_le_bytes());
        data.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for entry in &self.entries {
            match entry {
                Some(f) => {
                    data.extend_from_slice(&f.digest);
                    data.extend_from_slice(&f.bytes.to_le_bytes());
                }
                None => data.extend_from_slice(&[0; ENTRY as usize]),
            }
        }
        let temporary = self.directory.join("manifest.tmp");
        let mut file = File::create_new(&temporary)?;
        file.write_all(&data)?;
        file.sync_all()?;
        drop(file);
        let manifest = self.directory.join("manifest.srvseq");
        std::fs::rename(temporary, &manifest)?;
        self.committed = true;
        Ok(BakeReceipt {
            manifest,
            sha256: hex(&Sha256::digest(&data).into()),
            frames: self.entries.len(),
            bytes: self.bytes + data.len() as u64,
        })
    }
}
impl Drop for BakeWriter {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}

fn validate_time(start: f64, fps: f64, count: u32, limits: BakeLimits) -> Result<(), Error> {
    if count == 0 || count > limits.max_frames.min(MAX_FRAMES) {
        return Err(Error::Limit("bake requires 1..100000 frames"));
    }
    if !start.is_finite()
        || start < 0.
        || !fps.is_finite()
        || fps <= 0.
        || !(start + f64::from(count) / fps).is_finite()
        || start + 1. / fps <= start
    {
        return Err(Error::Invalid("bake time range must be finite and representable"));
    }
    Ok(())
}
fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}
fn read<const N: usize>(r: &mut impl Read) -> Result<[u8; N], Error> {
    let mut b = [0; N];
    r.read_exact(&mut b)?;
    Ok(b)
}
fn eof(r: &mut impl Read) -> Result<(), Error> {
    match r.read_exact(&mut [0]) {
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(()),
        Err(e) => Err(e.into()),
        Ok(()) => Err(Error::Invalid("trailing bake data")),
    }
}
struct HashReader<R> {
    input: R,
    hash: Sha256,
}
impl<R: Read> Read for HashReader<R> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.input.read(b)?;
        self.hash.update(&b[..n]);
        Ok(n)
    }
}
struct HashSink {
    hash: Sha256,
    bytes: u64,
    limit: u64,
}
impl Write for HashSink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let next = self
            .bytes
            .checked_add(b.len() as u64)
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| std::io::Error::other("baked frame byte limit"))?;
        self.hash.update(b);
        self.bytes = next;
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
