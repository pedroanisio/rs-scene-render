//! Bounded geometry reuse. Weak mesh owners keep identity keys from being
//! recycled without pinning uploaded meshes or permitting in-place mutation.
use super::*;
use crate::three::MeshGpu;
use std::collections::HashMap;
use std::sync::{Arc, Weak};

type DrawKey = (instances::Key, [u32; 16], bool);
pub(super) struct SceneKey {
    draws: Vec<DrawKey>,
    owners: Vec<Weak<MeshGpu>>,
}
impl SceneKey {
    pub(super) fn of(scene: &Scene3) -> Option<Self> {
        // Splats, deformed vertices and displacement use their original fresh
        // path. Rigid prototype reuse still applies to other draws in the pass.
        if !scene.splats.is_empty() {
            return None;
        }
        let mut key = Self { draws: Vec::new(), owners: Vec::new() };
        // Include skipped shadow catchers: the instance classifier counts their
        // identities too, so adding one can change a visible mesh's BVH layout.
        for dr in &scene.draws {
            let mesh_key = instances::key(dr)?;
            let MeshSrc::Cached(mesh) = &dr.mesh else { return None };
            key.draws.push((mesh_key, dr.model.to_cols_array().map(f32::to_bits), dr.shadow_catcher));
            key.owners.push(Arc::downgrade(mesh));
        }
        Some(key)
    }
    fn matches(&self, other: &Self) -> bool {
        self.draws == other.draws && self.owners.iter().all(|w| w.strong_count() > 0)
    }
    fn bytes(&self) -> usize {
        self.draws.len() * std::mem::size_of::<DrawKey>() + self.owners.len() * std::mem::size_of::<Weak<MeshGpu>>()
    }
}

#[derive(Clone)]
pub(super) struct Geometry {
    pos: Vec<[f32; 4]>,
    nrm: Vec<[f32; 4]>,
    uv: Vec<[[f32; 2]; 6]>,
    colors: Vec<[f32; 4]>,
    tri_mat: Vec<u32>,
    nodes: Vec<PtNode>,
    instances: HashMap<usize, [[f32; 4]; 24]>,
}
impl Geometry {
    fn bytes_of(s: &PtScene) -> usize {
        s.pos.len() * 16
            + s.nrm.len() * 16
            + s.uv.len() * 48
            + s.colors.len() * 16
            + s.tri_mat.len() * 4
            + s.nodes.len() * std::mem::size_of::<PtNode>()
            + s.instances.len() * (std::mem::size_of::<(usize, [[f32; 4]; 24])>() + 32)
    }
    fn of(s: &PtScene) -> Self {
        Self {
            pos: s.pos.clone(),
            nrm: s.nrm.clone(),
            uv: s.uv.clone(),
            colors: s.colors.clone(),
            tri_mat: s.tri_mat.clone(),
            nodes: s.nodes.clone(),
            instances: s.instances.clone(),
        }
    }
    pub(super) fn apply(self, s: &mut PtScene) {
        s.pos = self.pos;
        s.nrm = self.nrm;
        s.uv = self.uv;
        s.colors = self.colors;
        s.tri_mat = self.tri_mat;
        s.nodes = self.nodes;
        s.instances = self.instances;
    }
}

pub(super) struct Prototype {
    pub triangles: Vec<Triangle>,
    pub nodes: Vec<PtNode>,
    pub order: Vec<usize>,
}
struct Entry {
    value: Arc<Prototype>,
    owner: Weak<MeshGpu>,
}

/// Reuses rigid path-tracing geometry across camera/material/light changes and
/// object-space prototype BVHs across instance motion. Deformed or displaced
/// meshes and splats retain fresh geometry. Retained geometry has a byte budget.
/// GPU buffers remain per-pass, so several passes can be recorded before submit.
pub struct BuildCache {
    budget: usize,
    bytes: usize,
    geometry: Option<(SceneKey, Geometry, usize)>,
    prototypes: HashMap<instances::Key, Entry>,
}
impl Default for BuildCache {
    fn default() -> Self {
        Self::new(256 << 20)
    }
}
impl BuildCache {
    /// A cache with at most `bytes` of retained geometry payload; zero disables reuse.
    pub fn new(bytes: usize) -> Self {
        Self { budget: bytes, bytes: 0, geometry: None, prototypes: HashMap::new() }
    }
    /// Builds current materials/lights and reuses eligible geometry.
    pub fn build(&mut self, scene: &Scene3) -> PtScene {
        super::build_inner(scene, self)
    }
    /// Retained geometry payload, excluding allocator bookkeeping.
    pub fn resident_bytes(&self) -> usize {
        self.bytes
    }
    /// Drops all retained geometry without affecting previously returned scenes.
    pub fn clear(&mut self) {
        self.geometry = None;
        self.prototypes.clear();
        self.bytes = 0;
    }
    pub(super) fn reuse(&mut self, key: Option<&SceneKey>) -> Option<Geometry> {
        if let Some((old, geometry, _)) = &self.geometry {
            if key.is_some_and(|k| old.matches(k)) {
                return Some(geometry.clone());
            }
        }
        if let Some((_, _, bytes)) = self.geometry.take() {
            self.bytes -= bytes;
        }
        None
    }
    pub(super) fn remember(&mut self, key: Option<SceneKey>, scene: &PtScene) {
        let Some(key) = key else { return };
        let bytes = Geometry::bytes_of(scene).saturating_add(key.bytes());
        if bytes > self.budget {
            return;
        }
        if self.bytes.saturating_add(bytes) > self.budget {
            self.clear();
        }
        self.geometry = Some((key, Geometry::of(scene), bytes));
        self.bytes += bytes;
    }
    pub(super) fn prototype(&mut self, dr: &Draw3, timing: &mut BuildTiming) -> Arc<Prototype> {
        let key = instances::key(dr).expect("eligible rigid prototype");
        if let Some(entry) = self.prototypes.get(&key).filter(|e| e.owner.strong_count() > 0) {
            timing.prototype_hits += 1;
            return entry.value.clone();
        }
        let triangles: Vec<_> = dr
            .mesh
            .cpu()
            .1
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| triangle(dr, t, Mat4::IDENTITY, Mat4::IDENTITY))
            .collect();
        let positions: Vec<_> = triangles.iter().map(|t| t.p).collect();
        let mut nodes = Vec::new();
        let clock = std::time::Instant::now();
        let order = bvh(&positions, &mut nodes);
        timing.bvh_seconds += clock.elapsed().as_secs_f64();
        timing.prototype_builds += 1;
        let value = Arc::new(Prototype { triangles, nodes, order });
        let bytes = value.triangles.len() * std::mem::size_of::<Triangle>()
            + value.nodes.len() * std::mem::size_of::<PtNode>()
            + value.order.len() * std::mem::size_of::<usize>();
        if bytes <= self.budget && self.budget > 0 {
            if self.prototypes.len() >= 64 || self.bytes.saturating_add(bytes) > self.budget {
                self.clear();
            }
            let MeshSrc::Cached(mesh) = &dr.mesh else { unreachable!() };
            self.prototypes.insert(key, Entry { value: value.clone(), owner: Arc::downgrade(mesh) });
            self.bytes += bytes;
        }
        value
    }
}
