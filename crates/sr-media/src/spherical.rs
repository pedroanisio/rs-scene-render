//! Spherical video metadata for MP4/MOV files: Google Spherical Video V1
//! (an XMP `uuid` box in the video track) and V2 (`st3d` and `sv3d` boxes in
//! the video sample entry, with `equi` or `cbmp` projections). The `moov`
//! box is rebuilt with the new boxes and chunk offsets move when `moov`
//! precedes the media data.

use std::path::Path;

/// Projection written into the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Equirectangular,
    /// 3×2 cubemap: right, left, up / down, front, back (V2 `cbmp` layout 0).
    Cubemap,
}

/// Stereo packing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stereo {
    Mono,
    TopBottom,
    LeftRight,
}

const V1_UUID: [u8; 16] =
    [0xff, 0xcc, 0x82, 0x63, 0xf8, 0x55, 0x4a, 0x93, 0x88, 0x14, 0x58, 0x7a, 0x02, 0x52, 0x1f, 0xdd];

#[derive(Clone, Debug)]
struct Mp4Box {
    typ: [u8; 4],
    /// Bytes between the header and the children (containers), or the whole payload (leaves).
    head: Vec<u8>,
    children: Option<Vec<Mp4Box>>,
}

/// Payload bytes before the child boxes of a container type.
fn container_prefix(typ: &[u8; 4]) -> Option<usize> {
    Some(match typ {
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"dinf" | b"udta" | b"sv3d" | b"proj" => 0,
        b"stsd" => 8,
        // visual sample entries: 8 reserved/index bytes + 70 bytes of visual fields
        b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"av01" | b"vp09" | b"mp4v" | b"apch" | b"apcn" | b"apcs" | b"apco"
        | b"ap4h" | b"ap4x" => 78,
        _ => return None,
    })
}

fn parse(data: &[u8]) -> Result<Vec<Mp4Box>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= data.len() {
        let mut size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as u64;
        let typ: [u8; 4] = data[i + 4..i + 8].try_into().unwrap();
        let mut hdr = 8;
        if size == 1 {
            size = u64::from_be_bytes(data.get(i + 8..i + 16).ok_or("truncated box")?.try_into().unwrap());
            hdr = 16;
        } else if size == 0 {
            size = (data.len() - i) as u64;
        }
        let end = i + size as usize;
        if size < hdr as u64 || end > data.len() {
            return Err(format!("bad box {}", String::from_utf8_lossy(&typ)));
        }
        let payload = &data[i + hdr..end];
        let b = match container_prefix(&typ) {
            Some(k) if payload.len() >= k => {
                Mp4Box { typ, head: payload[..k].to_vec(), children: Some(parse(&payload[k..])?) }
            }
            _ => Mp4Box { typ, head: payload.to_vec(), children: None },
        };
        out.push(b);
        i = end;
    }
    Ok(out)
}

fn write(b: &Mp4Box, out: &mut Vec<u8>) {
    let start = out.len();
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&b.typ);
    out.extend_from_slice(&b.head);
    if let Some(c) = &b.children {
        for k in c {
            write(k, out);
        }
    }
    let size = (out.len() - start) as u32;
    out[start..start + 4].copy_from_slice(&size.to_be_bytes());
}

fn leaf(typ: &[u8; 4], payload: Vec<u8>) -> Mp4Box {
    Mp4Box { typ: *typ, head: payload, children: None }
}

fn full(typ: &[u8; 4], body: &[u8]) -> Mp4Box {
    let mut p = vec![0, 0, 0, 0];
    p.extend_from_slice(body);
    leaf(typ, p)
}

fn is_video_trak(t: &Mp4Box) -> bool {
    let Some(c) = &t.children else { return false };
    c.iter()
        .filter(|b| &b.typ == b"mdia")
        .flat_map(|m| m.children.iter().flatten())
        .any(|h| &h.typ == b"hdlr" && h.head.get(8..12) == Some(b"vide"))
}

fn find_mut<'a>(b: &'a mut Mp4Box, path: &[&[u8; 4]]) -> Option<&'a mut Mp4Box> {
    if path.is_empty() {
        return Some(b);
    }
    let c = b.children.as_mut()?.iter_mut().find(|k| &k.typ == path[0])?;
    find_mut(c, &path[1..])
}

fn shift_offsets(b: &mut Mp4Box, delta: i64) {
    if let Some(c) = &mut b.children {
        for k in c {
            shift_offsets(k, delta);
        }
        return;
    }
    let (width, ok) = match &b.typ {
        b"stco" => (4, true),
        b"co64" => (8, true),
        _ => (0, false),
    };
    if !ok || b.head.len() < 8 {
        return;
    }
    let n = u32::from_be_bytes(b.head[4..8].try_into().unwrap()) as usize;
    for k in 0..n {
        let o = 8 + k * width;
        if o + width > b.head.len() {
            break;
        }
        if width == 4 {
            let v = u32::from_be_bytes(b.head[o..o + 4].try_into().unwrap()) as i64 + delta;
            b.head[o..o + 4].copy_from_slice(&(v as u32).to_be_bytes());
        } else {
            let v = u64::from_be_bytes(b.head[o..o + 8].try_into().unwrap()) as i64 + delta;
            b.head[o..o + 8].copy_from_slice(&(v as u64).to_be_bytes());
        }
    }
}

/// Writes V1 and V2 spherical metadata into an MP4/MOV file in place.
pub fn inject(path: &Path, projection: Projection, stereo: Stereo) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let top = parse(&data)?;
    // byte ranges of the top-level boxes
    let mut ranges = Vec::new();
    let mut i = 0usize;
    for _ in &top {
        let size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
        let size = if size == 1 {
            u64::from_be_bytes(data[i + 8..i + 16].try_into().unwrap()) as usize
        } else if size == 0 {
            data.len() - i
        } else {
            size
        };
        ranges.push((i, i + size));
        i += size;
    }
    let mi = top.iter().position(|b| &b.typ == b"moov").ok_or("no moov box")?;
    let mdat_after = top.iter().enumerate().any(|(k, b)| &b.typ == b"mdat" && k > mi);
    let mut moov = top[mi].clone();
    let trak = moov
        .children
        .as_mut()
        .ok_or("empty moov")?
        .iter_mut()
        .find(|t| &t.typ == b"trak" && is_video_trak(t))
        .ok_or("no video track")?;
    // V1: XMP uuid in the track
    let stereo_name = match stereo {
        Stereo::Mono => None,
        Stereo::TopBottom => Some("top-bottom"),
        Stereo::LeftRight => Some("left-right"),
    };
    if projection == Projection::Equirectangular {
        let xmp = format!(
            "<?xml version=\"1.0\"?><rdf:SphericalVideo xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:GSpherical=\"http://ns.google.com/videos/1.0/spherical/\"><GSpherical:Spherical>true</GSpherical:Spherical><GSpherical:Stitched>true</GSpherical:Stitched><GSpherical:StitchingSoftware>scene-render</GSpherical:StitchingSoftware><GSpherical:ProjectionType>equirectangular</GSpherical:ProjectionType>{}</rdf:SphericalVideo>",
            stereo_name.map(|s| format!("<GSpherical:StereoMode>{s}</GSpherical:StereoMode>")).unwrap_or_default()
        );
        let mut p = V1_UUID.to_vec();
        p.extend_from_slice(xmp.as_bytes());
        let c = trak.children.as_mut().ok_or("empty trak")?;
        c.retain(|b| !(&b.typ == b"uuid" && b.head.starts_with(&V1_UUID)));
        c.push(leaf(b"uuid", p));
    }
    // V2: st3d + sv3d in the visual sample entry
    let stsd = find_mut(trak, &[b"mdia", b"minf", b"stbl", b"stsd"]).ok_or("no stsd")?;
    let entry = stsd.children.as_mut().and_then(|c| c.first_mut()).ok_or("no sample entry")?;
    let Some(ec) = entry.children.as_mut() else {
        return Err(format!("sample entry {} is not a known visual entry", String::from_utf8_lossy(&entry.typ)));
    };
    ec.retain(|b| &b.typ != b"st3d" && &b.typ != b"sv3d");
    let mode = match stereo {
        Stereo::Mono => 0u8,
        Stereo::TopBottom => 1,
        Stereo::LeftRight => 2,
    };
    ec.push(full(b"st3d", &[mode]));
    let mut svhd = b"scene-render".to_vec();
    svhd.push(0);
    let proj_kind = match projection {
        Projection::Equirectangular => full(b"equi", &[0u8; 16]),
        Projection::Cubemap => full(b"cbmp", &[0u8; 8]),
    };
    let proj = Mp4Box { typ: *b"proj", head: Vec::new(), children: Some(vec![full(b"prhd", &[0u8; 12]), proj_kind]) };
    ec.push(Mp4Box { typ: *b"sv3d", head: Vec::new(), children: Some(vec![full(b"svhd", &svhd), proj]) });
    // chunk offsets move by the growth of moov when it precedes the media
    let old_len = ranges[mi].1 - ranges[mi].0;
    let mut tmp = Vec::new();
    write(&moov, &mut tmp);
    let delta = tmp.len() as i64 - old_len as i64;
    if mdat_after {
        shift_offsets(&mut moov, delta);
    }
    let mut out = Vec::with_capacity(data.len() + delta.max(0) as usize);
    for (k, (s, e)) in ranges.iter().enumerate() {
        if k == mi {
            write(&moov, &mut out);
        } else {
            out.extend_from_slice(&data[*s..*e]);
        }
    }
    let tmp_path = path.with_extension("spherical.tmp");
    std::fs::write(&tmp_path, &out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp_path, path).map_err(|e| e.to_string())
}

/// Reads back (projection, stereo) from V2 boxes (tests and tools).
pub fn read(path: &Path) -> Result<Option<(Projection, Stereo)>, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut top = parse(&data)?;
    let moov = top.iter_mut().find(|b| &b.typ == b"moov").ok_or("no moov")?;
    let Some(trak) = moov.children.as_mut().and_then(|c| c.iter_mut().find(|t| &t.typ == b"trak" && is_video_trak(t)))
    else {
        return Ok(None);
    };
    let Some(stsd) = find_mut(trak, &[b"mdia", b"minf", b"stbl", b"stsd"]) else { return Ok(None) };
    let Some(entry) = stsd.children.as_ref().and_then(|c| c.first()) else { return Ok(None) };
    let ec = entry.children.as_deref().unwrap_or(&[]);
    let Some(sv3d) = ec.iter().find(|b| &b.typ == b"sv3d") else { return Ok(None) };
    let proj = sv3d
        .children
        .iter()
        .flatten()
        .find(|b| &b.typ == b"proj")
        .and_then(|p| p.children.as_ref())
        .ok_or("sv3d without proj")?;
    let projection =
        if proj.iter().any(|b| &b.typ == b"cbmp") { Projection::Cubemap } else { Projection::Equirectangular };
    let stereo = match ec.iter().find(|b| &b.typ == b"st3d").and_then(|b| b.head.get(4)) {
        Some(1) => Stereo::TopBottom,
        Some(2) => Stereo::LeftRight,
        _ => Stereo::Mono,
    };
    Ok(Some((projection, stereo)))
}
