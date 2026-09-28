//! A minimal zip reader for dotLottie containers (stored and deflated entries).

/// Reads every file of a zip archive: (name, contents).
pub fn entries(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
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
    let mut out = Vec::new();
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
        let body = match method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, usize_.max(1) * 2 + 1024)
                .map_err(|e| format!("{name}: {e:?}"))?,
            m => return Err(format!("{name}: unsupported zip compression method {m}")),
        };
        out.push((name, body));
    }
    Ok(out)
}
