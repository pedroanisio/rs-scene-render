//! Sorted indices are immutable. Pending encoders may be dropped or submitted
//! out of order, so only GPU-completed results can be reused by another pass.
use super::SplatGpu;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Weak,
};
struct Entry {
    owner: Weak<SplatGpu>,
    n: u32,
    view: [u32; 16],
    indices: wgpu::Buffer,
    ready: Arc<AtomicBool>,
}
pub(super) struct Cache {
    entries: VecDeque<Entry>,
    bytes: u64,
    budget: u64,
}
impl Default for Cache {
    fn default() -> Self {
        Self { entries: VecDeque::new(), bytes: 0, budget: 64 << 20 }
    }
}
impl Cache {
    pub fn get(&mut self, cloud: &Arc<SplatGpu>, view: [u32; 16]) -> Option<wgpu::Buffer> {
        let i = self.entries.iter().position(|e| {
            e.owner.as_ptr() == Arc::as_ptr(cloud)
                && e.n == cloud.n
                && e.view == view
                && e.ready.load(Ordering::Acquire)
        })?;
        let entry = self.entries.remove(i).unwrap();
        let indices = entry.indices.clone();
        self.entries.push_back(entry);
        Some(indices)
    }
    pub fn insert(
        &mut self,
        cloud: &Arc<SplatGpu>,
        view: [u32; 16],
        indices: wgpu::Buffer,
        enc: &wgpu::CommandEncoder,
    ) {
        let bytes = indices.size();
        if bytes > self.budget {
            return;
        }
        while self.entries.len() >= 32 || self.bytes.saturating_add(bytes) > self.budget {
            let Some(old) = self.entries.pop_front() else { break };
            self.bytes -= old.indices.size();
        }
        let ready = Arc::new(AtomicBool::new(false));
        let flag = ready.clone();
        enc.on_submitted_work_done(move || flag.store(true, Ordering::Release));
        self.entries.push_back(Entry { owner: Arc::downgrade(cloud), n: cloud.n, view, indices, ready });
        self.bytes += bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_splat_indices_obey_lru_and_byte_limits() {
        let gpu = crate::gpu::test_gpu().unwrap();
        let e = crate::three::ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
        let cloud = e.upload_splats(&sr_3d::Splats::default());
        let buffer = |size| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let mut c = Cache { entries: VecDeque::new(), bytes: 0, budget: 32 };
        let put = |c: &mut Cache, key, size| {
            let enc = gpu.device.create_command_encoder(&Default::default());
            c.insert(&cloud, [key; 16], buffer(size), &enc);
            gpu.queue.submit([enc.finish()]);
            gpu.wait();
        };
        put(&mut c, 0, 16);
        put(&mut c, 1, 16);
        assert!(c.get(&cloud, [0; 16]).is_some());
        put(&mut c, 2, 16);
        assert!(c.get(&cloud, [1; 16]).is_none());
        assert!(c.get(&cloud, [0; 16]).is_some());
        assert!(c.get(&cloud, [2; 16]).is_some());
        put(&mut c, 3, 64);
        assert!(c.get(&cloud, [3; 16]).is_none());
        assert_eq!(c.bytes, 32);
        assert_eq!(c.entries.len(), 2);
        c.budget = 4096;
        for i in 4..44 {
            put(&mut c, i, 16);
        }
        assert_eq!(c.entries.len(), 32);
        assert_eq!(c.bytes, 32 * 16);
    }
}
