//! Bounded geometry reuse. Weak mesh owners keep identity keys from being
//! recycled without pinning uploaded meshes or permitting in-place mutation.
use super::*;
use crate::three::MeshGpu;
use std::collections::{HashMap, VecDeque};
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
    pub id: u64,
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
    fn of(s: &PtScene, id: u64) -> Self {
        Self {
            id,
            pos: s.pos.clone(),
            nrm: s.nrm.clone(),
            uv: s.uv.clone(),
            colors: s.colors.clone(),
            tri_mat: s.tri_mat.clone(),
            nodes: s.nodes.clone(),
            instances: s.instances.clone(),
        }
    }
    fn take(s: &mut PtScene, id: u64) -> Self {
        Self {
            id,
            pos: std::mem::take(&mut s.pos),
            nrm: std::mem::take(&mut s.nrm),
            uv: std::mem::take(&mut s.uv),
            colors: std::mem::take(&mut s.colors),
            tri_mat: std::mem::take(&mut s.tri_mat),
            nodes: std::mem::take(&mut s.nodes),
            instances: std::mem::take(&mut s.instances),
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
    bytes: usize,
}

/// Reuses rigid path-tracing geometry across camera/material/light changes and
/// object-space prototype BVHs across instance motion. Deformed or displaced
/// meshes and splats retain fresh geometry. Retained geometry has a byte budget.
/// Public builds own their returned data; the renderer moves cached geometry
/// between builds without copying it and keeps immutable GPU storage separately.
pub struct BuildCache {
    budget: usize,
    bytes: usize,
    geometry: VecDeque<(SceneKey, Geometry, usize)>,
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
        Self { budget: bytes, bytes: 0, geometry: VecDeque::new(), prototypes: HashMap::new() }
    }
    /// Builds current materials/lights and reuses eligible geometry.
    pub fn build(&mut self, scene: &Scene3) -> PtScene {
        super::build_inner(scene, self, false).0
    }
    /// Retained geometry payload, excluding allocator bookkeeping.
    pub fn resident_bytes(&self) -> usize {
        self.bytes
    }
    /// Drops all retained geometry without affecting previously returned scenes.
    pub fn clear(&mut self) {
        self.geometry.clear();
        self.prototypes.clear();
        self.bytes = 0;
    }
    /// The engine borrows geometry by moving its buffers out and returning them
    /// after recording. Public `build` continues returning independently owned data.
    pub(crate) fn build_for_render(&mut self, scene: &Scene3) -> (PtScene, Option<Ticket>) {
        super::build_inner(scene, self, true)
    }
    pub(crate) fn recycle(&mut self, mut scene: PtScene, ticket: Option<Ticket>) {
        if let Some(Ticket { key, id }) = ticket {
            let bytes = Geometry::bytes_of(&scene).saturating_add(key.bytes());
            if self.make_room(bytes) {
                self.geometry.push_back((key, Geometry::take(&mut scene, id), bytes));
                self.bytes += bytes;
            }
        }
    }
    pub(super) fn reuse(&mut self, key: Option<&SceneKey>, take: bool) -> Option<Geometry> {
        let index = self.geometry.iter().position(|(old, _, _)| key.is_some_and(|k| old.matches(k)))?;
        let (key, geometry, bytes) = self.geometry.remove(index).unwrap();
        if take {
            self.bytes -= bytes;
            Some(geometry)
        } else {
            let copy = geometry.clone();
            self.geometry.push_back((key, geometry, bytes));
            Some(copy)
        }
    }
    fn make_room(&mut self, bytes: usize) -> bool {
        if bytes > self.budget {
            return false;
        }
        while self.bytes.saturating_add(bytes) > self.budget || self.geometry.len() >= 32 {
            if let Some((_, _, bytes)) = self.geometry.pop_front() {
                self.bytes -= bytes;
            } else {
                self.bytes -= self.prototypes.values().map(|e| e.bytes).sum::<usize>();
                self.prototypes.clear();
            }
        }
        true
    }
    pub(super) fn remember(&mut self, key: Option<SceneKey>, scene: &PtScene, id: u64) {
        let Some(key) = key else { return };
        let bytes = Geometry::bytes_of(scene).saturating_add(key.bytes());
        if self.make_room(bytes) {
            self.geometry.push_back((key, Geometry::of(scene, id), bytes));
            self.bytes += bytes;
        }
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
            if self.prototypes.len() >= 64 {
                self.bytes -= self.prototypes.values().map(|e| e.bytes).sum::<usize>();
                self.prototypes.clear();
            }
            self.make_room(bytes);
            let MeshSrc::Cached(mesh) = &dr.mesh else { unreachable!() };
            self.prototypes.insert(key, Entry { value: value.clone(), owner: Arc::downgrade(mesh), bytes });
            self.bytes += bytes;
        }
        value
    }
}

/// A private identity: callers of the public packing API cannot forge cache hits.
pub(crate) struct Ticket {
    pub(super) key: SceneKey,
    pub id: u64,
}
pub(super) fn next_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::three::ThreeEngine;
    use sr_3d::{
        camera::{resolve, CameraParams},
        prim, MaterialParams,
    };
    fn fixture(draws: Vec<Draw3>) -> Scene3 {
        Scene3 {
            cam: resolve(&CameraParams::default(), 64., 64.),
            clip_fix: Mat4::IDENTITY,
            size: [64, 64],
            exposure: 1.,
            dof: None,
            lens_k1: 0.,
            draws,
            lights: Vec::new(),
            env: None,
            splats: Vec::new(),
            volumes: Vec::new(),
            encode_srgb: false,
            ao: None,
            ssr: false,
            path: None,
            geodesic: None,
        }
    }
    fn draw(engine: &ThreeEngine, at: Vec3) -> Draw3 {
        let mesh = prim::plane(24., 24., 1);
        Draw3 {
            mesh: MeshSrc::Cached(engine.upload_mesh(&mesh.vertices, &mesh.indices)),
            model: Mat4::from_translation(at),
            material: MaterialParams { unlit: true, double_sided: true, ..Default::default() },
            maps: Default::default(),
            opacity: 1.,
            cast_shadow: true,
            receive_shadow: true,
            shadow_catcher: false,
        }
    }

    #[test]
    fn render_geometry_moves_between_cache_and_scene_without_copying() {
        let gpu = crate::gpu::test_gpu().unwrap();
        let e = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
        let mut s = fixture(vec![draw(&e, Vec3::new(32., 32., 0.))]);
        let mut c = BuildCache::default();
        let (first, ticket) = c.build_for_render(&s);
        let pointers = (first.pos.as_ptr(), first.nrm.as_ptr(), first.uv.as_ptr(), first.nodes.as_ptr());
        let id = ticket.as_ref().unwrap().id;
        c.recycle(first, ticket);
        s.draws[0].material.base_color = [0.2, 0.4, 0.8, 1.];
        let (warm, ticket) = c.build_for_render(&s);
        assert!(warm.timing.geometry_reused);
        assert_eq!(id, ticket.as_ref().unwrap().id);
        assert_eq!(pointers, (warm.pos.as_ptr(), warm.nrm.as_ptr(), warm.uv.as_ptr(), warm.nodes.as_ptr()));
        let fresh = super::super::build(&s);
        assert_eq!(bytemuck::cast_slice::<_, u8>(&warm.mats), bytemuck::cast_slice::<_, u8>(&fresh.mats));
        c.recycle(warm, ticket);
        assert!(c.resident_bytes() > 0);
        c.clear();
        assert_eq!(c.resident_bytes(), 0);
        let (_, ticket) = c.build_for_render(&s);
        assert_ne!(id, ticket.unwrap().id);
    }
}
