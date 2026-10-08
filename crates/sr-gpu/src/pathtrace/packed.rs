//! Immutable storage, safe even when several encoders are submitted in a different order.
use super::PtScene;
use std::collections::VecDeque;
#[derive(Clone)]
pub(super) struct Buffers {
    pub verts: wgpu::Buffer,
    pub nodes: wgpu::Buffer,
    pub offset: u32,
}
struct Entry {
    id: u64,
    pixels: Vec<u32>,
    buffers: Buffers,
    bytes: u64,
}
pub(super) struct Cache {
    entries: VecDeque<Entry>,
    bytes: u64,
    budget: u64,
}
impl Default for Cache {
    fn default() -> Self {
        Self { entries: VecDeque::new(), bytes: 0, budget: 256 << 20 }
    }
}
impl Cache {
    pub fn get(&mut self, id: u64, pixels: &[u32]) -> Option<Buffers> {
        let i = self.entries.iter().position(|e| e.id == id && e.pixels == pixels)?;
        let entry = self.entries.remove(i).unwrap();
        let result = entry.buffers.clone();
        self.entries.push_back(entry);
        Some(result)
    }
    pub fn insert(&mut self, id: u64, pixels: &[u32], buffers: Buffers) {
        let bytes = buffers.verts.size().saturating_add(buffers.nodes.size()).saturating_add(pixels.len() as u64 * 4);
        if bytes > self.budget {
            return;
        }
        while self.entries.len() >= 32 || self.bytes.saturating_add(bytes) > self.budget {
            let Some(entry) = self.entries.pop_front() else { break };
            self.bytes -= entry.bytes;
        }
        self.entries.push_back(Entry { id, pixels: pixels.to_vec(), buffers, bytes });
        self.bytes += bytes;
    }
}
pub(super) fn vertices(data: &PtScene) -> (Vec<[f32; 4]>, u32) {
    // interleaved corners, the material index's bits in the first corner's position w
    let mut verts: Vec<[f32; 4]> = Vec::with_capacity(data.pos.len() * 8);
    for (c, (p, n)) in data.pos.iter().zip(&data.nrm).enumerate() {
        let w = if c % 3 == 0 { f32::from_bits(data.tri_mat[c / 3]) } else { 0.0 };
        verts.push([p[0], p[1], p[2], w]);
        verts.push(*n);
        verts.push(data.colors.get(c).copied().unwrap_or([1.0; 4]));
        let uv = data.uv.get(c).copied().unwrap_or([[0.0; 2]; 6]);
        for pair in uv.as_chunks::<2>().0 {
            verts.push([pair[0][0], pair[0][1], pair[1][0], pair[1][1]]);
        }
        if c % 3 == 2 {
            verts.extend([[0.0; 4]; 6]);
        }
    }
    for (&index, record) in &data.splats {
        verts[index * 24..(index + 1) * 24].copy_from_slice(record);
    }
    for (&index, record) in &data.instances {
        verts[index * 24..(index + 1) * 24].copy_from_slice(record);
    }
    let offset = (verts.len() * 4) as u32;
    for pixels in data.pixels.chunks(4) {
        verts.push(std::array::from_fn(|i| f32::from_bits(pixels.get(i).copied().unwrap_or(0))));
    }
    (verts, offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_cache_checks_pixels_and_evicts_within_its_byte_budget() {
        let gpu = crate::gpu::test_gpu().unwrap();
        let buffer = || {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let buffers = || Buffers { verts: buffer(), nodes: buffer(), offset: 0 };
        let mut c = Cache { entries: VecDeque::new(), bytes: 0, budget: 72 };
        c.insert(1, &[0], buffers());
        c.insert(2, &[0], buffers());
        assert!(c.get(1, &[1]).is_none());
        assert!(c.get(1, &[0]).is_some());
        c.insert(3, &[0], buffers());
        assert!(c.get(2, &[0]).is_none());
        assert!(c.get(1, &[0]).is_some());
        assert!(c.get(3, &[0]).is_some());
        assert_eq!(c.bytes, 72);
        c.insert(4, &[0; 32], buffers());
        assert!(c.get(4, &[0; 32]).is_none());
        assert_eq!(c.bytes, 72);
    }
}
