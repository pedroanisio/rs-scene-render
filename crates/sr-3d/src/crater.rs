//! Authored crater growth in object-space scene units, shared by surfaces and colliders.
//!
//! The displacement is along a fixed outward axis. A compact bowl/rim profile
//! grows in radius, depth and rim dimensions with progress; an axial envelope
//! keeps the stamp from modifying a distant back surface. This is an authored
//! deformation, not a prediction of excavation from impact energy.
use crate::Vertex;
use glam::{DMat3, DMat4, DVec3};

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub center: [f64; 3],
    /// Nonzero direction, normalized on construction.
    pub outward: [f64; 3],
    pub radius: f64,
    pub depth: f64,
    pub rim_height: f64,
    pub rim_width: f64,
    /// Axial half-width of the compact envelope. At least twice the larger of
    /// depth and rim height, ensuring an orientation-preserving deformation.
    pub influence_depth: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Crater {
    spec: Spec,
    axis: DVec3,
}

#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    pub position: [f64; 3],
    /// Differential of the position map. Normals use its inverse transpose;
    /// tangents and material displacements use it directly.
    pub jacobian: DMat3,
}

impl Crater {
    pub fn new(spec: Spec) -> Result<Self, String> {
        if spec
            .center
            .into_iter()
            .chain(spec.outward)
            .chain([spec.radius, spec.depth, spec.rim_height, spec.rim_width, spec.influence_depth])
            .any(|v| !v.is_finite())
            || spec.radius <= 0.
            || spec.depth < 0.
            || spec.rim_height < 0.
            || spec.rim_width <= 0.
            || spec.rim_width > spec.radius
            || spec.influence_depth <= 0.
            || spec.influence_depth * 0.5 < spec.depth.max(spec.rim_height)
        {
            return Err("invalid crater dimensions, envelope or center".into());
        }
        let axis = DVec3::from(spec.outward);
        let max = axis.abs().max_element();
        if max == 0. {
            return Err("crater outward direction must be nonzero".into());
        }
        // Normalize without squaring potentially enormous authored components.
        let axis = (axis / max).normalize();
        Ok(Self { spec, axis })
    }

    /// The dimensions this crater was made from; the axis is the one given, not normalised.
    pub fn spec(&self) -> Spec {
        self.spec
    }

    /// Progress is clamped to [0,1]. The map is deterministic and stateless.
    pub fn map(&self, point: [f64; 3], progress: f64) -> Result<Mapping, String> {
        if !progress.is_finite() || point.iter().any(|v| !v.is_finite()) {
            return Err("nonfinite crater point or progress".into());
        }
        let identity = Mapping { position: point, jacobian: DMat3::IDENTITY };
        let progress = progress.clamp(0., 1.);
        if progress == 0. {
            return Ok(identity);
        }
        let p = DVec3::from(point);
        let delta = p - DVec3::from(self.spec.center);
        let h = delta.dot(self.axis);
        if !delta.is_finite() || !h.is_finite() {
            return Err("crater relative position exceeds numeric range".into());
        }
        let q = h / self.spec.influence_depth;
        if q.abs() >= 1. {
            return Ok(identity);
        }
        let radial = delta - self.axis * h;
        let r = radial.x.hypot(radial.y).hypot(radial.z);
        let radius = self.spec.radius * progress;
        let width = self.spec.rim_width * progress;
        if radius == 0. || width == 0. {
            return Err("crater growth dimensions underflow".into());
        }
        if r / radius >= 1. + width / radius {
            return Ok(identity);
        }
        let (bowl, bowl_derivative) = bump(r / radius);
        let (rim, rim_derivative) = bump((r - radius) / width);
        let amount = (-self.spec.depth * bowl + self.spec.rim_height * rim) * progress;
        // Cancel progress analytically instead of dividing two tiny dimensions.
        let derivative = |height: f64, scale: f64, slope: f64| {
            if slope == 0. || height == 0. {
                0.
            } else {
                height / scale * slope
            }
        };
        let dr = -derivative(self.spec.depth, self.spec.radius, bowl_derivative)
            + derivative(self.spec.rim_height, self.spec.rim_width, rim_derivative);
        let (envelope, de) = bump(q);
        let gradient = self.axis * (amount / self.spec.influence_depth * de)
            + if r > 0. { radial / r * (dr * envelope) } else { DVec3::ZERO };
        let position = p + self.axis * (amount * envelope);
        let jacobian =
            DMat3::IDENTITY + DMat3::from_cols(self.axis * gradient.x, self.axis * gradient.y, self.axis * gradient.z);
        // det(I + axis⊗gradient) = 1 + axis·gradient. Since max|bump'|
        // is 8/(3√3), influence_depth >= 2 max(depth,rim_height) bounds
        // this determinant below by 1 - 4/(3√3), which is positive.
        let determinant = jacobian.determinant();
        if !position.is_finite() || !jacobian.is_finite() || !determinant.is_finite() || determinant <= 0. {
            return Err("crater deformation exceeds numeric range".into());
        }
        Ok(Mapping { position: position.to_array(), jacobian })
    }

    /// Allocate one output vertex array after admission. Input geometry and any
    /// caller-owned indices/caches are outside this destination-buffer budget.
    /// Missing (zero) normal/tangent attributes stay missing. Other attributes
    /// are preserved, and a failure never mutates the input.
    pub fn deform(&self, vertices: &[Vertex], progress: f64, max_bytes: usize) -> Result<Vec<Vertex>, String> {
        self.deform_in(vertices, DMat4::IDENTITY, progress, max_bytes)
    }

    /// Deform vertices in an imported primitive's coordinates, applying the
    /// crater after its basis/node transform. Output stays in primitive space,
    /// preserving the original transform and triangle winding in both renderers.
    pub fn deform_in(
        &self,
        vertices: &[Vertex],
        object_from_mesh: DMat4,
        progress: f64,
        max_bytes: usize,
    ) -> Result<Vec<Vertex>, String> {
        let bytes = vertices.len().checked_mul(std::mem::size_of::<Vertex>()).ok_or("crater buffer overflow")?;
        if bytes > max_bytes {
            return Err("crater vertex buffer exceeds memory budget".into());
        }
        if !progress.is_finite() {
            return Err("nonfinite crater progress".into());
        }
        let inverse = object_from_mesh.inverse();
        if !object_from_mesh.is_finite() || !inverse.is_finite() || object_from_mesh.row(3) != glam::DVec4::W {
            return Err("crater mesh transform must be finite, affine and invertible".into());
        }
        let linear = DMat3::from_mat4(object_from_mesh);
        let linear_inverse = DMat3::from_mat4(inverse);
        let mut out = Vec::new();
        out.try_reserve_exact(vertices.len()).map_err(|_| "cannot allocate crater vertex buffer")?;
        for v in vertices {
            if bytemuck::cast_slice::<Vertex, f32>(std::slice::from_ref(v)).iter().any(|v| !v.is_finite()) {
                return Err("nonfinite crater source vertex".into());
            }
            let point = object_from_mesh.transform_point3(DVec3::from(v.pos.map(f64::from)));
            let mapped = self.map(point.to_array(), progress)?;
            if mapped.jacobian == DMat3::IDENTITY && mapped.position == point.to_array() {
                out.push(*v);
                continue;
            }
            let jacobian = linear_inverse * mapped.jacobian * linear;
            let normal = jacobian.inverse().transpose() * DVec3::from(v.normal.map(f64::from));
            let normal = normalize_or_missing(normal)?;
            let tangent = jacobian * DVec3::new(v.tangent[0] as f64, v.tangent[1] as f64, v.tangent[2] as f64);
            let tangent = normalize_or_missing(tangent - normal * tangent.dot(normal))?;
            let vertex = Vertex {
                pos: inverse.transform_point3(DVec3::from(mapped.position)).as_vec3().to_array(),
                normal: normal.as_vec3().to_array(),
                tangent: [tangent.x as f32, tangent.y as f32, tangent.z as f32, v.tangent[3]],
                ..*v
            };
            if vertex.pos.iter().any(|v| !v.is_finite()) {
                return Err("crater position exceeds render precision".into());
            }
            out.push(vertex);
        }
        Ok(out)
    }
}

/// Compact C¹ bump and its derivative with respect to its argument.
fn bump(x: f64) -> (f64, f64) {
    if x.abs() >= 1. {
        return (0., 0.);
    }
    let a = 1. - x * x;
    (a * a, -4. * x * a)
}
fn normalize_or_missing(v: DVec3) -> Result<DVec3, String> {
    if v == DVec3::ZERO {
        return Ok(v);
    }
    v.try_normalize().ok_or_else(|| "crater direction exceeds numeric range".into())
}
