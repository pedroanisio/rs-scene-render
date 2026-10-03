//! OpenVDB io/Compression.h mask and block decoding. Compressed sizes are
//! checked against the expected node payload before calling a decompressor.
use super::{read::Input, Kind, Value};
use crate::Error;
use std::io::{Read, Seek};

pub(super) fn bit(mask: &[u64], i: usize) -> bool {
    mask[i / 64] & (1 << (i % 64)) != 0
}

impl Kind {
    pub fn width(self, half: bool) -> usize {
        self.components() * if half { 2 } else { self.scalar_bytes }
    }
    pub fn parse(self, bytes: &[u8], half: bool) -> Result<Value, Error> {
        let size = if half { 2 } else { self.scalar_bytes };
        let mut out = [0.; 3];
        for (component, chunk) in out.iter_mut().zip(bytes.chunks_exact(size)) {
            let n = match size {
                2 => f64::from(half::f16::from_bits(u16::from_le_bytes(chunk.try_into().unwrap())).to_f32()),
                4 => f64::from(f32::from_le_bytes(chunk.try_into().unwrap())),
                8 => f64::from_le_bytes(chunk.try_into().unwrap()),
                _ => unreachable!("validated scalar kind"),
            };
            if !n.is_finite() || n.abs() > f64::from(f32::MAX) {
                return Err(Error::Invalid("OpenVDB sample exceeds finite f32 range"));
            }
            *component = n as f32;
        }
        Ok(out)
    }
    pub fn raw<R: Read + Seek>(self, r: &mut Input<R>) -> Result<Value, Error> {
        let mut bytes = [0; 24];
        let n = self.width(false);
        r.read(&mut bytes[..n])?;
        self.parse(&bytes[..n], false)
    }
}

pub(super) fn values<R: Read + Seek>(
    r: &mut Input<R>,
    count: usize,
    mask: &[u64],
    kind: Kind,
    half: bool,
    compression: u32,
    bg: Value,
) -> Result<Vec<Value>, Error> {
    let mode = r.u8()?;
    if mode > 6 {
        return Err(Error::Invalid("OpenVDB node mask encoding"));
    }
    let mut low = if mode == 0 { bg } else { bg.map(|v| -v) };
    let mut high = bg;
    if [2, 4, 5].contains(&mode) {
        low = kind.raw(r)?;
    }
    if mode == 5 {
        high = kind.raw(r)?;
    }
    let select = if [3, 4, 5].contains(&mode) { r.mask(count)? } else { Vec::new() };
    let compact = compression & 2 != 0 && mode != 6;
    let stored = if compact { mask.iter().map(|v| v.count_ones() as usize).sum() } else { count };
    let expected = stored * kind.width(half);
    let raw = if half && stored == 0 { Vec::new() } else { block(r, expected, compression)? };
    let mut source = raw.chunks_exact(kind.width(half));
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(if !compact || bit(mask, i) {
            kind.parse(source.next().ok_or(Error::Invalid("OpenVDB value count"))?, half)?
        } else if !select.is_empty() && bit(&select, i) {
            high
        } else {
            low
        });
    }
    if source.next().is_some() {
        return Err(Error::Invalid("OpenVDB trailing values"));
    }
    Ok(out)
}

fn block<R: Read + Seek>(r: &mut Input<R>, expected: usize, compression: u32) -> Result<Vec<u8>, Error> {
    if compression & 5 == 0 {
        let mut data = vec![0; expected];
        r.read(&mut data)?;
        return Ok(data);
    }
    let size = r.i64()?;
    if size <= 0 {
        if size.checked_neg().and_then(|n| usize::try_from(n).ok()) != Some(expected) {
            return Err(Error::Invalid("OpenVDB uncompressed block size"));
        }
        let mut data = vec![0; expected];
        r.read(&mut data)?;
        return Ok(data);
    }
    let size = usize::try_from(size).map_err(|_| Error::Limit("OpenVDB compressed block size"))?;
    if size > expected + 65536 {
        return Err(Error::Limit("OpenVDB compressed block exceeds node allowance"));
    }
    let mut packed = vec![0; size];
    r.read(&mut packed)?;
    let mut output = vec![0; expected];
    if compression & 4 != 0 {
        if size < 16 {
            return Err(Error::Invalid("OpenVDB Blosc header"));
        }
        let mut decoded = 0usize;
        // SAFETY: validate receives the actual buffer length and cannot read
        // beyond it. Its decoded size must match our fixed node payload.
        let valid = unsafe { blosc_src::blosc_cbuffer_validate(packed.as_ptr().cast(), packed.len(), &mut decoded) };
        if valid != 0 || decoded != expected {
            return Err(Error::Invalid("OpenVDB Blosc decoded size"));
        }
        // SAFETY: the validated source is alive, the output has exactly the
        // validated size, and the context API uses one thread without globals.
        let n = unsafe {
            blosc_src::blosc_decompress_ctx(packed.as_ptr().cast(), output.as_mut_ptr().cast(), output.len(), 1)
        };
        if n < 0 || n as usize != expected {
            return Err(Error::Invalid("OpenVDB Blosc payload"));
        }
    } else {
        let mut decoder = flate2::read::ZlibDecoder::new(packed.as_slice());
        decoder.read_exact(&mut output)?;
        let mut extra = [0];
        if decoder.read(&mut extra)? != 0 || decoder.total_in() != packed.len() as u64 {
            return Err(Error::Invalid("OpenVDB ZIP size or trailing payload"));
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn blosc_blocks_require_exact_envelope_and_bounded_output() {
        // The official-library fixture's first nonempty Blosc block stores
        // one float. Locate it by its full header, independent of UUID/offsets.
        let file = include_bytes!("../../tests/data/openvdb/density-blosc.vdb");
        let header = [2, 1, 0x33, 4, 4, 0, 0, 0, 4, 0, 0, 0, 20, 0, 0, 0];
        let start = file.windows(16).position(|b| b == header).unwrap();
        let packed = file[start..start + 20].to_vec();
        let decode = |packed: &[u8]| {
            let mut data = (packed.len() as i64).to_le_bytes().to_vec();
            data.extend_from_slice(packed);
            block(&mut Input::new(Cursor::new(data), 1024).unwrap(), 4, 4)
        };
        assert_eq!(decode(&packed).unwrap(), packed[16..]);
        let mut trailing = packed.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err(), "the outer envelope cannot hide trailing bytes");
        for (offset, value) in [(4, u32::MAX), (8, u32::MAX), (12, u32::MAX)] {
            let mut bad = packed.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode(&bad).is_err(), "invalid header field at {offset}");
        }
        assert!(decode(&packed[..19]).is_err());
    }
}
