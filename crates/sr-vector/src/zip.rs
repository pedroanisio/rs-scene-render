//! A minimal zip reader for dotLottie containers (stored and deflated entries).

/// Bounds on what an archive may hold.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Central-directory entries.
    pub entries: usize,
    /// Bytes of one entry once inflated.
    pub entry_bytes: usize,
    /// Bytes of every entry together.
    pub total_bytes: usize,
}

impl Limits {
    /// 4096 entries, 256 MiB each, 512 MiB together.
    pub const DEFAULT: Limits = Limits { entries: 4096, entry_bytes: 256 << 20, total_bytes: 512 << 20 };
}

/// Reads every file of a zip archive: (name, contents).
pub fn entries(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    entries_within(data, &Limits::DEFAULT)
}

/// [`entries`] under other limits.
pub fn entries_within(data: &[u8], lim: &Limits) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16le = |o: usize| -> Result<usize, String> {
        data.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or_else(|| "truncated zip".to_string())
    };
    let u32le = |o: usize| -> Result<usize, String> {
        data.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or_else(|| "truncated zip".to_string())
    };
    // end of central directory: last occurrence of the signature
    let eocd = (0..data.len().saturating_sub(21))
        .rev()
        .find(|&i| data[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or("not a zip archive (no end of central directory)")?;
    let count = u16le(eocd + 10)?;
    let mut at = u32le(eocd + 16)?;
    if count > lim.entries {
        return Err(format!("zip archive has {count} entries (at most {} are read)", lim.entries));
    }
    let mut out = Vec::new();
    let mut left = lim.total_bytes;
    for _ in 0..count {
        if u32le(at)? != 0x0201_4b50 {
            return Err("corrupt zip central directory".into());
        }
        let method = u16le(at + 10)?;
        let csize = u32le(at + 20)?;
        let usize_ = u32le(at + 24)?;
        let (nlen, xlen, clen) = (u16le(at + 28)?, u16le(at + 30)?, u16le(at + 32)?);
        let local = u32le(at + 42)?;
        let name = String::from_utf8_lossy(data.get(at + 46..at + 46 + nlen).ok_or("truncated zip")?).to_string();
        at += 46 + nlen + xlen + clen;
        if u32le(local)? != 0x0403_4b50 {
            return Err(format!("corrupt zip entry {name}"));
        }
        let start = local + 30 + u16le(local + 26)? + u16le(local + 28)?;
        let raw = data.get(start..start + csize).ok_or("truncated zip entry")?;
        // the declared size is a hint, never the bound
        let declared = usize_.max(1) * 2 + 1024;
        let room = lim.entry_bytes.min(left);
        let over = || {
            if left < lim.entry_bytes {
                format!("zip archive holds more than {} bytes in total", lim.total_bytes)
            } else {
                format!("{name}: larger than {} bytes", lim.entry_bytes)
            }
        };
        let body = match method {
            0 if raw.len() > room => return Err(over()),
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, room.min(declared)).map_err(|e| {
                if declared > room && e.status == miniz_oxide::inflate::TINFLStatus::HasMoreOutput {
                    over()
                } else {
                    format!("{name}: {e:?}")
                }
            })?,
            m => return Err(format!("{name}: unsupported zip compression method {m}")),
        };
        left -= body.len();
        out.push((name, body));
    }
    Ok(out)
}
