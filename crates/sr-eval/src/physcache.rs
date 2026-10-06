//! What a baked physics cache carries beyond poses: the body velocities, the contacts the
//! rigid bodies resolved, and the identity of the document's physics.
//!
//! The identity is a SHA-256 over everything that determines the simulation: the 2D and 3D
//! world definitions (bodies, shapes, joints, gravity, step, fractures) and, for every step
//! of the baked span, what the document feeds the world (kinematic poses, visibility,
//! force fields, deforming-surface revisions). A cache whose identity differs from the
//! document's is an error: it was baked from something else, and simulating in its place
//! would hide that.

use std::fmt::{Debug, Write};

use sha2::{Digest, Sha256};
use sr_sim::physics3d::{
    Body3Spec, BodyKind, Contact3, ContactLogConfig, Fracture3, ImpactWatch, Shape3, Velocity3, World3Spec,
};

/// First eight bytes of a cache that carries velocities, contacts and an identity.
pub(crate) const MAGIC: &[u8; 8] = b"SRPHYS04";

/// Bytes of one stored contact: step, two body indices, point, normal, impulse and
/// relative velocity.
pub(crate) const CONTACT_RECORD: usize = 96;

/// Contact points one step may produce before recording fails.
const CONTACTS_PER_STEP: usize = 1024;

/// Bytes of memory the recorded contacts may use before recording fails.
const CONTACT_LOG_BYTES: usize = 64 << 20;

/// Pairs whose contact points together do not exceed this multiple of the weight impulse of
/// all dynamic bodies in one step are not recorded: a body at rest or sliding pushes with about its weight, an
/// impact with much more.
const RESTING_IMPULSE_FACTOR: f64 = 2.0;

/// The velocities and contacts of the 3D rigid bodies over the document's physics.
#[derive(Clone, Debug, PartialEq)]
pub struct PhysicsTrace {
    pub start: f64,
    pub step: f64,
    /// Steps held, the first at `start`.
    pub frames: u64,
    /// Contacts in step order, as `sr_sim::physics3d::Contact3` describes them. Empty for a
    /// cache baked before contacts were recorded.
    pub contacts: Vec<Contact3>,
    /// Velocities of every 3D body at every step; empty for a cache baked before they were
    /// recorded.
    pub velocities: Vec<Vec<Velocity3>>,
}

/// The impulse a body of `mass` pushes with in one step while it rests: its weight in one
/// step, in scene units, times the factor above. An impact has to push harder.
pub(crate) fn rest_threshold(mass: f64, gravity: [f64; 3], pixels_per_meter: f64, step: f64) -> f64 {
    let g = gravity.iter().map(|c| c * c).sum::<f64>().sqrt();
    RESTING_IMPULSE_FACTOR * mass * g * pixels_per_meter * step
}

/// What recording asks of the world for `spec`: the threshold for all dynamic bodies, or for
/// the lightest watched source if that is lower, so that every watched impact is in the record.
pub(crate) fn contact_config(spec: &World3Spec, watches: &[ImpactWatch]) -> ContactLogConfig {
    let mass: f64 = spec.bodies.iter().filter(|b| b.kind == BodyKind::Dynamic).map(|b| b.mass).sum();
    let all = rest_threshold(mass, spec.gravity, spec.pixels_per_meter, spec.step);
    ContactLogConfig {
        min_impulse: watches.iter().map(|w| w.min_impulse).fold(all, f64::min),
        ..ContactLogConfig::new(CONTACTS_PER_STEP, CONTACT_LOG_BYTES)
    }
}

pub(crate) fn write_contacts(out: &mut Vec<u8>, contacts: &[Contact3]) {
    let index = |b: Option<usize>| b.map_or(-1, |i| i as i32);
    for c in contacts {
        out.extend_from_slice(&c.step.to_le_bytes());
        out.extend_from_slice(&index(c.bodies[0]).to_le_bytes());
        out.extend_from_slice(&index(c.bodies[1]).to_le_bytes());
        for v in c.point.iter().chain(&c.normal).chain([&c.impulse]).chain(&c.relative_velocity) {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

/// `count` records, checked: steps ascend and exist, bodies exist, numbers are finite.
pub(crate) fn read_contacts(
    data: &[u8],
    count: usize,
    frames: u64,
    bodies: usize,
    start: f64,
    step: f64,
) -> Result<Vec<Contact3>, String> {
    if data.len() / CONTACT_RECORD < count {
        return Err("truncated contacts".into());
    }
    let f64_at = |o: usize| f64::from_le_bytes(data[o..o + 8].try_into().unwrap());
    let i32_at = |o: usize| i32::from_le_bytes(data[o..o + 4].try_into().unwrap());
    let body = |k: usize, v: i32| match v {
        -1 => Ok(None),
        i if (0..bodies as i32).contains(&i) => Ok(Some(i as usize)),
        _ => Err(format!("contact {k} names body {v}, and the cache has {bodies}")),
    };
    let mut contacts = Vec::with_capacity(count);
    for k in 0..count {
        let at = k * CONTACT_RECORD;
        let index = u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
        if index >= frames {
            return Err(format!("contact {k} is at step {index}, and the cache has {frames} steps"));
        }
        if contacts.last().is_some_and(|c: &Contact3| c.step > index) {
            return Err(format!("contact {k} is out of step order"));
        }
        let bodies = [body(k, i32_at(at + 8))?, body(k, i32_at(at + 12))?];
        match bodies {
            [None, _] => return Err(format!("contact {k} has no first body")),
            [Some(a), Some(b)] if a >= b => return Err(format!("contact {k} lists its bodies out of order")),
            _ => {}
        }
        let mut v = [0.0; 10];
        for (i, slot) in v.iter_mut().enumerate() {
            *slot = f64_at(at + 16 + 8 * i);
        }
        if v.iter().any(|x| !x.is_finite()) {
            return Err(format!("contact {k} has a number that is not finite"));
        }
        contacts.push(Contact3 {
            step: index,
            time: start + (index + 1) as f64 * step,
            bodies,
            point: [v[0], v[1], v[2]],
            normal: [v[3], v[4], v[5]],
            impulse: v[6],
            relative_velocity: [v[7], v[8], v[9]],
        });
    }
    Ok(contacts)
}

/// Accumulates what identifies a document's physics.
pub(crate) struct Identity(Sha256);

impl Identity {
    pub(crate) fn new() -> Identity {
        let mut hash = Sha256::new();
        hash.update(MAGIC);
        Identity(hash)
    }

    /// A labelled value, by its `Debug` text: floats print with the shortest text that reads
    /// back exactly, so equal values give equal text and different ones do not.
    pub(crate) fn value(&mut self, label: &str, value: &dyn Debug) {
        let mut text = Text(&mut self.0);
        let _ = write!(text, "{label}={value:?};");
    }

    pub(crate) fn part(&mut self, label: &str, digest: Option<[u8; 32]>) {
        self.0.update(label.as_bytes());
        match digest {
            Some(d) => {
                self.0.update([1]);
                self.0.update(d);
            }
            None => self.0.update([0]),
        }
    }

    pub(crate) fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

struct Text<'a>(&'a mut Sha256);

impl Write for Text<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.update(s.as_bytes());
        Ok(())
    }
}

fn absorb_mesh(id: &mut Identity, label: &str, vertices: &[[f64; 3]], triangles: &[[u32; 3]]) {
    id.value(label, &(vertices.len(), triangles.len()));
    for v in vertices.iter().flatten() {
        id.0.update(v.to_bits().to_le_bytes());
    }
    for i in triangles.iter().flatten() {
        id.0.update(i.to_le_bytes());
    }
}

fn absorb_shape(id: &mut Identity, shape: &Shape3) {
    match shape {
        Shape3::TriMesh(v, t) => absorb_mesh(id, "trimesh", v, t),
        Shape3::Decomposition(v, t) => absorb_mesh(id, "decomposition", v, t),
        Shape3::Convex(v) => absorb_mesh(id, "convex", v, &[]),
        other => id.value("shape", other),
    }
}

/// The identity of a 3D world's definition. Destructuring without `..` makes a new field a
/// compile error here, so it cannot be left out of the identity.
pub(crate) fn digest_world3(
    spec: &World3Spec,
    events: &[Fracture3],
    watches: &[ImpactWatch],
    links: &[crate::sim3d::CraterLink],
) -> [u8; 32] {
    let World3Spec { start, step, gravity, pixels_per_meter, iterations, bounds, bodies, joints, fix_internal_edges } =
        spec;
    let mut id = Identity::new();
    id.value("world", &(start, step, gravity, pixels_per_meter, iterations, bounds, bodies.len(), joints.len()));
    // only when on, so that the identity of every world without it is what it was
    if *fix_internal_edges {
        id.value("internalEdges", &true);
    }
    for body in bodies {
        let Body3Spec {
            kind,
            shape,
            mass,
            friction,
            restitution,
            linear_damping,
            angular_damping,
            velocity,
            angular_velocity,
            group,
            collides_with,
            sensor,
            fixed_rotation,
            bullet,
            activate_at,
            start,
        } = body;
        id.value("body", &(kind, mass, friction, restitution, linear_damping, angular_damping));
        id.value("motion", &(velocity, angular_velocity, activate_at, start));
        id.value("collision", &(group, collides_with, sensor, fixed_rotation, bullet));
        absorb_shape(&mut id, shape);
    }
    for joint in joints {
        id.value("joint", joint);
    }
    for event in events {
        id.value("fracture", event);
    }
    id.value("impacts", &watches);
    for link in links {
        id.value("crater", &(link.watch, link.owner, &*link.source));
    }
    id.finish()
}
