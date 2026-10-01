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
    let data = pcm_bytes(buf, bits, dither, 0x5d17);
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(&header(data.len() as u64, buf.len() as u16, rate, bits, layout, false))?;
    f.write_all(&data)?;
    f.flush()
}

/// Everything before the samples of a file with `data` bytes of them: RIFF, or RF64 (EBU Tech 3306) when
/// the sizes do not fit 32 bits or `rf64` asks for it.
fn header(data: u64, chans: u16, rate: u32, bits: u16, layout: Layout, rf64: bool) -> Vec<u8> {
    let float = bits == 32;
    let extensible = chans > 2 || bits > 16;
    let fmt_len: u32 = if extensible { 40 } else { 16 };
    let block = chans * bits / 8;
    let riff = 4 + 8 + fmt_len as u64 + 8 + data;
    let rf64 = rf64 || riff > u32::MAX as u64;
    let mut f = Vec::new();
    if rf64 {
        f.extend_from_slice(b"RF64\xFF\xFF\xFF\xFFWAVEds64");
        f.extend_from_slice(&28u32.to_le_bytes());
        f.extend_from_slice(&(riff + 36).to_le_bytes());
        f.extend_from_slice(&data.to_le_bytes());
        f.extend_from_slice(&(data / block.max(1) as u64).to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
    } else {
        f.extend_from_slice(b"RIFF");
        f.extend_from_slice(&(riff as u32).to_le_bytes());
        f.extend_from_slice(b"WAVE");
    }
    f.extend_from_slice(b"fmt ");
    f.extend_from_slice(&fmt_len.to_le_bytes());
    let tag: u16 = if extensible {
        0xFFFE
    } else if float {
        3
    } else {
        1
    };
    f.extend_from_slice(&tag.to_le_bytes());
    f.extend_from_slice(&chans.to_le_bytes());
    f.extend_from_slice(&rate.to_le_bytes());
    f.extend_from_slice(&(rate * block as u32).to_le_bytes());
    f.extend_from_slice(&block.to_le_bytes());
    f.extend_from_slice(&bits.to_le_bytes());
    if extensible {
        f.extend_from_slice(&22u16.to_le_bytes());
        f.extend_from_slice(&bits.to_le_bytes());
        f.extend_from_slice(&layout.wav_mask().to_le_bytes());
        let sub: u16 = if float { 3 } else { 1 };
        f.extend_from_slice(&sub.to_le_bytes());
        f.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]);
    }
    f.extend_from_slice(b"data");
    f.extend_from_slice(&(if rf64 { u32::MAX } else { data as u32 }).to_le_bytes());
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u64_at(b: &[u8], o: usize) -> u64 {
        u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
    }

    #[test]
    fn small_files_keep_the_riff_header() {
        let h = header(4800 * 12, 6, 48000, 16, Layout::Surround51, false);
        assert_eq!(&h[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()) as usize, h.len() - 8 + 4800 * 12);
        assert_eq!(&h[h.len() - 8..h.len() - 4], b"data");
        assert_eq!(u32::from_le_bytes(h[h.len() - 4..].try_into().unwrap()), 4800 * 12);
    }

    #[test]
    fn data_beyond_4_gib_is_written_as_rf64() {
        // 7.1 float at 48 kHz for 50 minutes
        let frames = 48000u64 * 60 * 50;
        let data = frames * 8 * 4;
        assert!(data > u32::MAX as u64);
        let h = header(data, 8, 48000, 32, Layout::Surround71, false);
        assert_eq!(&h[..4], b"RF64");
        assert_eq!(&h[4..8], &[0xFF; 4]);
        assert_eq!(&h[8..16], b"WAVEds64");
        assert_eq!(u32::from_le_bytes(h[16..20].try_into().unwrap()), 28);
        assert_eq!(u64_at(&h, 20), h.len() as u64 - 8 + data, "RIFF size");
        assert_eq!(u64_at(&h, 28), data, "data size");
        assert_eq!(u64_at(&h, 36), frames, "sample frames");
        assert_eq!(&h[48..52], b"fmt ");
        assert_eq!(&h[h.len() - 8..], b"data\xFF\xFF\xFF\xFF");
        // the largest RIFF file stays RIFF
        let fits = u32::MAX as u64 - 60;
        assert_eq!(&header(fits, 8, 48000, 32, Layout::Surround71, false)[..4], b"RIFF");
        assert_eq!(&header(fits + 8, 8, 48000, 32, Layout::Surround71, false)[..4], b"RF64");
    }

    #[test]
    fn ffmpeg_reads_rf64() {
        let dir = std::env::temp_dir().join(format!("sr-audio-rf64-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.wav");
        let buf: Planar = vec![vec![0.25; 4800]; 6];
        let data = pcm_bytes(&buf, 32, false, 0);
        let mut bytes = header(data.len() as u64, 6, 48000, 32, Layout::Surround51, true);
        assert_eq!(&bytes[..4], b"RF64");
        bytes.extend_from_slice(&data);
        std::fs::write(&path, bytes).unwrap();
        let probe = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=channels,duration", "-of", "csv=p=0"])
            .arg(&path)
            .output();
        let Ok(out) = probe else { return };
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success() && text.trim().starts_with("6,0.1"), "{text}");
    }
}
