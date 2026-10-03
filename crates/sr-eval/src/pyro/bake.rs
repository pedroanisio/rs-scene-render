use sr_volume::bake::{BakeLimits, BakeReceipt, BakeWriter};
use std::path::Path;

impl crate::Evaluator {
    /// Freezes an instantiated native pyro object at project frames [first, end).
    /// Samples composition time, preserving all parent clocks, source retiming and
    /// physics coupling. Absent nodes produce transparent entries. Playback is a
    /// frozen render asset: source edits require a new bake, not solver resumption.
    pub fn bake_pyro(
        &self,
        id: &str,
        directory: &Path,
        first: u64,
        end: u64,
        limits: BakeLimits,
    ) -> Result<BakeReceipt, String> {
        self.bake_pyro_with_progress(id, directory, first, end, limits, |_, _| {})
    }
    /// The progress callback receives (completed samples, total samples), after
    /// each successful append. It never reports completion before the write.
    pub fn bake_pyro_with_progress(
        &self,
        id: &str,
        directory: &Path,
        first: u64,
        end: u64,
        limits: BakeLimits,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<BakeReceipt, String> {
        let count = end
            .checked_sub(first)
            .filter(|n| *n > 0 && *n <= u64::from(limits.max_frames.min(100_000)))
            .ok_or("bake requires an ordered range of 1..100000 frames within its frame budget")?;
        if end > self.frame_count() {
            return Err("bake frame range exceeds project duration".into());
        }
        let node =
            self.program().nodes.iter().find(|n| &*n.id == id).ok_or_else(|| format!("pyro object {id} not found"))?;
        let native = matches!(&*node.elem,sr_model::model::Node::Object3D(o) if o.children.iter().any(|c|matches!(c,sr_model::model::Object3DChild::Pyro(_))));
        if !native {
            return Err(format!("{id} does not own native pyro"));
        }
        let fps = self.program().fps;
        let mut writer =
            BakeWriter::new(directory, fps.frame_time(first), fps.as_f64(), limits).map_err(|e| e.to_string())?;
        for index in first..first + count {
            let frame = self.evaluate_frame(index);
            if !frame.problems.is_empty() {
                return Err(format!("frame {index}: {}", frame.problems.join("; ")));
            }
            let volume = match frame.nodes.iter().find(|n| &*n.id == id) {
                Some(node) => Some(
                    &*node
                        .sim_volume
                        .as_ref()
                        .ok_or_else(|| format!("frame {index}: {id} has no simulated volume"))?
                        .data,
                ),
                None => None,
            };
            writer.push(volume).map_err(|e| format!("frame {index}: {e}"))?;
            progress(index - first + 1, count);
        }
        writer.finish().map_err(|e| e.to_string())
    }
}
