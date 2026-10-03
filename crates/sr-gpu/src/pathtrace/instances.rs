use std::collections::HashMap;

use crate::three::{Draw3, MeshSrc, Scene3};
use glam::Mat4;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Key(usize, [u32; 2], bool);

pub(super) fn key(draw: &Draw3) -> Option<Key> {
    let MeshSrc::Cached(mesh) = &draw.mesh else { return None };
    let m = draw.model;
    // Shader displacement and changing vertices keep the expanded geometry path.
    if draw.maps[5].is_some()
        || mesh.count < 3
        || mesh.count % 3 != 0
        || !m.is_finite()
        || !m.inverse().is_finite()
        || !m.determinant().is_finite()
        || m.determinant() == 0.
        || m.x_axis.w != 0.
        || m.y_axis.w != 0.
        || m.z_axis.w != 0.
        || m.w_axis.w != 1.
    {
        return None;
    }
    Some(Key(
        std::sync::Arc::as_ptr(mesh) as usize,
        draw.material.uv_scale.map(f32::to_bits),
        draw.material.separate_uvs,
    ))
}

pub(super) fn repetitions(scene: &Scene3) -> HashMap<Key, usize> {
    let mut counts = HashMap::new();
    for draw in &scene.draws {
        if let Some(key) = key(draw) {
            *counts.entry(key).or_default() += 1;
        }
    }
    counts
}

pub(super) fn storage_primitives(scene: &Scene3) -> u64 {
    let counts = repetitions(scene);
    let mut seen = std::collections::HashSet::new();
    let mut total = scene.splats.iter().map(|s| u64::from(s.gpu.n)).sum::<u64>();
    for draw in &scene.draws {
        if let Some(key) = key(draw).filter(|k| counts[k] > 1) {
            total = total.saturating_add(1);
            if seen.insert(key) {
                total = total.saturating_add(draw.mesh_triangles());
            }
        } else {
            total = total.saturating_add(draw.mesh_triangles());
        }
    }
    total
}

pub(super) fn bounds(model: Mat4, lo: glam::Vec3, hi: glam::Vec3) -> [glam::Vec3; 3] {
    let (mut min, mut max) = (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN));
    for bits in 0..8 {
        let p = glam::Vec3::new(
            if bits & 1 == 0 { lo.x } else { hi.x },
            if bits & 2 == 0 { lo.y } else { hi.y },
            if bits & 4 == 0 { lo.z } else { hi.z },
        );
        let world = model.transform_point3(p);
        min = min.min(world);
        max = max.max(world);
    }
    // Enclose rounding in transformed interior vertices as well as the corners.
    let pad = min.abs().max(max.abs()).max(glam::Vec3::ONE) * (4. * f32::EPSILON);
    [min - pad, max + pad, min * 0.5 + max * 0.5]
}

pub(super) fn record(model: Mat4, prototype: usize, material: u32) -> [[f32; 4]; 24] {
    let mut rows = [[0.; 4]; 24];
    rows[0][3] = f32::from_bits(u32::MAX - 1);
    rows[1..5].copy_from_slice(&model.to_cols_array_2d());
    rows[5..9].copy_from_slice(&model.inverse().to_cols_array_2d());
    rows[9..13].copy_from_slice(&model.inverse().transpose().to_cols_array_2d());
    rows[13] = [f32::from_bits(prototype as u32), f32::from_bits(material), model.determinant(), 0.];
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    #[test]
    fn transformed_bounds_enclose_interior_points_under_reflection_and_shear() {
        let lo = Vec3::new(-5., -3., -2.);
        let hi = Vec3::new(7., 4., 6.);
        for scale in [Vec3::new(-2., 0.01, 3.), Vec3::new(1e-4, 5., -0.2)] {
            let mut model =
                Mat4::from_scale_rotation_translation(scale, Quat::from_rotation_y(0.7), Vec3::new(100., -20., 7.));
            model.y_axis += model.x_axis * 0.3;
            let bounds = bounds(model, lo, hi);
            for x in 0..=12 {
                for y in 0..=8 {
                    for z in 0..=8 {
                        let fraction = Vec3::new(x as f32 / 12., y as f32 / 8., z as f32 / 8.);
                        let p = model.transform_point3(lo + (hi - lo) * fraction);
                        assert!(p.cmpge(bounds[0]).all() && p.cmple(bounds[1]).all(), "{p:?} outside {bounds:?}");
                    }
                }
            }
        }
    }
}
