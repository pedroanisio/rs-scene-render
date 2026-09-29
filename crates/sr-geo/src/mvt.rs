//! Mapbox Vector Tiles (MVT 2.1): layers of features whose geometry is in
//! tile coordinates (0‥extent, y down), with typed properties.

use serde_json::{Map, Value};

/// Geometry types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeomType {
    Unknown,
    Point,
    LineString,
    Polygon,
}

/// A feature: its geometry in tile coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub id: Option<u64>,
    pub kind: GeomType,
    pub properties: Map<String, Value>,
    /// Points: one part per point. Lines: one part per line. Polygons: rings, each exterior ring
    /// (positive area in tile coordinates) followed by its holes.
    pub parts: Vec<Vec<[f64; 2]>>,
}

impl Feature {
    /// Polygons: rings grouped as [exterior, holes…] (MVT 2.1 winding).
    pub fn polygons(&self) -> Vec<Vec<Vec<[f64; 2]>>> {
        let mut out: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
        for r in &self.parts {
            let a = signed_area(r);
            if a > 0.0 || out.is_empty() {
                out.push(vec![r.clone()]);
            } else if a < 0.0 {
                out.last_mut().expect("non-empty").push(r.clone());
            }
        }
        out
    }
}

/// Shoelace area in tile coordinates (positive for MVT exterior rings).
pub fn signed_area(r: &[[f64; 2]]) -> f64 {
    let n = r.len();
    (0..n).map(|i| r[i][0] * r[(i + 1) % n][1] - r[(i + 1) % n][0] * r[i][1]).sum::<f64>() / 2.0
}

/// A layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name: String,
    pub extent: u32,
    pub features: Vec<Feature>,
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn varint(&mut self) -> Result<u64, String> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let byte = *self.b.get(self.i).ok_or("truncated protobuf")?;
            self.i += 1;
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
    fn done(&self) -> bool {
        self.i >= self.b.len()
    }
    /// (field number, wire type).
    fn key(&mut self) -> Result<(u64, u8), String> {
        let k = self.varint()?;
        Ok((k >> 3, (k & 7) as u8))
    }
    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let n = self.varint()? as usize;
        let s = self.b.get(self.i..self.i + n).ok_or("truncated protobuf field")?;
        self.i += n;
        Ok(s)
    }
    fn fixed(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.b.get(self.i..self.i + n).ok_or("truncated protobuf field")?;
        self.i += n;
        Ok(s)
    }
    fn skip(&mut self, wire: u8) -> Result<(), String> {
        match wire {
            0 => self.varint().map(|_| ()),
            1 => self.fixed(8).map(|_| ()),
            2 => self.bytes().map(|_| ()),
            5 => self.fixed(4).map(|_| ()),
            w => Err(format!("unsupported protobuf wire type {w}")),
        }
    }
    fn packed(&mut self) -> Result<Vec<u64>, String> {
        let mut r = Reader { b: self.bytes()?, i: 0 };
        let mut out = Vec::new();
        while !r.done() {
            out.push(r.varint()?);
        }
        Ok(out)
    }
}

fn zigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

fn value(b: &[u8]) -> Result<Value, String> {
    let mut r = Reader { b, i: 0 };
    let mut v = Value::Null;
    while !r.done() {
        let (f, w) = r.key()?;
        v = match (f, w) {
            (1, 2) => Value::String(String::from_utf8_lossy(r.bytes()?).into_owned()),
            (2, 5) => serde_json::json!(f32::from_le_bytes(r.fixed(4)?.try_into().unwrap()) as f64),
            (3, 1) => serde_json::json!(f64::from_le_bytes(r.fixed(8)?.try_into().unwrap())),
            (4, 0) => serde_json::json!(r.varint()? as i64),
            (5, 0) => serde_json::json!(r.varint()?),
            (6, 0) => serde_json::json!(zigzag(r.varint()?)),
            (7, 0) => Value::Bool(r.varint()? != 0),
            (_, w) => {
                r.skip(w)?;
                continue;
            }
        };
    }
    Ok(v)
}

fn geometry(cmds: &[u64]) -> Result<Vec<Vec<[f64; 2]>>, String> {
    let mut parts: Vec<Vec<[f64; 2]>> = Vec::new();
    let (mut x, mut y) = (0i64, 0i64);
    let mut i = 0;
    while i < cmds.len() {
        let c = cmds[i];
        i += 1;
        let (id, count) = (c & 7, (c >> 3) as usize);
        match id {
            1 | 2 => {
                for _ in 0..count {
                    let (dx, dy) =
                        (*cmds.get(i).ok_or("truncated geometry")?, *cmds.get(i + 1).ok_or("truncated geometry")?);
                    i += 2;
                    x += zigzag(dx);
                    y += zigzag(dy);
                    let p = [x as f64, y as f64];
                    if id == 1 {
                        parts.push(vec![p]);
                    } else {
                        parts.last_mut().ok_or("LineTo before MoveTo")?.push(p);
                    }
                }
            }
            7 => {} // ClosePath: rings are closed implicitly
            other => return Err(format!("unknown geometry command {other}")),
        }
    }
    Ok(parts)
}

/// Decodes a (decompressed) tile.
pub fn decode(b: &[u8]) -> Result<Vec<Layer>, String> {
    let mut r = Reader { b, i: 0 };
    let mut layers = Vec::new();
    while !r.done() {
        let (f, w) = r.key()?;
        if f != 3 || w != 2 {
            r.skip(w)?;
            continue;
        }
        let mut l = Reader { b: r.bytes()?, i: 0 };
        let (mut name, mut extent) = (String::new(), 4096u32);
        let (mut keys, mut values, mut raw) = (Vec::new(), Vec::new(), Vec::new());
        while !l.done() {
            let (f, w) = l.key()?;
            match (f, w) {
                (1, 2) => name = String::from_utf8_lossy(l.bytes()?).into_owned(),
                (2, 2) => raw.push(l.bytes()?),
                (3, 2) => keys.push(String::from_utf8_lossy(l.bytes()?).into_owned()),
                (4, 2) => values.push(value(l.bytes()?)?),
                (5, 0) => extent = l.varint()? as u32,
                (_, w) => l.skip(w)?,
            }
        }
        let mut features = Vec::with_capacity(raw.len());
        for fb in raw {
            let mut fr = Reader { b: fb, i: 0 };
            let (mut id, mut tags, mut kind, mut geom) = (None, Vec::new(), GeomType::Unknown, Vec::new());
            while !fr.done() {
                let (f, w) = fr.key()?;
                match (f, w) {
                    (1, 0) => id = Some(fr.varint()?),
                    (2, 2) => tags = fr.packed()?,
                    (3, 0) => {
                        kind = match fr.varint()? {
                            1 => GeomType::Point,
                            2 => GeomType::LineString,
                            3 => GeomType::Polygon,
                            _ => GeomType::Unknown,
                        }
                    }
                    (4, 2) => geom = fr.packed()?,
                    (_, w) => fr.skip(w)?,
                }
            }
            let mut properties = Map::new();
            for kv in tags.chunks_exact(2) {
                if let (Some(k), Some(v)) = (keys.get(kv[0] as usize), values.get(kv[1] as usize)) {
                    properties.insert(k.clone(), v.clone());
                }
            }
            features.push(Feature { id, kind, properties, parts: geometry(&geom)? });
        }
        layers.push(Layer { name, extent, features });
    }
    Ok(layers)
}
