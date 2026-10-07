//! The `.vox` file of MagicaVoxel: a header, then a `MAIN` chunk whose children are the models (`SIZE` and `XYZI`), the
//! palette (`RGBA`), the materials (`MATL`) and the scene graph that places the models (`nTRN`, `nGRP`, `nSHP`); every other chunk
//! is skipped.
//!
//! What this reader takes from the format, so that what it does not is plain:
//!
//! * A chunk is an id of four bytes, the bytes of its content, the bytes of its children, all little endian; the header is `VOX `
//!   and the version, 150 or 200.
//! * A model is a `SIZE` (x, y, z) and the `XYZI` that follows it: a count and that many cells of four bytes, x, y, z and a colour
//!   index 1 to 255, each cell inside the size and no cell twice.
//! * The `RGBA` chunk is 256 colours; the colour of index `c` is the entry `c - 1` (the description of the format: "color [0-254] are
//!   mapped to palette index [1-255]"). A file with no RGBA chunk has the default palette of the description (`default_palette`).
//! * A `MATL` chunk is a palette index and a dictionary of strings, kept as they are spelt.
//! * The scene graph: `nTRN` (a transform of one child: frame 0 only, `_r` the rotation byte and `_t` the translation in cells),
//!   `nGRP` (children), `nSHP` (the models). The cells of a model are placed about its centre, `size / 2` with integer division,
//!   by `p[k] = sign[k] * q[column[k]] + t[k]`. The rotation byte holds the column of the nonzero entry of the first row in bits 0
//!   and 1, that of the second row in bits 2 and 3 (the third is the column that is left) and the three signs in bits 4 to 6.
//!   Where cells of two models fall on one place the later one in the graph wins. Layers, hidden nodes, animation (frames after the
//!   first) and cameras are not read.
//!
//! Where each of these comes from: the palette offset and the default palette, from section 7 and 8 of the description of the file
//! format by the author of MagicaVoxel (<https://github.com/ephtracy/voxel-model>, `MagicaVoxel-file-format-vox.txt`); the rotation byte,
//! from section (c) of its extension file (the example `R = [[0, 1, 0], [0, 0, -1], [-1, 0, 0]]` is the byte 105) and applied as
//! `p = R q`, the centre of a model at `floor(size / 2)` and a `MATL` id as the palette index, from `ogt_vox.h` of
//! opengametools (<https://github.com/jpaver/opengametools>, MIT: "the centre pivot for that model is located at floor(size.xyz / 2)"; its
//! `materials.matl[color_index]` beside `palette.color[color_index]`). The fixtures written by `tools/make_vox.py` follow the same
//! reading; the files of MagicaVoxel's author and of the `dot_vox` crate that are next to them are real.

use super::default_palette::DEFAULT_PALETTE;
use super::{scene_cell, Colours, Imported};
use crate::occupancy::{Limits, Occupancy};
use std::collections::{BTreeMap, BTreeSet};

/// Bounds of what the reader accepts before it allocates, besides the limits of the grid.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    /// Bytes of the file.
    pub max_file_bytes: usize,
    /// Models in the file.
    pub max_models: usize,
    /// Nodes of the scene graph.
    pub max_nodes: usize,
    /// Depth of the scene graph.
    pub max_depth: usize,
}

impl Default for Bounds {
    fn default() -> Self {
        Self { max_file_bytes: 1 << 30, max_models: 65_536, max_nodes: 1 << 20, max_depth: 64 }
    }
}

/// One model: its size and its cells `[x, y, z, colour index]`.
#[derive(Clone, Debug)]
pub struct Model {
    pub size: [u32; 3],
    pub voxels: Vec<[u8; 4]>,
}

/// A rotation: row `k` of the matrix has the entry `sign[k]` in column `column[k]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rotation {
    column: [u8; 3],
    sign: [i8; 3],
}

impl Rotation {
    const IDENTITY: Rotation = Rotation { column: [0, 1, 2], sign: [1; 3] };

    fn from_byte(byte: u8) -> Result<Rotation, String> {
        let (c0, c1) = (byte & 3, (byte >> 2) & 3);
        if c0 > 2 || c1 > 2 || c0 == c1 {
            return Err(format!("the rotation byte {byte} is not a rotation"));
        }
        let c2 = 3 - c0 - c1;
        let s = |bit: u8| if byte >> bit & 1 == 1 { -1 } else { 1 };
        Ok(Rotation { column: [c0, c1, c2], sign: [s(4), s(5), s(6)] })
    }

    fn apply(&self, q: [i64; 3]) -> [i64; 3] {
        std::array::from_fn(|k| i64::from(self.sign[k]) * q[usize::from(self.column[k])])
    }

    /// `self` after `inner`: the rotation that does `inner` and then `self`.
    fn after(&self, inner: &Rotation) -> Rotation {
        // (self * inner) q: row k of self picks inner's row column[k]
        let mut column = [0u8; 3];
        let mut sign = [1i8; 3];
        for k in 0..3 {
            let row = usize::from(self.column[k]);
            column[k] = inner.column[row];
            sign[k] = self.sign[k] * inner.sign[row];
        }
        Rotation { column, sign }
    }
}

#[derive(Clone, Copy, Debug)]
struct Placement {
    rotation: Rotation,
    translation: [i64; 3],
}

impl Placement {
    const IDENTITY: Placement = Placement { rotation: Rotation::IDENTITY, translation: [0; 3] };

    /// `self` applied after `inner`.
    fn after(&self, inner: &Placement) -> Placement {
        let moved = self.rotation.apply(inner.translation);
        Placement {
            rotation: self.rotation.after(&inner.rotation),
            translation: std::array::from_fn(|k| moved[k] + self.translation[k]),
        }
    }
}

#[derive(Clone, Debug)]
enum Node {
    Transform { child: i32, placement: Placement },
    Group { children: Vec<i32> },
    Shape { models: Vec<usize> },
}

/// A file read: the models, the colours, the materials and the scene graph if it has one.
#[derive(Debug)]
pub struct Vox {
    pub version: i32,
    pub models: Vec<Model>,
    /// The 256 colours of the `RGBA` chunk, as stored.
    pub palette: Option<Box<[[u8; 4]; 256]>>,
    /// The `MATL` chunks: palette index and the dictionary.
    pub materials: BTreeMap<u32, BTreeMap<String, String>>,
    nodes: BTreeMap<i32, Node>,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn left(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn take(&mut self, n: usize, what: &str) -> Result<&'a [u8], String> {
        if n > self.left() {
            return Err(format!("{what}: needs {n} bytes and the file has {} left", self.left()));
        }
        let out = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(out)
    }

    fn i32(&mut self, what: &str) -> Result<i32, String> {
        let b = self.take(4, what)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn count(&mut self, what: &str) -> Result<usize, String> {
        let v = self.i32(what)?;
        usize::try_from(v).map_err(|_| format!("{what}: {v} is negative"))
    }

    fn string(&mut self, what: &str) -> Result<String, String> {
        let n = self.count(what)?;
        let b = self.take(n, what)?;
        Ok(String::from_utf8_lossy(b).into_owned())
    }

    fn dictionary(&mut self, what: &str) -> Result<BTreeMap<String, String>, String> {
        let n = self.count(what)?;
        // every pair is at least 8 bytes, so a count that the bytes cannot hold is refused before it is looped over
        if n > self.left() / 8 {
            return Err(format!("{what}: a dictionary of {n} entries in {} bytes", self.left()));
        }
        let mut out = BTreeMap::new();
        for _ in 0..n {
            let key = self.string(what)?;
            let value = self.string(what)?;
            out.insert(key, value);
        }
        Ok(out)
    }
}

/// How far from the origin, in cells, a translation or a placed cell may be: far inside the keys of an occupancy, and small enough that
/// the translations of a scene graph at its deepest add up in 64 bits.
const REACH: i64 = 1 << 24;

/// The largest side of a model that is accepted: a size beyond a few thousand is not a model.
const MAX_SIDE: u32 = 4096;

/// Reads the file: the models, the palette, the materials and the scene graph, validated.
pub fn parse(bytes: &[u8], bounds: &Bounds) -> Result<Vox, String> {
    if bytes.len() > bounds.max_file_bytes {
        return Err(format!("the file has {} bytes and the limit is {} bytes", bytes.len(), bounds.max_file_bytes));
    }
    let mut c = Cursor { bytes, at: 0 };
    if c.take(4, "the header").map_err(|_| "not a VOX file: it has no header".to_string())? != b"VOX " {
        return Err("not a VOX file: the header is not \"VOX \"".into());
    }
    let version = c.i32("the version")?;
    if version != 150 && version != 200 {
        return Err(format!("the VOX version {version} is not read (150 and 200 are)"));
    }
    let id = c.take(4, "the MAIN chunk")?;
    if id != b"MAIN" {
        return Err("the first chunk is not MAIN".into());
    }
    let content = c.count("the MAIN content")?;
    let children = c.count("the MAIN children")?;
    c.take(content, "the MAIN content")?;
    if children > c.left() {
        return Err(format!(
            "the MAIN chunk says it has {children} bytes of children and the file has {} left",
            c.left()
        ));
    }
    let mut body = Cursor { bytes: &bytes[c.at..c.at + children], at: 0 };
    let mut vox =
        Vox { version, models: Vec::new(), palette: None, materials: BTreeMap::new(), nodes: BTreeMap::new() };
    let mut size: Option<[u32; 3]> = None;
    let mut pack: Option<usize> = None;
    while body.left() > 0 {
        let id: [u8; 4] = body.take(4, "a chunk id")?.try_into().expect("four bytes");
        let content = body.count("a chunk's content size")?;
        let kids = body.count("a chunk's children size")?;
        let mut data = Cursor { bytes: body.take(content, "a chunk's content")?, at: 0 };
        body.take(kids, "a chunk's children")?;
        match &id {
            b"PACK" => pack = Some(data.count("PACK")?),
            b"SIZE" => {
                let s = [data.i32("SIZE")?, data.i32("SIZE")?, data.i32("SIZE")?];
                if s.iter().any(|v| *v < 1 || *v as u32 > MAX_SIDE) {
                    return Err(format!("SIZE {s:?} is not a model size (1 to {MAX_SIDE} to a side)"));
                }
                size = Some(s.map(|v| v as u32));
            }
            b"XYZI" => {
                let size = size.take().ok_or("an XYZI chunk comes with no SIZE before it")?;
                if vox.models.len() >= bounds.max_models {
                    return Err(format!("the file has more than {} models", bounds.max_models));
                }
                let n = data.count("XYZI")?;
                if n != data.left() / 4 || data.left() % 4 != 0 {
                    return Err(format!("XYZI says {n} voxels and holds {} bytes of them", data.left()));
                }
                let mut voxels = Vec::with_capacity(n);
                let mut seen = BTreeSet::new();
                for _ in 0..n {
                    let v: [u8; 4] = data.take(4, "XYZI")?.try_into().expect("four bytes");
                    if (0..3).any(|a| u32::from(v[a]) >= size[a]) {
                        return Err(format!("the voxel {:?} is outside its model of {size:?}", &v[..3]));
                    }
                    if v[3] == 0 {
                        return Err(format!("the voxel {:?} has the colour index 0 (1 to 255)", &v[..3]));
                    }
                    if !seen.insert([v[0], v[1], v[2]]) {
                        return Err(format!("the voxel {:?} is in the model twice", &v[..3]));
                    }
                    voxels.push(v);
                }
                vox.models.push(Model { size, voxels });
            }
            b"RGBA" => {
                if data.left() != 1024 {
                    return Err(format!("RGBA holds {} bytes and a palette is 1024", data.left()));
                }
                let mut palette = Box::new([[0u8; 4]; 256]);
                for entry in palette.iter_mut() {
                    *entry = data.take(4, "RGBA")?.try_into().expect("four bytes");
                }
                vox.palette = Some(palette);
            }
            b"MATL" => {
                let index = data.i32("MATL")?;
                let dictionary = data.dictionary("MATL")?;
                if let Ok(index) = u32::try_from(index) {
                    vox.materials.insert(index, dictionary);
                }
            }
            b"nTRN" => {
                let node = data.i32("nTRN")?;
                data.dictionary("nTRN")?;
                let child = data.i32("nTRN")?;
                data.i32("nTRN")?;
                data.i32("nTRN")?;
                let frames = data.count("nTRN")?;
                let mut placement = Placement::IDENTITY;
                if frames > data.left() / 4 {
                    return Err(format!("nTRN says {frames} frames in {} bytes", data.left()));
                }
                if frames > 0 {
                    // the first frame places the node; the rest are animation
                    let frame = data.dictionary("nTRN")?;
                    if let Some(r) = frame.get("_r") {
                        let byte: u8 = r.trim().parse().map_err(|_| format!("the rotation \"{r}\" is not a byte"))?;
                        placement.rotation = Rotation::from_byte(byte)?;
                    }
                    if let Some(t) = frame.get("_t") {
                        let parts: Vec<i64> = t
                            .split_whitespace()
                            .map(|p| p.parse().map_err(|_| format!("the translation \"{t}\" is not three integers")))
                            .collect::<Result<_, _>>()?;
                        if parts.len() != 3 {
                            return Err(format!("the translation \"{t}\" is not three integers"));
                        }
                        // a translation is bounded where it is read, so that the sum of up to `max_depth` of them cannot wrap
                        if parts.iter().any(|c| !(-REACH..=REACH).contains(c)) {
                            return Err(format!("the translation \"{t}\" is too far from the origin"));
                        }
                        placement.translation = [parts[0], parts[1], parts[2]];
                    }
                }
                insert_node(&mut vox, bounds, node, Node::Transform { child, placement })?;
            }
            b"nGRP" => {
                let node = data.i32("nGRP")?;
                data.dictionary("nGRP")?;
                let n = data.count("nGRP")?;
                if n > data.left() / 4 {
                    return Err(format!("nGRP says {n} children in {} bytes", data.left()));
                }
                let children = (0..n).map(|_| data.i32("nGRP")).collect::<Result<Vec<_>, _>>()?;
                insert_node(&mut vox, bounds, node, Node::Group { children })?;
            }
            b"nSHP" => {
                let node = data.i32("nSHP")?;
                data.dictionary("nSHP")?;
                let n = data.count("nSHP")?;
                if n > data.left() / 8 {
                    return Err(format!("nSHP says {n} models in {} bytes", data.left()));
                }
                let mut models = Vec::with_capacity(n);
                for _ in 0..n {
                    let m = data.count("nSHP")?;
                    data.dictionary("nSHP")?;
                    models.push(m);
                }
                insert_node(&mut vox, bounds, node, Node::Shape { models })?;
            }
            _ => {}
        }
    }
    if vox.models.is_empty() {
        return Err("the file has no model".into());
    }
    if let Some(n) = pack {
        if n != vox.models.len() {
            return Err(format!("PACK says {n} models and the file has {}", vox.models.len()));
        }
    }
    for node in vox.nodes.values() {
        if let Node::Shape { models } = node {
            if let Some(m) = models.iter().find(|m| **m >= vox.models.len()) {
                return Err(format!("the scene graph uses the model {m} and the file has {}", vox.models.len()));
            }
        }
    }
    Ok(vox)
}

fn insert_node(vox: &mut Vox, bounds: &Bounds, id: i32, node: Node) -> Result<(), String> {
    if vox.nodes.len() >= bounds.max_nodes {
        return Err(format!("the scene graph has more than {} nodes", bounds.max_nodes));
    }
    if vox.nodes.insert(id, node).is_some() {
        return Err(format!("the scene graph has the node {id} twice"));
    }
    Ok(())
}

impl Vox {
    /// The shapes the scene graph places, in the order of the walk from its root: the model and where it is. `None` if the file has no
    /// scene graph.
    fn placed(&self, bounds: &Bounds) -> Result<Option<Vec<(usize, Placement)>>, String> {
        let referenced: BTreeSet<i32> = self
            .nodes
            .values()
            .flat_map(|n| match n {
                Node::Transform { child, .. } => vec![*child],
                Node::Group { children } => children.clone(),
                Node::Shape { .. } => Vec::new(),
            })
            .collect();
        let root = self
            .nodes
            .iter()
            .find(|(id, n)| matches!(n, Node::Transform { .. }) && !referenced.contains(id))
            .map(|(id, _)| *id);
        let Some(root) = root else { return Ok(None) };
        let mut out = Vec::new();
        self.walk(root, Placement::IDENTITY, &mut Vec::new(), bounds, &mut out)?;
        Ok(Some(out))
    }

    fn walk(
        &self,
        id: i32,
        above: Placement,
        path: &mut Vec<i32>,
        bounds: &Bounds,
        out: &mut Vec<(usize, Placement)>,
    ) -> Result<(), String> {
        if path.len() >= bounds.max_depth {
            return Err(format!("the scene graph is more than {} deep", bounds.max_depth));
        }
        if path.contains(&id) {
            return Err(format!("the scene graph has a cycle through the node {id}"));
        }
        let node = self.nodes.get(&id).ok_or_else(|| format!("the scene graph names the node {id} and has none"))?;
        path.push(id);
        match node {
            Node::Transform { child, placement } => self.walk(*child, above.after(placement), path, bounds, out)?,
            Node::Group { children } => {
                for child in children {
                    self.walk(*child, above, path, bounds, out)?;
                }
            }
            Node::Shape { models } => out.extend(models.iter().map(|m| (*m, above))),
        }
        path.pop();
        Ok(())
    }

    /// The cells of the file in the scene's lattice: one model, unplaced, or every model as the scene graph places it.
    pub fn occupancy(&self, model: Option<usize>, limits: Limits, bounds: &Bounds) -> Result<Imported, String> {
        let mut cells: BTreeMap<[i32; 3], u8> = BTreeMap::new();
        let cap = |count: u64| -> Result<(), String> {
            if count > limits.max_cells {
                return Err(format!("the file places {count} cells and the limit is {} cells", limits.max_cells));
            }
            Ok(())
        };
        match model {
            Some(m) => {
                let model = self.models.get(m).ok_or_else(|| {
                    format!("the file has {} models and the model {m} was asked for", self.models.len())
                })?;
                cap(model.voxels.len() as u64)?;
                for v in &model.voxels {
                    cells.insert(scene_cell([i32::from(v[0]), i32::from(v[1]), i32::from(v[2])]), v[3]);
                }
            }
            None => match self.placed(bounds)? {
                None => {
                    if self.models.len() != 1 {
                        return Err(format!(
                            "the file has {} models and no scene graph to place them: name one with model",
                            self.models.len()
                        ));
                    }
                    cap(self.models[0].voxels.len() as u64)?;
                    for v in &self.models[0].voxels {
                        cells.insert(scene_cell([i32::from(v[0]), i32::from(v[1]), i32::from(v[2])]), v[3]);
                    }
                }
                Some(shapes) => {
                    // a model used many times places its cells many times: the count is of what is placed
                    cap(shapes.iter().map(|(m, _)| self.models[*m].voxels.len() as u64).sum())?;
                    for (m, placement) in shapes {
                        let model = &self.models[m];
                        let centre = model.size.map(|s| i64::from(s / 2));
                        for v in &model.voxels {
                            let local =
                                [i64::from(v[0]) - centre[0], i64::from(v[1]) - centre[1], i64::from(v[2]) - centre[2]];
                            let r = placement.rotation.apply(local);
                            let p: [i64; 3] = std::array::from_fn(|k| r[k] + placement.translation[k]);
                            if p.iter().any(|c| !(-REACH..=REACH).contains(c)) {
                                return Err(format!("the scene graph places a cell at {p:?}, too far from the origin"));
                            }
                            cells.insert(scene_cell([p[0] as i32, p[1] as i32, p[2] as i32]), v[3]);
                        }
                    }
                }
            },
        }
        let mut occupancy = Occupancy::from_cells_with_limits(limits, cells)?;
        // the colour of palette index c is the file's entry c - 1; with no RGBA chunk it is the default palette's entry c
        let colours = if self.palette.is_some() { Colours::File } else { Colours::Default };
        let mut colors = [[0u8; 4]; 256];
        for c in 1..=255usize {
            colors[c] = self.palette.as_ref().map_or(DEFAULT_PALETTE[c], |file| file[c - 1]);
        }
        occupancy.set_palette(colors);
        let materials = self
            .materials
            .iter()
            .filter_map(|(i, m)| u8::try_from(*i).ok().filter(|i| *i != 0).map(|i| (i, m.clone())))
            .collect();
        Ok(Imported { occupancy, colours, materials })
    }
}

/// Reads a file and makes its cells: `model` is one model by its number, unplaced, or every model as the scene graph places them.
pub fn import(bytes: &[u8], model: Option<usize>, limits: Limits, bounds: &Bounds) -> Result<Imported, String> {
    parse(bytes, bounds)?.occupancy(model, limits, bounds)
}
