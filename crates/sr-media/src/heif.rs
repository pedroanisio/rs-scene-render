//! Colour information of HEIF containers (HEIC, AVIF).
//!
//! FFmpeg decodes the pixels of these files but does not hand over their
//! colour profile, so the `colr` property of the primary item is read here:
//! either an ICC profile (`prof`, `rICC`) or ITU-T H.273 code points (`nclx`).

/// A `colr` property.
#[derive(Clone, Debug, PartialEq)]
pub enum Colr {
    /// Embedded ICC profile.
    Icc(Vec<u8>),
    /// Colour primaries, transfer characteristics, matrix coefficients, full range.
    Nclx(u16, u16, u16, bool),
}

/// Whether the bytes start a HEIF file (HEIC, HEIF or AVIF brands).
pub fn is_heif(head: &[u8]) -> bool {
    if head.len() < 12 || &head[4..8] != b"ftyp" {
        return false;
    }
    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let brands = head.get(8..size.min(head.len())).unwrap_or(&[]);
    brands.chunks(4).any(|b| {
        matches!(b, b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"avif" | b"avis")
    })
}

/// Boxes of `data`: (type, payload).
fn boxes(mut data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    while data.len() >= 8 {
        let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as u64;
        let kind: [u8; 4] = data[4..8].try_into().unwrap();
        let (head, size) = match size {
            0 => (8, data.len() as u64),
            1 if data.len() >= 16 => (16, u64::from_be_bytes(data[8..16].try_into().unwrap())),
            _ => (8, size),
        };
        if size < head as u64 || size > data.len() as u64 {
            break;
        }
        out.push((kind, &data[head..size as usize]));
        data = &data[size as usize..];
    }
    out
}

fn colr(p: &[u8]) -> Option<Colr> {
    match p.get(0..4)? {
        b"prof" | b"rICC" => Some(Colr::Icc(p[4..].to_vec())),
        b"nclx" if p.len() >= 11 => {
            let u = |o: usize| u16::from_be_bytes([p[o], p[o + 1]]);
            Some(Colr::Nclx(u(4), u(6), u(8), p[10] & 0x80 != 0))
        }
        _ => None,
    }
}

/// The `colr` properties of the primary item: ICC first, then `nclx`. When
/// the associations cannot be read, the first `colr` of the property container.
pub fn colour(file: &[u8]) -> Vec<Colr> {
    let Some((_, meta)) = boxes(file).into_iter().find(|(k, _)| k == b"meta") else { return Vec::new() };
    let children = boxes(meta.get(4..).unwrap_or(&[]));
    let primary = children.iter().find(|(k, _)| k == b"pitm").and_then(|(_, p)| match p.first()? {
        0 => Some(u16::from_be_bytes([*p.get(4)?, *p.get(5)?]) as u32),
        _ => Some(u32::from_be_bytes(p.get(4..8)?.try_into().ok()?)),
    });
    let Some((_, iprp)) = children.iter().find(|(k, _)| k == b"iprp") else { return Vec::new() };
    let props = boxes(iprp);
    let ipco: Vec<([u8; 4], &[u8])> =
        props.iter().find(|(k, _)| k == b"ipco").map(|(_, p)| boxes(p)).unwrap_or_default();
    let mut indices = Vec::new();
    if let (Some(primary), Some((_, ipma))) = (primary, props.iter().find(|(k, _)| k == b"ipma")) {
        let (version, flags) = (ipma.first().copied().unwrap_or(0), ipma.get(3).copied().unwrap_or(0));
        let mut o = 4;
        let n = ipma.get(4..8).map(|b| u32::from_be_bytes(b.try_into().unwrap())).unwrap_or(0);
        o += 4;
        for _ in 0..n {
            let item = if version < 1 {
                let v = ipma.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as u32);
                o += 2;
                v
            } else {
                let v = ipma.get(o..o + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
                o += 4;
                v
            };
            let Some(item) = item else { break };
            let Some(&count) = ipma.get(o) else { break };
            o += 1;
            for _ in 0..count {
                let index = if flags & 1 != 0 {
                    let v = ipma.get(o..o + 2).map(|b| (u16::from_be_bytes([b[0], b[1]]) & 0x7fff) as usize);
                    o += 2;
                    v
                } else {
                    let v = ipma.get(o).map(|b| (b & 0x7f) as usize);
                    o += 1;
                    v
                };
                if let Some(i) = index.filter(|_| item == primary) {
                    indices.push(i);
                }
            }
        }
    }
    let mut out: Vec<Colr> = indices
        .iter()
        .filter_map(|&i| ipco.get(i.checked_sub(1)?))
        .filter(|(k, _)| k == b"colr")
        .filter_map(|(_, p)| colr(p))
        .collect();
    if out.is_empty() {
        out.extend(ipco.iter().filter(|(k, _)| k == b"colr").filter_map(|(_, p)| colr(p)).take(1));
    }
    out.sort_by_key(|c| matches!(c, Colr::Nclx(..)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend(kind);
        v.extend(payload);
        v
    }

    #[test]
    fn finds_the_primary_items_colour() {
        let ftyp = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        let thumb = bx(b"colr", b"nclx\0\x01\0\x0d\0\x06\x80");
        let main = bx(b"colr", b"profICCBYTES");
        let ipco = bx(b"ipco", &[thumb, main].concat());
        // The thumbnail (item id 1) has property 1; the primary (item id 2) has property 2.
        let ipma = bx(b"ipma", &[0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 1, 0x81, 0, 2, 1, 0x82]);
        let iprp = bx(b"iprp", &[ipco, ipma].concat());
        let pitm = bx(b"pitm", &[0, 0, 0, 0, 0, 2]);
        let meta = bx(b"meta", &[vec![0, 0, 0, 0], pitm, iprp].concat());
        let file = [ftyp, meta].concat();
        assert!(is_heif(&file));
        assert_eq!(colour(&file), vec![Colr::Icc(b"ICCBYTES".to_vec())]);
        assert!(!is_heif(b"\x89PNG\r\n\x1a\n\0\0\0\0"));
    }
}
