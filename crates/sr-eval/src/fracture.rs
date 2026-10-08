//! Frozen fracture surfaces and world-space fragment poses for rendering.
pub(crate) mod source;

use crate::{FrameNode, Program};
use sr_3d::fracture::{self, Surface};
use sr_sim::physics3d::{Pose3, Shape3};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

#[derive(Debug)]
pub struct Piece {
    pub surfaces: Vec<Surface>,
    pub mass: f64,
    pub offset: [f64; 3],
    pub(crate) shape: Shape3,
}
#[derive(Debug)]
pub struct Geometry {
    pub key: u64,
    pub pieces: Vec<Piece>,
    pub interior_material: String,
    /// Imported materials and texture payloads retained for exterior batches.
    pub model: Option<Arc<sr_3d::Model>>,
    /// What freezing the source reported without failing (an unknown `animationClipTo`, SREP 42).
    pub notes: Vec<String>,
}
#[derive(Debug)]
pub struct SimFracture {
    pub key: u64,
    pub geometry: Arc<Geometry>,
    pub poses: Vec<Pose3>,
    pub enabled: Vec<bool>,
}

pub(crate) fn prepare(
    p: &Program,
    n: &FrameNode,
    scale: [f64; 3],
    config: &sr_model::model::Fracture,
    mass: f64,
) -> Result<Arc<Geometry>, String> {
    let value = |key, default| crate::sim::num(config, key, default);
    let budget = (value("maxMemoryMiB", 256.) as usize).checked_mul(1 << 20).ok_or("fracture memory overflow")?;
    let mut notes = Vec::new();
    let (sources, model) = source::load(p, n, scale, budget, &mut notes)?;
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut used = model.as_ref().map_or(0, |m| sr_3d::sequence::bytes(m));
    for source in &sources {
        let base = u32::try_from(vertices.len()).map_err(|_| "fracture vertex count")?;
        used = used
            .saturating_add(source.vertices.len().saturating_mul(512))
            .saturating_add(source.indices.len().saturating_mul(16));
        if used >= budget {
            return Err("fracture source memory allowance".into());
        }
        vertices.extend(source.vertices.iter().map(|v| v.pos.map(f64::from)));
        for t in source.indices.as_chunks::<3>().0 {
            triangles.push([base + t[0], base + t[1], base + t[2]]);
        }
    }
    let pieces = fracture::fracture(
        &vertices,
        &triangles,
        fracture::Spec {
            pieces: value("pieces", 8.) as usize,
            seed: config.seed,
            mass,
            max_bytes: budget - used,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    // Account for all partitions together before rebuilding render surfaces.
    for piece in &pieces {
        used = used
            .saturating_add(piece.vertices.len().saturating_mul(48))
            .saturating_add(piece.faces.len().saturating_mul(64));
    }
    if used >= budget {
        return Err("fracture partition memory allowance".into());
    }
    let mut output = Vec::new();
    for piece in &pieces {
        let surfaces = fracture::surface(piece, &sources, value("interiorUvScale", 1.), budget - used)
            .map_err(|e| e.to_string())?;
        for surface in &surfaces {
            used = used
                .saturating_add(surface.mesh.vertices.len().saturating_mul(256))
                .saturating_add(surface.mesh.indices.len().saturating_mul(8));
        }
        if used >= budget {
            return Err("fracture surfaces memory allowance".into());
        }
        output.push(Piece {
            surfaces,
            mass: piece.mass,
            offset: piece.center,
            shape: Shape3::Decomposition(piece.vertices.clone(), piece.faces.iter().map(|f| f.indices).collect()),
        });
    }
    static NEXT: AtomicU64 = AtomicU64::new(1);
    Ok(Arc::new(Geometry {
        key: NEXT.fetch_add(1, Ordering::Relaxed),
        pieces: output,
        // a fracture of a mesh has one (FRX3 refuses it without); a fracture of cells has none (FRX8) and is not made here
        interior_material: config.interior_material.clone().unwrap_or_default(),
        model,
        notes,
    }))
}
