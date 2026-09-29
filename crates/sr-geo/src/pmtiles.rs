//! PMTiles version 3: a single-file archive of map tiles addressed by a
//! Hilbert-curve tile id, readable without a server (Protomaps).
//!
//! [`Archive`] reads tiles from a file; [`write`] builds an archive from tiles
//! (the resolve step stores fetched tiles this way, so one digest covers them).

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

/// Tile formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileType {
    Unknown,
    Mvt,
    Png,
    Jpeg,
    Webp,
    Avif,
}

impl TileType {
    fn from_u8(v: u8) -> TileType {
        match v {
            1 => TileType::Mvt,
            2 => TileType::Png,
            3 => TileType::Jpeg,
            4 => TileType::Webp,
            5 => TileType::Avif,
            _ => TileType::Unknown,
        }
    }
    fn to_u8(self) -> u8 {
        match self {
            TileType::Unknown => 0,
            TileType::Mvt => 1,
            TileType::Png => 2,
            TileType::Jpeg => 3,
            TileType::Webp => 4,
            TileType::Avif => 5,
        }
    }
    /// Whether tiles are raster images.
    pub fn raster(self) -> bool {
        matches!(self, TileType::Png | TileType::Jpeg | TileType::Webp | TileType::Avif)
    }
}

/// The archive header.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub root_offset: u64,
    pub root_length: u64,
    pub metadata_offset: u64,
    pub metadata_length: u64,
    pub leaf_offset: u64,
    pub leaf_length: u64,
    pub data_offset: u64,
    pub data_length: u64,
    pub addressed_tiles: u64,
    pub tile_entries: u64,
    pub tile_contents: u64,
    pub clustered: bool,
    /// 1 none, 2 gzip, 3 brotli, 4 zstd.
    pub internal_compression: u8,
    pub tile_compression: u8,
    pub tile_type: TileType,
    pub min_zoom: u8,
    pub max_zoom: u8,
    /// West, south, east, north (degrees).
    pub bounds: [f64; 4],
    pub center_zoom: u8,
    pub center: [f64; 2],
}

const HEADER_LEN: usize = 127;

fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

impl Header {
    /// Parses the 127-byte header.
    pub fn parse(b: &[u8]) -> Result<Header, String> {
        if b.len() < HEADER_LEN || &b[0..7] != b"PMTiles" {
            return Err("not a PMTiles archive".into());
        }
        if b[7] != 3 {
            return Err(format!("PMTiles version {} (only version 3 is read)", b[7]));
        }
        let e7 = |o| i32_at(b, o) as f64 / 1e7;
        Ok(Header {
            root_offset: u64_at(b, 8),
            root_length: u64_at(b, 16),
            metadata_offset: u64_at(b, 24),
            metadata_length: u64_at(b, 32),
            leaf_offset: u64_at(b, 40),
            leaf_length: u64_at(b, 48),
            data_offset: u64_at(b, 56),
            data_length: u64_at(b, 64),
            addressed_tiles: u64_at(b, 72),
            tile_entries: u64_at(b, 80),
            tile_contents: u64_at(b, 88),
            clustered: b[96] == 1,
            internal_compression: b[97],
            tile_compression: b[98],
            tile_type: TileType::from_u8(b[99]),
            min_zoom: b[100],
            max_zoom: b[101],
            bounds: [e7(102), e7(106), e7(110), e7(114)],
            center_zoom: b[118],
            center: [e7(119), e7(123)],
        })
    }

    fn bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(HEADER_LEN);
        b.extend(b"PMTiles");
        b.push(3);
        for v in [
            self.root_offset,
            self.root_length,
            self.metadata_offset,
            self.metadata_length,
            self.leaf_offset,
            self.leaf_length,
            self.data_offset,
            self.data_length,
            self.addressed_tiles,
            self.tile_entries,
            self.tile_contents,
        ] {
            b.extend(v.to_le_bytes());
        }
        b.extend([self.clustered as u8, self.internal_compression, self.tile_compression, self.tile_type.to_u8()]);
        b.extend([self.min_zoom, self.max_zoom]);
        for v in self.bounds {
            b.extend(((v * 1e7).round() as i32).to_le_bytes());
        }
        b.push(self.center_zoom);
        for v in self.center {
            b.extend(((v * 1e7).round() as i32).to_le_bytes());
        }
        b
    }
}

/// The tile id of z/x/y: the tiles of lower zooms, then the position on zoom z's Hilbert curve.
pub fn tile_id(z: u8, x: u32, y: u32) -> u64 {
    let acc = ((1u64 << (2 * z as u32)) - 1) / 3;
    let n = 1u64 << z;
    let (mut x, mut y) = (x as u64, y as u64);
    let mut d = 0u64;
    let mut s = n / 2;
    while s > 0 {
        let rx = u64::from(x & s > 0);
        let ry = u64::from(y & s > 0);
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    acc + d
}

/// z/x/y of a tile id.
pub fn tile_zxy(id: u64) -> (u8, u32, u32) {
    let mut acc = 0u64;
    for z in 0..32u8 {
        let count = 1u64 << (2 * z as u32);
        if id < acc + count {
            let pos = id - acc;
            let n = 1u64 << z;
            let (mut x, mut y) = (0u64, 0u64);
            let mut t = pos;
            let mut s = 1u64;
            while s < n {
                let rx = 1 & (t / 2);
                let ry = 1 & (t ^ rx);
                if ry == 0 {
                    if rx == 1 {
                        x = s - 1 - x;
                        y = s - 1 - y;
                    }
                    std::mem::swap(&mut x, &mut y);
                }
                x += s * rx;
                y += s * ry;
                t /= 4;
                s *= 2;
            }
            return (z, x as u32, y as u32);
        }
        acc += count;
    }
    (0, 0, 0)
}

/// A directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub tile_id: u64,
    pub offset: u64,
    pub length: u32,
    /// 0: the entry points at a leaf directory.
    pub run_length: u32,
}

fn read_varint(b: &[u8], i: &mut usize) -> Result<u64, String> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let byte = *b.get(*i).ok_or("truncated varint")?;
        *i += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(v);
        }
        shift += 7;
        if shift > 63 {
            return Err("varint too long".into());
        }
    }
}

fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Decodes a (decompressed) directory.
pub fn parse_directory(b: &[u8]) -> Result<Vec<Entry>, String> {
    let mut i = 0;
    let n = read_varint(b, &mut i)? as usize;
    let mut entries = vec![Entry { tile_id: 0, offset: 0, length: 0, run_length: 0 }; n];
    let mut last = 0u64;
    for e in entries.iter_mut() {
        last += read_varint(b, &mut i)?;
        e.tile_id = last;
    }
    for e in entries.iter_mut() {
        e.run_length = read_varint(b, &mut i)? as u32;
    }
    for e in entries.iter_mut() {
        e.length = read_varint(b, &mut i)? as u32;
    }
    for k in 0..n {
        let v = read_varint(b, &mut i)?;
        entries[k].offset = if v == 0 && k > 0 { entries[k - 1].offset + entries[k - 1].length as u64 } else { v - 1 };
    }
    Ok(entries)
}

/// Encodes a directory (uncompressed).
pub fn serialize_directory(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint(&mut out, entries.len() as u64);
    let mut last = 0;
    for e in entries {
        write_varint(&mut out, e.tile_id - last);
        last = e.tile_id;
    }
    for e in entries {
        write_varint(&mut out, e.run_length as u64);
    }
    for e in entries {
        write_varint(&mut out, e.length as u64);
    }
    for (k, e) in entries.iter().enumerate() {
        let contiguous = k > 0 && e.offset == entries[k - 1].offset + entries[k - 1].length as u64;
        write_varint(&mut out, if contiguous { 0 } else { e.offset + 1 });
    }
    out
}

/// Decompresses by a PMTiles compression code.
pub fn decompress(code: u8, b: Vec<u8>) -> Result<Vec<u8>, String> {
    match code {
        0 | 1 => Ok(b),
        2 => {
            // tiles are sometimes stored uncompressed despite the header; gzip starts 1f 8b
            if b.len() < 2 || b[0] != 0x1f || b[1] != 0x8b {
                return Ok(b);
            }
            let mut out = Vec::new();
            flate2::read::MultiGzDecoder::new(&b[..]).read_to_end(&mut out).map_err(|e| format!("gzip: {e}"))?;
            Ok(out)
        }
        3 => Err("brotli-compressed PMTiles are not supported".into()),
        4 => Err("zstd-compressed PMTiles are not supported".into()),
        c => Err(format!("unknown PMTiles compression {c}")),
    }
}

/// An open archive.
pub struct Archive {
    pub header: Header,
    pub metadata: serde_json::Value,
    file: Mutex<File>,
    root: Vec<Entry>,
    leaves: Mutex<HashMap<u64, std::sync::Arc<Vec<Entry>>>>,
}

impl Archive {
    /// Opens an archive file.
    pub fn open(path: &Path) -> Result<Archive, String> {
        let mut f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut head = vec![0u8; HEADER_LEN];
        f.read_exact(&mut head).map_err(|e| format!("{}: {e}", path.display()))?;
        let header = Header::parse(&head).map_err(|e| format!("{}: {e}", path.display()))?;
        let read = |f: &mut File, off: u64, len: u64| -> Result<Vec<u8>, String> {
            let mut b = vec![0u8; len as usize];
            f.seek(SeekFrom::Start(off))
                .and_then(|_| f.read_exact(&mut b))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(b)
        };
        let root = parse_directory(&decompress(
            header.internal_compression,
            read(&mut f, header.root_offset, header.root_length)?,
        )?)?;
        let metadata = if header.metadata_length > 0 {
            let m =
                decompress(header.internal_compression, read(&mut f, header.metadata_offset, header.metadata_length)?)?;
            serde_json::from_slice(&m).unwrap_or(serde_json::Value::Null)
        } else {
            serde_json::Value::Null
        };
        Ok(Archive { header, metadata, file: Mutex::new(f), root, leaves: Mutex::new(HashMap::new()) })
    }

    fn read(&self, off: u64, len: u64) -> Result<Vec<u8>, String> {
        let mut f = self.file.lock().unwrap_or_else(|p| p.into_inner());
        let mut b = vec![0u8; len as usize];
        f.seek(SeekFrom::Start(off)).and_then(|_| f.read_exact(&mut b)).map_err(|e| e.to_string())?;
        Ok(b)
    }

    fn find(entries: &[Entry], id: u64) -> Option<Entry> {
        // the last entry whose id is not after `id`
        let k = entries.partition_point(|e| e.tile_id <= id);
        let e = *entries.get(k.checked_sub(1)?)?;
        if e.run_length == 0 || id < e.tile_id + e.run_length as u64 {
            Some(e)
        } else {
            None
        }
    }

    /// The (decompressed) bytes of tile z/x/y, or `None` when the archive does not have it.
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, String> {
        let id = tile_id(z, x, y);
        let mut dir: std::sync::Arc<Vec<Entry>> = std::sync::Arc::new(self.root.clone());
        for _ in 0..4 {
            let Some(e) = Self::find(&dir, id) else { return Ok(None) };
            if e.run_length > 0 {
                let b = self.read(self.header.data_offset + e.offset, e.length as u64)?;
                return decompress(self.header.tile_compression, b).map(Some);
            }
            let off = self.header.leaf_offset + e.offset;
            let cached = self.leaves.lock().unwrap_or_else(|p| p.into_inner()).get(&off).cloned();
            dir = match cached {
                Some(d) => d,
                None => {
                    let raw = self.read(off, e.length as u64)?;
                    let d = std::sync::Arc::new(parse_directory(&decompress(self.header.internal_compression, raw)?)?);
                    self.leaves.lock().unwrap_or_else(|p| p.into_inner()).insert(off, d.clone());
                    d
                }
            };
        }
        Err("PMTiles directories nest too deeply".into())
    }
}

/// A tile's address (z, x, y) and bytes.
pub type TileBytes = ((u8, u32, u32), Vec<u8>);

/// Builds an archive from tiles (z, x, y, bytes stored as given, compressed per `tile_compression`).
/// Identical tiles are stored once; directories are uncompressed, with leaf directories when the
/// root would not fit in the first 16 KiB.
pub fn write(tiles: &[TileBytes], tile_type: TileType, tile_compression: u8, metadata: &serde_json::Value) -> Vec<u8> {
    let mut sorted: Vec<(u64, &Vec<u8>)> = tiles.iter().map(|((z, x, y), b)| (tile_id(*z, *x, *y), b)).collect();
    sorted.sort_by_key(|t| t.0);
    sorted.dedup_by_key(|t| t.0);
    let mut data: Vec<u8> = Vec::new();
    let mut seen: HashMap<&[u8], (u64, u32)> = HashMap::new();
    let mut entries: Vec<Entry> = Vec::new();
    for (id, b) in &sorted {
        let (off, len) = *seen.entry(b.as_slice()).or_insert_with(|| {
            let off = data.len() as u64;
            data.extend_from_slice(b);
            (off, b.len() as u32)
        });
        match entries.last_mut() {
            Some(l) if l.offset == off && l.tile_id + l.run_length as u64 == *id => l.run_length += 1,
            _ => entries.push(Entry { tile_id: *id, offset: off, length: len, run_length: 1 }),
        }
    }
    let meta = serde_json::to_vec(metadata).expect("json");
    // root directory, or a root of leaf pointers
    let mut root = serialize_directory(&entries);
    let mut leaves = Vec::new();
    if root.len() > 16384 - HEADER_LEN {
        let mut per = 4096;
        loop {
            leaves.clear();
            let mut pointers = Vec::new();
            for chunk in entries.chunks(per) {
                let d = serialize_directory(chunk);
                pointers.push(Entry {
                    tile_id: chunk[0].tile_id,
                    offset: leaves.len() as u64,
                    length: d.len() as u32,
                    run_length: 0,
                });
                leaves.extend(d);
            }
            root = serialize_directory(&pointers);
            if root.len() <= 16384 - HEADER_LEN {
                break;
            }
            per *= 2;
        }
    }
    let zooms: Vec<u8> = tiles.iter().map(|t| t.0 .0).collect();
    let (minz, maxz) = (zooms.iter().copied().min().unwrap_or(0), zooms.iter().copied().max().unwrap_or(0));
    let mut h = Header {
        root_offset: HEADER_LEN as u64,
        root_length: root.len() as u64,
        metadata_offset: 0,
        metadata_length: meta.len() as u64,
        leaf_offset: 0,
        leaf_length: leaves.len() as u64,
        data_offset: 0,
        data_length: data.len() as u64,
        addressed_tiles: sorted.len() as u64,
        tile_entries: entries.len() as u64,
        tile_contents: seen.len() as u64,
        clustered: true,
        internal_compression: 1,
        tile_compression,
        tile_type,
        min_zoom: minz,
        max_zoom: maxz,
        bounds: [-180.0, -85.0511287, 180.0, 85.0511287],
        center_zoom: minz,
        center: [0.0, 0.0],
    };
    h.metadata_offset = h.root_offset + h.root_length;
    h.leaf_offset = h.metadata_offset + h.metadata_length;
    h.data_offset = h.leaf_offset + h.leaf_length;
    let mut out = h.bytes();
    out.extend(root);
    out.extend(meta);
    out.extend(leaves);
    out.extend(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_ids_follow_the_hilbert_curve_and_invert() {
        // values from the PMTiles specification's test vectors
        assert_eq!(tile_id(0, 0, 0), 0);
        assert_eq!(tile_id(1, 0, 0), 1);
        assert_eq!(tile_id(1, 0, 1), 2);
        assert_eq!(tile_id(1, 1, 1), 3);
        assert_eq!(tile_id(1, 1, 0), 4);
        assert_eq!(tile_id(2, 0, 0), 5);
        for z in 0..9u8 {
            for (x, y) in [(0, 0), ((1u32 << z) - 1, 0), (3 % (1 << z), 5 % (1 << z))] {
                assert_eq!(tile_zxy(tile_id(z, x, y)), (z, x, y));
            }
        }
    }

    #[test]
    fn archives_round_trip_with_leaf_directories() {
        let mut tiles = Vec::new();
        for x in 0..128u32 {
            for y in 0..128u32 {
                // repeated contents make runs; distinct ones need many entries
                let b = if (x + y) % 3 == 0 { b"same".to_vec() } else { format!("{x}/{y}").into_bytes() };
                tiles.push(((7u8, x, y), b));
            }
        }
        let bytes = write(&tiles, TileType::Png, 1, &serde_json::json!({"name": "t"}));
        let dir = std::env::temp_dir().join(format!("pmtiles-rt-{}.pmtiles", std::process::id()));
        std::fs::write(&dir, &bytes).unwrap();
        let a = Archive::open(&dir).unwrap();
        assert!(a.header.leaf_length > 0, "needs leaves");
        assert_eq!(a.metadata["name"], "t");
        for ((z, x, y), b) in tiles.iter().step_by(97) {
            assert_eq!(a.tile(*z, *x, *y).unwrap().as_ref(), Some(b));
        }
        assert_eq!(a.tile(6, 0, 0).unwrap(), None);
        std::fs::remove_file(dir).ok();
    }
}
