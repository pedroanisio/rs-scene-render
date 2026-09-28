//! WAV output: 16- and 24-bit PCM with TPDF dither, or 32-bit float,
//! WAVE_FORMAT_EXTENSIBLE with a channel mask beyond stereo.

use std::io::Write;
use std::path::Path;

use crate::dsp::Rng;
use crate::layout::Layout;
use crate::loudness::Planar;

/// Encodes interleaved PCM bytes (TPDF-dithered for integer depths when `dither`).
pub fn pcm_bytes(buf: &Planar, bits: u16, dither: bool, seed: u64) -> Vec<u8> {
    let n = buf.first().map(Vec::len).unwrap_or(0);
    let mut out = Vec::with_capacity(n * buf.len() * bits as usize / 8);
    let mut rng = Rng::new(seed);
    let scale = match bits {
        16 => 32767.0,
        24 => 8_388_607.0,
        _ => 0.0,
    };
    for i in 0..n {
        for c in buf {
            let v = c[i] as f64;
            match bits {
                32 => out.extend_from_slice(&(v as f32).to_le_bytes()),
                _ => {
                    let d = if dither { rng.uniform() - rng.uniform() } else { 0.0 };
                    let q = (v * scale + d).round().clamp(-scale - 1.0, scale) as i32;
                    if bits == 16 {
                        out.extend_from_slice(&(q as i16).to_le_bytes());
                    } else {
                        out.extend_from_slice(&q.to_le_bytes()[..3]);
                    }
                }
            }
        }
    }
    out
}

/// Writes a WAV file.
pub fn write(path: &Path, buf: &Planar, rate: u32, bits: u16, layout: Layout, dither: bool) -> std::io::Result<()> {
    let chans = buf.len() as u16;
    let data = pcm_bytes(buf, bits, dither, 0x5d17);
    let float = bits == 32;
    let extensible = chans > 2 || bits > 16;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let fmt_len: u32 = if extensible { 40 } else { 16 };
    let block = chans * bits / 8;
    f.write_all(b"RIFF")?;
    f.write_all(&(4 + 8 + fmt_len + 8 + data.len() as u32).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&fmt_len.to_le_bytes())?;
    let tag: u16 = if extensible {
        0xFFFE
    } else if float {
        3
    } else {
        1
    };
    f.write_all(&tag.to_le_bytes())?;
    f.write_all(&chans.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * block as u32).to_le_bytes())?;
    f.write_all(&block.to_le_bytes())?;
    f.write_all(&bits.to_le_bytes())?;
    if extensible {
        f.write_all(&22u16.to_le_bytes())?;
        f.write_all(&bits.to_le_bytes())?;
        f.write_all(&layout.wav_mask().to_le_bytes())?;
        let sub: u16 = if float { 3 } else { 1 };
        f.write_all(&sub.to_le_bytes())?;
        f.write_all(&[0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71])?;
    }
    f.write_all(b"data")?;
    f.write_all(&(data.len() as u32).to_le_bytes())?;
    f.write_all(&data)?;
    f.flush()
}
