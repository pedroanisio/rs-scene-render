//! The quads of a surface as the vertices and indices a renderer reads.

use super::Quad;
use crate::{Primitive, Vertex};

/// Four vertices and six indices for each quad, with nothing shared: the mesh of a surface of quads in the coordinates of the cells
/// (a cell is a unit, the corner of the lattice is the origin; the size of a cell and the pose are the model matrix's).
///
/// * The corners run (u0, v0), (u0 + w, v0), (u0 + w, v0 + h), (u0, v0 + h), the position being the plane on the quad's axis and the
///   corner on the two others (u on the next axis, v on the one after), and `uv` is the corner's (u, v) in cells.
/// * The normal is `+-axis` and the same at all four, the tangent runs along `+u` with `w = +-1` as the facing, so that
///   `cross(normal, tangent) * w` is `+v` as `compute_tangents` makes it.
/// * The triangles are (0, 1, 2) and (0, 2, 3) facing `+axis` and (0, 3, 2) and (0, 2, 1) facing `-axis`: both wind counter-clockwise
///   seen from outside, whatever the axes are, since `u x v = +axis`.
/// * The colour is `color(class)`, the same at the four corners; the second sets of texture coordinates are zero.
pub fn expand(quads: &[Quad], color: impl Fn(u8) -> [f32; 4]) -> Primitive {
    let mut vertices = Vec::with_capacity(4 * quads.len());
    let mut indices = Vec::with_capacity(6 * quads.len());
    for q in quads {
        let axis = usize::from(q.axis);
        let (au, av) = ((axis + 1) % 3, (axis + 2) % 3);
        let sign = if q.positive { 1.0 } else { -1.0 };
        let mut normal = [0.0; 3];
        normal[axis] = sign;
        let mut tangent = [0.0; 4];
        tangent[au] = 1.0;
        tangent[3] = sign;
        let base = vertices.len() as u32;
        let (u1, v1) = (q.u0 + q.w as i32, q.v0 + q.h as i32);
        for (u, v) in [(q.u0, q.v0), (u1, q.v0), (u1, v1), (q.u0, v1)] {
            let mut pos = [0.0; 3];
            pos[axis] = q.plane as f32;
            pos[au] = u as f32;
            pos[av] = v as f32;
            vertices.push(Vertex {
                pos,
                normal,
                uv: [u as f32, v as f32],
                tangent,
                map_uv: [[0.0; 2]; 4],
                color: color(q.class),
            });
        }
        let order = if q.positive { [0, 1, 2, 0, 2, 3] } else { [0, 3, 2, 0, 2, 1] };
        indices.extend(order.into_iter().map(|i| base + i));
    }
    Primitive { vertices, indices, ..Default::default() }
}
