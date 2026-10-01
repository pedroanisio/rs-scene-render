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
        let n = usize::try_from(self.varint()?).map_err(|_| "truncated protobuf field")?;
        let end = self.i.checked_add(n).ok_or("truncated protobuf field")?;
        let s = self.b.get(self.i..end).ok_or("truncated protobuf field")?;
        self.i = end;
        Ok(s)
    }
    fn fixed(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.i.checked_add(n).ok_or("truncated protobuf field")?;
        let s = self.b.get(self.i..end).ok_or("truncated protobuf field")?;
        self.i = end;
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
                    x = x.checked_add(zigzag(dx)).ok_or("geometry coordinate overflows")?;
                    y = y.checked_add(zigzag(dy)).ok_or("geometry coordinate overflows")?;
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
            for kv in tags.as_chunks::<2>().0 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(out: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 {
            out.push(v as u8 | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    #[test]
    fn a_field_longer_than_the_address_space_is_truncated() {
        for len in [u64::MAX, u64::MAX - 1, 1 << 40] {
            let mut b = vec![0x1a];
            varint(&mut b, len);
            b.extend([0; 16]);
            assert!(decode(&b).is_err(), "{len}");
        }
        let mut b = vec![0x1a, 3, 0x0d];
        b.extend([1, 2]);
        assert!(decode(&b).is_err());
    }

    #[test]
    fn coordinates_that_overflow_are_errors() {
        let far = u64::MAX - 1; // zigzag of i64::MAX
        assert!(geometry(&[(2 << 3) | 1, far, 0, far, 0]).is_err());
        assert!(geometry(&[(2 << 3) | 1, 0, far, 0, far]).is_err());
        assert!(geometry(&[(1 << 3) | 1, u64::MAX, u64::MAX, (1 << 3) | 2, u64::MAX, u64::MAX]).is_err());
        assert_eq!(geometry(&[(1 << 3) | 1, 4, 6, (1 << 3) | 2, 2, 1]).unwrap(), [[[2.0, 3.0], [3.0, 2.0]]]);
    }
}
