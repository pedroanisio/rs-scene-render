//! Continuous sphere collision against translating/rotating 3D geometry.
//! Geometry is shared and immutable; callers sample a new rigid motion
//! for each authored substep. All coordinates and rotations use scene axes.

use super::{Error, Hit};
mod deforming;
use crate::physics3d::Pose3;
use rapier3d_f64::parry::{
    math::{Pose, Rotation, Vector},
    query::{self, NonlinearRigidMotion, ShapeCastOptions, ShapeCastStatus},
    shape::{Ball, SharedShape},
};
use std::sync::Arc;

/// Shared triangle geometry. The conservative charge includes construction
/// workspace and acceleration structures, as well as retained vertex/index data.
#[derive(Clone, Debug)]
pub struct Geometry {
    shape: SharedShape,
    bytes: usize,
}
impl Geometry {
    pub fn new(points: &[[f64; 3]], indices: &[[u32; 3]], max_bytes: usize) -> Result<Self, Error> {
        let bytes = points
            .len()
            .checked_mul(48)
            .and_then(|b| indices.len().checked_mul(512).and_then(|t| b.checked_add(t)))
            .and_then(|b| b.checked_add(4096))
            .ok_or(Error::Limit("collider geometry bytes"))?;
        if points.len() < 3
            || points.len() > 1_000_000
            || indices.is_empty()
            || indices.len() > 1_000_000
            || bytes > max_bytes
        {
            return Err(Error::Limit("collider geometry count or memory budget"));
        }
        if points.iter().any(|p| !super::finite(*p)) || indices.iter().flatten().any(|&i| i as usize >= points.len()) {
            return Err(Error::Invalid("collider geometry vertices or indices"));
        }
        let shape = SharedShape::trimesh(points.iter().copied().map(Vector::from_array).collect(), indices.to_vec())
            .map_err(|_| Error::Invalid("collider triangle topology"))?;
        Ok(Self { shape, bytes })
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn moving(
        &self,
        pose: Pose3,
        velocity: [f64; 3],
        angular_velocity: [f64; 3],
        epoch: f64,
    ) -> Result<Collider, Error> {
        Collider::new(self.shape.clone(), pose, velocity, angular_velocity, epoch)
    }
}

#[derive(Clone, Debug)]
pub struct Collider(Motion);
#[derive(Clone, Debug)]
enum Motion {
    Rigid(RigidCollider),
    Deforming(Arc<deforming::Surface>),
}
impl Collider {
    pub fn new(
        shape: SharedShape,
        pose: Pose3,
        velocity: [f64; 3],
        angular_velocity: [f64; 3],
        epoch: f64,
    ) -> Result<Self, Error> {
        Ok(Self(Motion::Rigid(RigidCollider::new(shape, pose, velocity, angular_velocity, epoch)?)))
    }
    /// A surface with linearly moving world-space vertices over one interval.
    /// The collider owns bounded geometry and a swept triangle BVH. Queries may
    /// use any subinterval, but may not extrapolate beyond the supplied endpoints.
    pub fn deforming(
        start: &[[f64; 3]],
        end: &[[f64; 3]],
        triangles: &[[u32; 3]],
        epoch: f64,
        duration: f64,
        max_bytes: usize,
    ) -> Result<Self, Error> {
        Ok(Self(Motion::Deforming(Arc::new(deforming::Surface::new(
            start, end, triangles, epoch, duration, max_bytes,
        )?))))
    }
    pub fn sweep(&self, time: f64, dt: f64, from: [f64; 3], to: [f64; 3], radius: f64) -> Result<Option<Hit>, Error> {
        match &self.0 {
            Motion::Rigid(r) => r.sweep(time, dt, from, to, radius),
            Motion::Deforming(d) => d.sweep(time, dt, from, to, radius),
        }
    }
}

#[derive(Clone, Debug)]
struct RigidCollider {
    shape: SharedShape,
    pose: Pose,
    velocity: Vector,
    omega: Vector,
    epoch: f64,
}
impl RigidCollider {
    /// The caller owns geometry construction/budgets. Angular velocity is in
    /// degrees/second about scene axes; `pose` is the pose at `epoch` seconds.
    pub fn new(
        shape: SharedShape,
        pose: Pose3,
        velocity: [f64; 3],
        angular_velocity: [f64; 3],
        epoch: f64,
    ) -> Result<Self, Error> {
        let velocity = Vector::from_array(velocity);
        let omega = Vector::from_array(angular_velocity.map(f64::to_radians));
        let q = pose.rot;
        let norm = q[0].hypot(q[1]).hypot(q[2]).hypot(q[3]);
        let position = Vector::from_array(pose.pos);
        let bounds = shape.compute_local_aabb();
        if !epoch.is_finite()
            || !position.is_finite()
            || !velocity.is_finite()
            || !omega.length_squared().is_finite()
            || !norm.is_finite()
            || norm == 0.
            || !bounds.mins.is_finite()
            || !bounds.maxs.is_finite()
            || !bounds.mins.cmple(bounds.maxs).all()
        {
            return Err(Error::Invalid("collider pose, velocity or bounds"));
        }
        let rotation = Rotation::from_xyzw(q[0] / norm, q[1] / norm, q[2] / norm, q[3] / norm);
        Ok(Self { shape, pose: Pose::from_parts(position, rotation), velocity, omega, epoch })
    }
    pub fn sweep(&self, time: f64, dt: f64, from: [f64; 3], to: [f64; 3], radius: f64) -> Result<Option<Hit>, Error> {
        let from = Vector::from_array(from);
        let to = Vector::from_array(to);
        if !time.is_finite()
            || !dt.is_finite()
            || dt <= 0.
            || !from.is_finite()
            || !to.is_finite()
            || !radius.is_finite()
            || radius < 0.
        {
            return Err(Error::Invalid("particle sweep interval, path or radius"));
        }
        let angular_speed = self.omega.length();
        if angular_speed * dt > std::f64::consts::FRAC_PI_2 + 1e-12 {
            return Err(Error::Limit("collider rotates more than 90 degrees per sweep; reduce fixed step"));
        }
        let delta = time - self.epoch;
        let angle = angular_speed * delta;
        if !delta.is_finite() || !angle.is_finite() {
            return Err(Error::Invalid("collider motion time overflow"));
        }
        let spin = if angular_speed > 0. {
            Rotation::from_axis_angle(self.omega / angular_speed, angle.rem_euclid(std::f64::consts::TAU))
        } else {
            Rotation::IDENTITY
        };
        let pose = Pose::from_parts(self.pose.translation + self.velocity * delta, spin * self.pose.rotation);
        let velocity = (to - from) / dt;
        if !pose.translation.is_finite()
            || !velocity.is_finite()
            || !(pose.translation + self.velocity * dt).is_finite()
        {
            return Err(Error::Invalid("collider sweep motion overflow"));
        }
        let ball = Ball::new(radius);
        let particle = Pose::from_parts(from, Rotation::IDENTITY);
        // Recover an initially penetrating particle even if it is already moving
        // outward. A shape-cast alone may deliberately ignore that overlap.
        if let Some(c) = query::contact(&pose, &*self.shape, &particle, &ball, 0.)
            .map_err(|_| Error::Invalid("unsupported particle contact geometry"))?
        {
            let surface_velocity = self.velocity + self.omega.cross(c.point1 - pose.translation);
            if c.dist < 0. || (velocity - surface_velocity).dot(c.normal1) < 0. {
                return checked(Hit {
                    fraction: 0.,
                    position: (from - c.normal1 * c.dist).to_array(),
                    normal: c.normal1.to_array(),
                    velocity: surface_velocity.to_array(),
                })
                .map(Some);
            }
        }
        let motion = NonlinearRigidMotion::new(pose, Vector::ZERO, self.velocity, self.omega);
        let hit = if angular_speed == 0. {
            query::cast_shapes(
                &pose,
                self.velocity,
                &*self.shape,
                &particle,
                velocity,
                &ball,
                ShapeCastOptions { max_time_of_impact: dt, stop_at_penetration: false, ..Default::default() },
            )
        } else {
            let particle_motion = NonlinearRigidMotion::new(particle, Vector::ZERO, velocity, Vector::ZERO);
            query::cast_shapes_nonlinear(&motion, &*self.shape, &particle_motion, &ball, 0., dt, false)
        }
        .map_err(|_| Error::Invalid("unsupported continuous particle collision geometry"))?;
        let Some(hit) = hit else {
            return Ok(None);
        };
        if matches!(hit.status, ShapeCastStatus::Failed | ShapeCastStatus::OutOfIterations) {
            return Err(Error::Invalid("continuous particle collision did not converge"));
        }
        if !hit.time_of_impact.is_finite() || !(0. ..=dt).contains(&hit.time_of_impact) {
            return Err(Error::Invalid("collision time of impact"));
        }
        let at = motion.position_at_time(hit.time_of_impact);
        let normal = at.rotation * hit.normal1;
        let point = at * hit.witness1;
        checked(Hit {
            fraction: hit.time_of_impact / dt,
            position: (point + normal * radius).to_array(),
            normal: normal.to_array(),
            velocity: (self.velocity + self.omega.cross(point - at.translation)).to_array(),
        })
        .map(Some)
    }
}
fn checked(hit: Hit) -> Result<Hit, Error> {
    if !super::finite(hit.position)
        || !super::finite(hit.velocity)
        || !super::finite(hit.normal)
        || super::length(hit.normal) == 0.
    {
        return Err(Error::Invalid("nonfinite rigid particle contact"));
    }
    Ok(hit)
}
