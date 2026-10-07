//! Foam as a coverage of the ocean surface. In `foamMode="albedo"` the foam tracers are not drawn as
//! triangles: each gives the vertices of the surface around it a share of foam, and a renderer mixes
//! the water's albedo, transmission and roughness toward the foam's by that share.

use sr_3d::Vertex;
use sr_sim::ocean::whitewater::{Kind, Particle};
use std::collections::HashMap;

/// What the coverage holds for each vertex while it is made, in bytes and as an upper bound: its 4-byte index in a bin, a table
/// entry of 48 bytes (the worst case, a bin for each vertex), 16 for the bin's vector, the share of the vertex not yet covered (8)
/// and the coverage itself (4).
pub(super) const COVERAGE_BYTES_PER_VERTEX: usize = 4 + 48 + 8 + 4 + 16;

/// Share of a tracer's life during which it covers fully; it fades linearly to nothing after.
const FULL_UNTIL: f64 = 0.6;

/// The share of the surface under `vertices` that foam covers at `time`, in 0 to 1 for each: one minus the
/// product over the foam tracers of one minus the tracer's weight there. A tracer weighs
/// `(1 - (d / radius)^2)^2` at the horizontal distance `d` from it (nothing from `radius` on), times its
/// fade: 1 until 60 % of its life, then linear to 0 at its death. Spray covers nothing. The products are
/// taken in the order of the tracers, so the same tracers give the same bits.
///
/// The work is the number of distances taken (a tracer takes one for every vertex of the 3 x 3 squares around it): it is counted before
/// any is taken, and a coverage that needs more than `max_work` is an error that says so, never a frame that took its time.
pub(super) fn coverage_within(
    vertices: &[Vertex],
    particles: &[Particle],
    time: f64,
    radius: f64,
    max_work: u64,
) -> Result<Vec<f32>, String> {
    let mut bare = vec![1.0f64; vertices.len()];
    if !(radius > 0.0 && radius.is_finite()) {
        return Ok(vec![0.0; vertices.len()]);
    }
    // the vertices by the square of side `radius` they lie in: a tracer reaches the 3 x 3 squares around its own
    let square = |x: f64, z: f64| ((x / radius).floor() as i64, (z / radius).floor() as i64);
    let mut bins: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    for (i, v) in vertices.iter().enumerate() {
        bins.entry(square(f64::from(v.pos[0]), f64::from(v.pos[2]))).or_default().push(i as u32);
    }
    let living = |p: &&Particle| p.kind == Kind::Foam && (0.0..1.0).contains(&((time - p.birth) / p.lifetime));
    let mut work = 0u64;
    for p in particles.iter().filter(living) {
        let (bx, bz) = square(p.position[0], p.position[2]);
        for dx in -1..=1 {
            for dz in -1..=1 {
                work += bins.get(&(bx + dx, bz + dz)).map_or(0, |bin| bin.len() as u64);
            }
        }
        if work > max_work {
            return Err(format!(
                "ocean foam coverage needs more than {max_work} distances (the whitewater's maxWork): a foamRadius of {radius} reaches too many vertices of the surface for the tracers"
            ));
        }
    }
    for p in particles.iter().filter(living) {
        let age = (time - p.birth) / p.lifetime;
        let fade = if age <= FULL_UNTIL { 1.0 } else { (1.0 - age) / (1.0 - FULL_UNTIL) };
        let (bx, bz) = square(p.position[0], p.position[2]);
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(bin) = bins.get(&(bx + dx, bz + dz)) else { continue };
                for &i in bin {
                    let v = &vertices[i as usize];
                    let d = ((f64::from(v.pos[0]) - p.position[0]).powi(2)
                        + (f64::from(v.pos[2]) - p.position[2]).powi(2))
                    .sqrt();
                    if d < radius {
                        let w = fade * (1.0 - (d / radius).powi(2)).powi(2);
                        bare[i as usize] *= 1.0 - w;
                    }
                }
            }
        }
    }
    Ok(bare.iter().map(|b| (1.0 - b) as f32).collect())
}

/// The coverage with no budget, for the tests of its arithmetic.
#[cfg(test)]
pub(super) fn coverage(vertices: &[Vertex], particles: &[Particle], time: f64, radius: f64) -> Vec<f32> {
    coverage_within(vertices, particles, time, radius, u64::MAX).expect("no budget to exceed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(x: f32, z: f32) -> Vertex {
        Vertex { pos: [x, 0.3, z], ..Default::default() }
    }

    fn tracer(id: u64, kind: Kind, x: f64, z: f64, birth: f64) -> Particle {
        Particle { id, kind, birth, lifetime: 3.0, position: [x, 0.0, z], velocity: [0.0; 3], radius: 0.05 }
    }

    fn grid() -> Vec<Vertex> {
        (0..9).flat_map(|i| (0..9).map(move |j| vertex(i as f32 * 0.5, j as f32 * 0.5))).collect()
    }

    #[test]
    fn no_tracers_leave_the_surface_bare() {
        assert!(coverage(&grid(), &[], 1.0, 1.0).iter().all(|c| *c == 0.0));
    }

    #[test]
    fn a_tracer_covers_its_vertex_fully_and_falls_to_nothing_at_the_radius() {
        let v = vec![vertex(0.0, 0.0), vertex(0.5, 0.0), vertex(1.0, 0.0), vertex(1.5, 0.0), vertex(0.0, 1.0)];
        let c = coverage(&v, &[tracer(1, Kind::Foam, 0.0, 0.0, 0.0)], 0.0, 1.0);
        assert_eq!(c[0], 1.0);
        assert!((c[1] - 0.5625).abs() < 1e-6, "(1 - 0.25)^2 at half the radius: {}", c[1]);
        assert_eq!(c[2], 0.0, "nothing at the radius");
        assert_eq!(c[3], 0.0);
        assert_eq!(c[4], 0.0, "the distance is horizontal: the vertex along z is at the radius too");
        // the height of the vertex or of the tracer does not count
        let mut high = tracer(2, Kind::Foam, 0.0, 0.0, 0.0);
        high.position[1] = -5.0;
        assert_eq!(coverage(&v, &[high], 0.0, 1.0)[0], 1.0);
    }

    #[test]
    fn two_tracers_combine_as_one_minus_the_product_of_what_each_leaves_bare() {
        let v = vec![vertex(0.5, 0.0)];
        let a = tracer(1, Kind::Foam, 0.0, 0.0, 0.0);
        let b = tracer(2, Kind::Foam, 1.0, 0.0, 0.0);
        let one = coverage(&v, std::slice::from_ref(&a), 0.0, 1.0)[0];
        let both = coverage(&v, &[a, b], 0.0, 1.0)[0];
        assert!((both - (1.0 - (1.0 - one) * (1.0 - one))).abs() < 1e-6, "{both} against {one}");
        assert!(both > one && both < 1.0);
    }

    #[test]
    fn a_tracer_covers_fully_for_most_of_its_life_and_fades_to_nothing_at_its_death() {
        let v = vec![vertex(0.0, 0.0)];
        let t = tracer(1, Kind::Foam, 0.0, 0.0, 0.0);
        let at = |time: f64| coverage(&v, std::slice::from_ref(&t), time, 1.0)[0];
        assert_eq!(at(0.0), 1.0);
        assert_eq!(at(1.8), 1.0, "60 % of 3 s");
        assert!((at(2.4) - 0.5).abs() < 1e-6, "halfway through the fade: {}", at(2.4));
        assert_eq!(at(3.0), 0.0);
        assert_eq!(at(10.0), 0.0);
        assert_eq!(at(-1.0), 0.0, "not born yet");
    }

    #[test]
    fn spray_covers_nothing() {
        let v = vec![vertex(0.0, 0.0)];
        assert_eq!(coverage(&v, &[tracer(1, Kind::Spray, 0.0, 0.0, 0.0)], 0.0, 1.0)[0], 0.0);
    }

    #[test]
    fn the_search_by_bins_gives_the_bits_of_asking_every_tracer_of_every_vertex() {
        // pseudo-random tracers over and beyond the grid, in the order of their ids
        let mut state = 0x9e3779b97f4a7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let particles: Vec<Particle> = (0..40)
            .map(|i| {
                let kind = if i % 5 == 0 { Kind::Spray } else { Kind::Foam };
                tracer(i, kind, next() * 6.0 - 1.0, next() * 6.0 - 1.0, next() * 2.0)
            })
            .collect();
        let v = grid();
        let (radius, time) = (0.6, 2.5);
        let fast = coverage(&v, &particles, time, radius);
        // the definition, tracer by tracer, vertex by vertex
        let mut bare = vec![1.0f64; v.len()];
        for p in particles.iter().filter(|p| p.kind == Kind::Foam) {
            let age = (time - p.birth) / p.lifetime;
            if !(0.0..1.0).contains(&age) {
                continue;
            }
            let fade = if age <= FULL_UNTIL { 1.0 } else { (1.0 - age) / (1.0 - FULL_UNTIL) };
            for (i, vertex) in v.iter().enumerate() {
                let d = ((f64::from(vertex.pos[0]) - p.position[0]).powi(2)
                    + (f64::from(vertex.pos[2]) - p.position[2]).powi(2))
                .sqrt();
                if d < radius {
                    let w = fade * (1.0 - (d / radius).powi(2)).powi(2);
                    bare[i] *= 1.0 - w;
                }
            }
        }
        let want: Vec<f32> = bare.iter().map(|b| (1.0 - b) as f32).collect();
        assert!(want.iter().any(|c| *c > 0.0) && want.contains(&0.0), "the case covers part of the grid");
        assert_eq!(fast, want);
        assert_eq!(coverage(&v, &particles, time, radius), fast, "the same tracers give the same bits");
    }

    #[test]
    fn a_coverage_that_takes_more_distances_than_its_budget_is_refused_and_one_at_the_budget_is_made() {
        // a grid of 20 x 20 vertices, 10 foam tracers each within reach of all of them (a radius of 100 over a square of 20)
        let vertices: Vec<Vertex> = (0..400).map(|i| vertex((i % 20) as f32, (i / 20) as f32)).collect();
        let tracers: Vec<Particle> = (0..10)
            .map(|i| Particle {
                id: i,
                kind: Kind::Foam,
                birth: 0.0,
                lifetime: 10.0,
                position: [i as f64, 0.0, i as f64],
                velocity: [0.0; 3],
                radius: 0.05,
            })
            .collect();
        // radius 100: one bin holds every vertex, and every tracer reaches that bin: 400 distances each
        let needed = 10 * 400;
        let made = coverage_within(&vertices, &tracers, 1.0, 100.0, needed).expect("the budget is the work needed");
        assert_eq!(made.len(), 400);
        let refused = coverage_within(&vertices, &tracers, 1.0, 100.0, needed - 1).unwrap_err();
        assert!(refused.contains("foam coverage") && refused.contains("maxWork"), "{refused}");
        assert!(refused.contains("more than 3999"), "the budget it was over is named: {refused}");
        // a tracer that is not alive weighs nothing and costs nothing
        assert!(coverage_within(&vertices, &tracers, 11.0, 100.0, 0).is_ok());
    }
}
