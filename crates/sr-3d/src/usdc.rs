//! Reader for binary USD layers ("crate" files, `.usdc`, version 0.4.0 and later).
//!
//! A crate holds a table of contents pointing at six sections: tokens (LZ4-compressed text),
//! strings (token indices), fields (a token and a value representation each), field sets
//! (runs of field indices), paths (a compressed tree) and specs (path, field set, spec type).
//! Integer tables are delta-coded with USD's integer compression and then LZ4-compressed.
//! A value representation packs a type, array/inline/compressed flags and a 48-bit payload that
//! is either the value itself or its file offset.
//!
//! The reader decodes what geometry import needs: scalars, vectors, matrices and quaternions
//! (inlined or not), their arrays (with compressed integer and float arrays), tokens, strings,
//! asset paths, token and path vectors and list ops, and time samples. Other types decode as
//! [`Val::Other`] and are ignored.

/// A decoded field value.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Bool(bool),
    /// Any number, or the components of one vector, matrix or quaternion (quaternions as w, x, y, z).
    Nums(Vec<f64>),
    /// An array of numbers or of tuples, flattened.
    Array(Vec<f64>),
    Token(String),
    Str(String),
    Asset(String),
    Tokens(Vec<String>),
    Paths(Vec<String>),
    /// (time, value) pairs.
    TimeSamples(Vec<(f64, Val)>),
    Other(u8),
}

/// One spec: its path, type (Sdf spec type) and fields.
#[derive(Clone, Debug)]
pub struct Spec {
    pub path: String,
    pub kind: SpecKind,
    pub fields: Vec<(String, Val)>,
}

impl Spec {
    pub fn get(&self, name: &str) -> Option<&Val> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecKind {
    Attribute,
    Prim,
    PseudoRoot,
    Relationship,
    Other,
}

// ---------------------------------------------------------------- compression

/// LZ4 block decompression (the format TfFastCompression wraps).
fn lz4_block(src: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
    let mut i = 0;
    let len = |i: &mut usize, base: usize| -> Result<usize, String> {
        let mut n = base;
        if base == 15 {
            loop {
                let b = *src.get(*i).ok_or("lz4: truncated length")?;
                *i += 1;
                n += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        Ok(n)
    };
    while i < src.len() {
        let token = src[i];
        i += 1;
        let lit = len(&mut i, (token >> 4) as usize)?;
        let end = i.checked_add(lit).filter(|&e| e <= src.len()).ok_or("lz4: literals overrun")?;
        out.extend_from_slice(&src[i..end]);
        i = end;
        if i >= src.len() {
            break;
        }
        let off = u16::from_le_bytes([src[i], *src.get(i + 1).ok_or("lz4: truncated offset")?]) as usize;
        i += 2;
        if off == 0 || off > out.len() {
            return Err("lz4: bad offset".into());
        }
        let n = len(&mut i, (token & 15) as usize)? + 4;
        let start = out.len() - off;
        for k in 0..n {
            let b = out[start + k];
            out.push(b);
        }
    }
    Ok(())
}

/// TfFastCompression: a chunk count byte, then one LZ4 block (count 0) or `count` blocks each
/// preceded by its i32 size.
fn fast_decompress(src: &[u8]) -> Result<Vec<u8>, String> {
    let (&chunks, rest) = src.split_first().ok_or("empty compressed block")?;
    let mut out = Vec::new();
    if chunks == 0 {
        lz4_block(rest, &mut out)?;
    } else {
        let mut i = 0;
        for _ in 0..chunks {
            let n = i32::from_le_bytes(rest.get(i..i + 4).ok_or("truncated chunk")?.try_into().unwrap()) as usize;
            i += 4;
            lz4_block(rest.get(i..i + n).ok_or("truncated chunk")?, &mut out)?;
            i += n;
        }
    }
    Ok(out)
}

/// USD integer compression: a common delta, 2-bit codes, then 8/16/32-bit (or 16/32/64-bit for
/// 64-bit integers) deltas; the running sum is the value.
fn decode_ints(data: &[u8], n: usize, wide: bool) -> Result<Vec<i64>, String> {
    let w = if wide { 8 } else { 4 };
    let rd = |b: &[u8], size: usize| -> i64 {
        match size {
            1 => b[0] as i8 as i64,
            2 => i16::from_le_bytes([b[0], b[1]]) as i64,
            4 => i32::from_le_bytes(b[..4].try_into().unwrap()) as i64,
            _ => i64::from_le_bytes(b[..8].try_into().unwrap()),
        }
    };
    let common = rd(data.get(..w).ok_or("ints: truncated")?, w);
    let codes = data.get(w..w + (n * 2).div_ceil(8)).ok_or("ints: truncated codes")?;
    let mut v = w + codes.len();
    let (small, medium) = if wide { (2, 4) } else { (1, 2) };
    let mut prev = 0i64;
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let code = (codes[k / 4] >> (2 * (k % 4))) & 3;
        let size = [0, small, medium, w][code as usize];
        let d = if size == 0 {
            common
        } else {
            let b = data.get(v..v + size).ok_or("ints: truncated values")?;
            v += size;
            rd(b, size)
        };
        // 32-bit tables accumulate in 32 bits, as the writer's running sum does
        prev = if wide { prev.wrapping_add(d) } else { (prev as i32).wrapping_add(d as i32) as i64 };
        out.push(prev);
    }
    Ok(out)
}

// ---------------------------------------------------------------- reader

struct Crate<'a> {
    d: &'a [u8],
    tokens: Vec<String>,
    strings: Vec<u32>,
    paths: Vec<String>,
    minor: u8,
}

struct Cur<'a> {
    d: &'a [u8],
    i: usize,
}

impl Cur<'_> {
    fn bytes(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.i.checked_add(n).filter(|&e| e <= self.d.len()).ok_or("usdc: read past the end")?;
        let b = &self.d[self.i..end];
        self.i = end;
        Ok(b)
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, String> {
        Ok(self.u64()? as i64)
    }
    /// A compressed integer table of `n` entries: its compressed size, then the data.
    fn ints(&mut self, n: usize, wide: bool) -> Result<Vec<i64>, String> {
        let size = self.u64()? as usize;
        let raw = fast_decompress(self.bytes(size)?)?;
        decode_ints(&raw, n, wide)
    }
}

/// Bytes per component, components, and the component decoder of a numeric type.
type Layout<'f> = (usize, usize, &'f dyn Fn(&[u8]) -> f64);

const ARRAY: u64 = 1 << 63;
const INLINE: u64 = 1 << 62;
const COMPRESSED: u64 = 1 << 61;

impl Crate<'_> {
    fn at(&self, i: usize) -> Cur<'_> {
        Cur { d: self.d, i }
    }

    fn token(&self, i: u64) -> String {
        self.tokens.get(i as usize).cloned().unwrap_or_default()
    }

    fn path(&self, i: u64) -> String {
        self.paths.get(i as usize).cloned().unwrap_or_default()
    }

    /// Element count of an array at the cursor (u32 before 0.7.0).
    fn count(&self, c: &mut Cur) -> Result<usize, String> {
        Ok(if self.minor >= 7 { c.u64()? as usize } else { c.u32()? as usize })
    }

    /// `n` scalars of `size` bytes decoded by `f`.
    fn raw(c: &mut Cur, n: usize, size: usize, f: &dyn Fn(&[u8]) -> f64) -> Result<Vec<f64>, String> {
        let b = c.bytes(n.checked_mul(size).ok_or("usdc: array too large")?)?;
        Ok(b.chunks_exact(size).map(f).collect())
    }

    fn value(&self, rep: u64) -> Result<Val, String> {
        let ty = ((rep >> 48) & 0xff) as u8;
        let payload = rep & ((1 << 48) - 1);
        let array = rep & ARRAY != 0;
        let inline = rep & INLINE != 0;
        let compressed = rep & COMPRESSED != 0;
        // scalar layouts: (bytes per component, components, decoder)
        let f32d = |b: &[u8]| f32::from_le_bytes(b[..4].try_into().unwrap()) as f64;
        let f64d = |b: &[u8]| f64::from_le_bytes(b[..8].try_into().unwrap());
        let f16d = |b: &[u8]| half::f16::from_le_bytes([b[0], b[1]]).to_f64();
        let i32d = |b: &[u8]| i32::from_le_bytes(b[..4].try_into().unwrap()) as f64;
        let layout: Option<Layout> = match ty {
            1 | 2 => Some((1, 1, &|b: &[u8]| b[0] as f64)),
            3 => Some((4, 1, &i32d)),
            4 => Some((4, 1, &|b: &[u8]| u32::from_le_bytes(b[..4].try_into().unwrap()) as f64)),
            5 => Some((8, 1, &|b: &[u8]| i64::from_le_bytes(b[..8].try_into().unwrap()) as f64)),
            6 => Some((8, 1, &|b: &[u8]| u64::from_le_bytes(b[..8].try_into().unwrap()) as f64)),
            7 => Some((2, 1, &f16d)),
            8 => Some((4, 1, &f32d)),
            9 | 56 => Some((8, 1, &f64d)),
            13 => Some((8, 4, &f64d)),
            14 => Some((8, 9, &f64d)),
            15 => Some((8, 16, &f64d)),
            16 => Some((8, 4, &f64d)),
            17 => Some((4, 4, &f32d)),
            18 => Some((2, 4, &f16d)),
            19 => Some((8, 2, &f64d)),
            20 => Some((4, 2, &f32d)),
            21 => Some((2, 2, &f16d)),
            22 => Some((4, 2, &i32d)),
            23 => Some((8, 3, &f64d)),
            24 => Some((4, 3, &f32d)),
            25 => Some((2, 3, &f16d)),
            26 => Some((4, 3, &i32d)),
            27 => Some((8, 4, &f64d)),
            28 => Some((4, 4, &f32d)),
            29 => Some((2, 4, &f16d)),
            30 => Some((4, 4, &i32d)),
            _ => None,
        };
        // quaternions are stored imaginary first (x, y, z, w); text order is w, x, y, z
        let quat_fix = |v: &mut Vec<f64>| {
            for q in v.chunks_exact_mut(4) {
                q.rotate_right(1);
            }
        };
        if let Some((size, comps, dec)) = layout {
            if array {
                if payload == 0 {
                    return Ok(Val::Array(Vec::new()));
                }
                let mut c = self.at(payload as usize);
                let n = self.count(&mut c)?;
                let mut v = if compressed && comps == 1 {
                    self.compressed_array(&mut c, n, ty, size, dec)?
                } else {
                    Self::raw(&mut c, n * comps, size, dec)?
                };
                if matches!(ty, 16..=18) {
                    quat_fix(&mut v);
                }
                return Ok(Val::Array(v));
            }
            if inline {
                let b = payload.to_le_bytes();
                let mut v: Vec<f64> = match ty {
                    // doubles that fit a float are inlined as floats
                    9 | 56 => vec![f32::from_le_bytes(b[..4].try_into().unwrap()) as f64],
                    // vectors with small integer components are inlined as signed bytes
                    19..=30 => b[..comps].iter().map(|&x| x as i8 as f64).collect(),
                    // diagonal matrices are inlined as their diagonal in signed bytes
                    13..=15 => {
                        let dim = [2, 3, 4][ty as usize - 13];
                        let mut m = vec![0.0; dim * dim];
                        for k in 0..dim {
                            m[k * dim + k] = b[k] as i8 as f64;
                        }
                        m
                    }
                    _ => vec![dec(&b)],
                };
                if matches!(ty, 16..=18) {
                    quat_fix(&mut v);
                }
                return Ok(match ty {
                    1 => Val::Bool(v[0] != 0.0),
                    _ => Val::Nums(v),
                });
            }
            let mut c = self.at(payload as usize);
            let mut v = Self::raw(&mut c, comps, size, dec)?;
            if matches!(ty, 16..=18) {
                quat_fix(&mut v);
            }
            return Ok(Val::Nums(v));
        }
        Ok(match ty {
            // token, string and asset path: an index, or an array of them
            10..=12 => {
                let name = |i: u64| {
                    if ty == 10 {
                        self.strings.get(i as usize).map(|&t| self.token(t as u64)).unwrap_or_default()
                    } else {
                        self.token(i)
                    }
                };
                if array {
                    let mut out = Vec::new();
                    if payload != 0 {
                        let mut c = self.at(payload as usize);
                        let n = self.count(&mut c)?;
                        for _ in 0..n {
                            out.push(name(c.u32()? as u64));
                        }
                    }
                    Val::Tokens(out)
                } else {
                    let s = name(if inline { payload } else { self.at(payload as usize).u32()? as u64 });
                    match ty {
                        10 => Val::Str(s),
                        11 => Val::Token(s),
                        _ => Val::Asset(s),
                    }
                }
            }
            // specifier, permission, variability: small enums, inlined
            42..=44 => Val::Nums(vec![payload as f64]),
            // token and path vectors: u64 count, u32 indices
            40 | 41 => {
                let mut c = self.at(payload as usize);
                let n = c.u64()? as usize;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    let i = c.u32()? as u64;
                    out.push(if ty == 40 { self.path(i) } else { self.token(i) });
                }
                if ty == 40 {
                    Val::Paths(out)
                } else {
                    Val::Tokens(out)
                }
            }
            // token and path list ops: a flags byte, then the explicit, added, prepended,
            // appended, deleted and ordered lists that are present. The value is the explicit
            // list, or else the prepended, appended and added items
            32 | 34 => {
                let mut c = self.at(payload as usize);
                let flags = c.bytes(1)?[0];
                let mut explicit = Vec::new();
                let mut items = Vec::new();
                for (bit, keep) in [(1u8, 0u8), (2, 1), (5, 1), (6, 1), (3, 2), (4, 2)] {
                    if flags & (1 << bit) == 0 {
                        continue;
                    }
                    let n = c.u64()? as usize;
                    for _ in 0..n {
                        let i = c.u32()? as u64;
                        let s = if ty == 34 { self.path(i) } else { self.token(i) };
                        match keep {
                            0 => explicit.push(s),
                            1 => items.push(s),
                            _ => {}
                        }
                    }
                }
                let list = if flags & 1 != 0 { explicit } else { items };
                if ty == 34 {
                    Val::Paths(list)
                } else {
                    Val::Tokens(list)
                }
            }
            // double vector: u64 count, doubles
            48 => {
                let mut c = self.at(payload as usize);
                let n = c.u64()? as usize;
                Val::Array(Self::raw(&mut c, n, 8, &|b: &[u8]| f64::from_le_bytes(b[..8].try_into().unwrap()))?)
            }
            46 => self.time_samples(payload as usize)?,
            _ => Val::Other(ty),
        })
    }

    /// Integer arrays compress with the integer coder; float arrays as integers ('i') or as a
    /// lookup table with compressed indices ('t'). Short arrays are stored plainly.
    fn compressed_array(
        &self,
        c: &mut Cur,
        n: usize,
        ty: u8,
        size: usize,
        dec: &dyn Fn(&[u8]) -> f64,
    ) -> Result<Vec<f64>, String> {
        if n < 16 {
            return Self::raw(c, n, size, dec);
        }
        match ty {
            3..=6 => Ok(c.ints(n, size == 8)?.into_iter().map(|x| x as f64).collect()),
            7..=9 => match c.bytes(1)?[0] {
                b'i' => Ok(c.ints(n, false)?.into_iter().map(|x| x as f64).collect()),
                b't' => {
                    let lut_n = c.u32()? as usize;
                    let lut = Self::raw(c, lut_n, size, dec)?;
                    let idx = c.ints(n, false)?;
                    idx.into_iter()
                        .map(|k| lut.get(k as usize).copied().ok_or_else(|| "usdc: bad table index".to_string()))
                        .collect()
                }
                code => Err(format!("usdc: unknown float array coding {code}")),
            },
            _ => Self::raw(c, n, size, dec),
        }
    }

    /// Time samples: a jump to the times' value rep; reading continues after that rep with a
    /// jump to the values (a count, then one rep per sample).
    fn time_samples(&self, at: usize) -> Result<Val, String> {
        let mut c = self.at(at);
        let jump = c.i64()?;
        let mut t = self.at(at.checked_add_signed(jump as isize).ok_or("usdc: bad jump")?);
        let times = match self.value(t.u64()?)? {
            Val::Array(v) | Val::Nums(v) => v,
            _ => Vec::new(),
        };
        let at2 = t.i;
        let jump = t.i64()?;
        let mut v = self.at(at2.checked_add_signed(jump as isize).ok_or("usdc: bad jump")?);
        let n = v.u64()? as usize;
        let mut out = Vec::with_capacity(n.min(times.len()));
        for k in 0..n {
            out.push((times.get(k).copied().unwrap_or(0.0), self.value(v.u64()?)?));
        }
        Ok(Val::TimeSamples(out))
    }
}

/// Decodes the specs of a crate file.
pub fn read(d: &[u8]) -> Result<Vec<Spec>, String> {
    if !d.starts_with(b"PXR-USDC") || d.len() < 88 {
        return Err("usdc: not a USD crate file, or truncated".into());
    }
    let (major, minor) = (d[8], d[9]);
    if major != 0 || minor < 4 {
        return Err(format!("usdc version {major}.{minor} is not supported (0.4 or later)"));
    }
    let toc = u64::from_le_bytes(d[16..24].try_into().unwrap()) as usize;
    let mut c = Cur { d, i: toc };
    let mut sections = std::collections::HashMap::new();
    for _ in 0..c.u64()? {
        let name = c.bytes(16)?;
        let name = String::from_utf8_lossy(&name[..name.iter().position(|&b| b == 0).unwrap_or(16)]).into_owned();
        let (start, _size) = (c.u64()? as usize, c.u64()?);
        sections.insert(name, start);
    }
    let sec = |n: &str| sections.get(n).copied().ok_or_else(|| format!("usdc: no {n} section"));
    let mut cr = Crate { d, tokens: Vec::new(), strings: Vec::new(), paths: Vec::new(), minor };

    let mut c = Cur { d, i: sec("TOKENS")? };
    let n = c.u64()? as usize;
    let _raw_size = c.u64()?;
    let size = c.u64()? as usize;
    let text = fast_decompress(c.bytes(size)?)?;
    cr.tokens = text.split(|&b| b == 0).take(n).map(|t| String::from_utf8_lossy(t).into_owned()).collect();

    let mut c = Cur { d, i: sec("STRINGS")? };
    let n = c.u64()? as usize;
    cr.strings = (0..n).map(|_| c.u32()).collect::<Result<_, _>>()?;

    let mut c = Cur { d, i: sec("FIELDS")? };
    let n = c.u64()? as usize;
    let field_tokens = c.ints(n, false)?;
    let size = c.u64()? as usize;
    let reps_raw = fast_decompress(c.bytes(size)?)?;
    let reps: Vec<u64> = reps_raw.chunks_exact(8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).collect();
    if reps.len() < n {
        return Err("usdc: truncated field values".into());
    }

    let mut c = Cur { d, i: sec("FIELDSETS")? };
    let n_sets = c.u64()? as usize;
    let sets = c.ints(n_sets, false)?;

    let mut c = Cur { d, i: sec("PATHS")? };
    let n_paths = c.u64()? as usize;
    let n_enc = c.u64()? as usize;
    let idx = c.ints(n_enc, false)?;
    let elem = c.ints(n_enc, false)?;
    let jumps = c.ints(n_enc, false)?;
    cr.paths = vec![String::new(); n_paths];
    // depth-first: a jump > 0 (sibling at +jump, child next) or -1 (child only) means the next
    // entry is a child; >= 0 means a sibling follows
    let mut stack: Vec<(usize, String)> = vec![(0, String::new())];
    while let Some((mut k, mut parent)) = stack.pop() {
        loop {
            if k >= n_enc {
                break;
            }
            let this = k;
            k += 1;
            let path = if parent.is_empty() {
                "/".to_string()
            } else {
                let t = elem[this];
                let name = cr.token(t.unsigned_abs());
                if t < 0 {
                    format!("{parent}.{name}")
                } else if parent == "/" {
                    format!("/{name}")
                } else {
                    format!("{parent}/{name}")
                }
            };
            if let Some(slot) = cr.paths.get_mut(idx[this] as usize) {
                *slot = path.clone();
            }
            let j = jumps[this];
            let (child, sibling) = (j > 0 || j == -1, j >= 0);
            if child {
                if sibling {
                    stack.push(((this as i64 + j) as usize, parent.clone()));
                }
                parent = path;
            } else if !sibling {
                break;
            }
        }
    }

    let mut c = Cur { d, i: sec("SPECS")? };
    let n = c.u64()? as usize;
    let spec_paths = c.ints(n, false)?;
    let spec_sets = c.ints(n, false)?;
    let spec_types = c.ints(n, false)?;
    let mut specs = Vec::with_capacity(n);
    for k in 0..n {
        let mut fields = Vec::new();
        let mut s = spec_sets[k] as usize;
        while let Some(&f) = sets.get(s) {
            if f < 0 || f as u32 == u32::MAX {
                break;
            }
            let f = f as usize;
            let name = cr.token(*field_tokens.get(f).ok_or("usdc: bad field index")? as u64);
            fields.push((name, cr.value(reps[f])?));
            s += 1;
        }
        specs.push(Spec {
            path: cr.path(spec_paths[k] as u64),
            kind: match spec_types[k] {
                1 => SpecKind::Attribute,
                6 => SpecKind::Prim,
                7 => SpecKind::PseudoRoot,
                8 => SpecKind::Relationship,
                _ => SpecKind::Other,
            },
            fields,
        });
    }
    Ok(specs)
}
