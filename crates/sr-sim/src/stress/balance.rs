//! What the rest of a rigid body puts on one side of a cut, from the motion of the body in a step and the loads that were put on it: Newton and Euler on the
//! side, in impulses over the step.
//!
//! A body that is rigid has the motion of every one of its points from six numbers, so the momentum and the angular momentum of any part of it, a set
//! of pieces, are known from the mass sums of the part (its mass, its first moment and its second moment, [`MassSum`]) and the motion of the body
//! ([`Rigid`]). The change of them over a step, less the impulses of the loads that act on the part (its weight, the contacts on it, the joint that holds
//! it), is what the rest of the body put on it: `F dt = dP - J` and `M dt = dL - K` about a point `q` that does not move. This is exact for any
//! number of solver iterations (the contact impulses of the world are the step's, which the sum of them over the momentum a body took in a step shows) and
//! for any motion, a spin included: the centripetal load of a spinning body is in `dP` and no load.
//!
//! What is unmodelled in a step (the damping of the body, a velocity that was set, a torque that the driver puts on it) is, if the body has no joint, a
//! rigid acceleration of every point and makes no stress in the body: it is worked out from the balance of the whole body and distributed as that. If it
//! has a joint (a weld to a static body that holds it) the balance of the whole body is the load of the joint, which is statically indeterminate for
//! two and is worked out for one, force and moment, at its anchor.

use super::{cross, dot, sub, Wrench, V3};

/// The mass of a part of a body and its first and second moments about the origin of the body's frame: `m`, `sum m l` and `sum m l l^T`, in kilograms,
/// kilogram metres and kilogram metres squared.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct MassSum {
    pub mass: f64,
    pub first: V3,
    pub second: [V3; 3],
}

impl MassSum {
    pub fn add(&mut self, other: &MassSum) {
        self.mass += other.mass;
        for a in 0..3 {
            self.first[a] += other.first[a];
            for b in 0..3 {
                self.second[a][b] += other.second[a][b];
            }
        }
    }

    /// The centre of mass of the part, in the body's frame.
    pub fn centre(&self) -> V3 {
        self.first.map(|f| f / self.mass)
    }

    /// The integral of `r r^T dm` about the point `about`, in the body's frame.
    pub fn second_about(&self, about: V3) -> [V3; 3] {
        let mut out = [[0.0; 3]; 3];
        for a in 0..3 {
            for b in 0..3 {
                out[a][b] = self.second[a][b] - self.first[a] * about[b] - about[a] * self.first[b]
                    + self.mass * about[a] * about[b];
            }
        }
        out
    }
}

/// The motion of a rigid body: the pose of its frame (a point of the body is at `position + rotation l`, `rotation` by rows), and the velocity of its centre of
/// mass and its angular velocity, in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rigid {
    pub position: V3,
    pub rotation: [V3; 3],
    pub linear: V3,
    pub angular: V3,
    /// The centre of mass of the body, in its own frame.
    pub centre: V3,
}

fn rotate(rows: &[V3; 3], v: V3) -> V3 {
    [dot(rows[0], v), dot(rows[1], v), dot(rows[2], v)]
}

fn rotate_back(rows: &[V3; 3], v: V3) -> V3 {
    std::array::from_fn(|c| rows[0][c] * v[0] + rows[1][c] * v[1] + rows[2][c] * v[2])
}

impl Rigid {
    /// The point `l` of the body's frame, in the world.
    pub fn world(&self, l: V3) -> V3 {
        let r = rotate(&self.rotation, l);
        [self.position[0] + r[0], self.position[1] + r[1], self.position[2] + r[2]]
    }

    /// A point of the world, in the body's frame.
    pub fn local(&self, x: V3) -> V3 {
        rotate_back(&self.rotation, sub(x, self.position))
    }

    /// A vector of the world (a force, a moment), in the body's frame.
    pub fn local_vector(&self, v: V3) -> V3 {
        rotate_back(&self.rotation, v)
    }

    fn com(&self) -> V3 {
        self.world(self.centre)
    }

    /// The inertia tensor of a part about the body's centre of mass, in the world.
    fn inertia(&self, part: &MassSum) -> [V3; 3] {
        let m = part.second_about(self.centre);
        let trace = m[0][0] + m[1][1] + m[2][2];
        let local: [V3; 3] =
            std::array::from_fn(|a| std::array::from_fn(|b| if a == b { trace - m[a][b] } else { -m[a][b] }));
        // R I R^T
        let ri: [V3; 3] =
            std::array::from_fn(|a| std::array::from_fn(|b| (0..3).map(|k| self.rotation[a][k] * local[k][b]).sum()));
        std::array::from_fn(|a| std::array::from_fn(|b| (0..3).map(|k| ri[a][k] * self.rotation[b][k]).sum()))
    }
}

/// The rotation halfway between two (rows: world = R l): the increment `D = A B^T` turned by half its angle about its axis, then `B`.
fn mid_rotation(before: &[V3; 3], after: &[V3; 3]) -> [V3; 3] {
    let d: [V3; 3] = std::array::from_fn(|i| std::array::from_fn(|j| dot(after[i], before[j])));
    let axis = [d[2][1] - d[1][2], d[0][2] - d[2][0], d[1][0] - d[0][1]];
    let sin2 = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if sin2 < 1e-12 {
        // no turn: the start; or a half turn, whose axis this does not find, and which a step does not make
        return *before;
    }
    let k = axis.map(|v| v / sin2);
    // the angle from its sine and cosine (the arc cosine of the trace is blunt for a small turn)
    let angle = (sin2 / 2.0).atan2((d[0][0] + d[1][1] + d[2][2] - 1.0) / 2.0);
    let (c, s) = ((angle / 2.0).cos(), (angle / 2.0).sin());
    let skew = [[0.0, -k[2], k[1]], [k[2], 0.0, -k[0]], [-k[1], k[0], 0.0]];
    let half: [V3; 3] = std::array::from_fn(|i| {
        std::array::from_fn(|j| (if i == j { c } else { 0.0 }) + s * skew[i][j] + (1.0 - c) * k[i] * k[j])
    });
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|m| half[i][m] * before[m][j]).sum()))
}

fn apply(m: &[V3; 3], v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

/// The momentum of the part `part` of the body in the motion `s`.
pub fn momentum(part: &MassSum, s: &Rigid) -> V3 {
    let arm = sub(s.world(part.centre()), s.com());
    let spin = cross(s.angular, arm);
    std::array::from_fn(|a| part.mass * (s.linear[a] + spin[a]))
}

/// The angular momentum of the part about the centre of mass of the body, of its motion relative to that centre (the translation of the body is not in it):
/// `I w`, the inertia of the part about the centre of mass and the angular velocity. What changes over a step is the same whatever the speed of the body, which
/// the angular momentum about a point that is fixed in the world is not (the body moves its own length in a step at a few hundred metres a second, and what
/// it is is the moment of an impulse about where the body was).
pub fn spin_momentum(part: &MassSum, s: &Rigid) -> V3 {
    apply(&s.inertia(part), s.angular)
}

/// A load on the body that acts at a point: its impulse over the step, in newton seconds, at the point `at` of the world, on the piece `piece`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Located {
    pub at: V3,
    pub impulse: V3,
    pub piece: usize,
}

/// A load on the body that is a pure moment: the torque impulse over the step, in newton metre seconds, in the world, on the piece `piece` (the twist of a contact's friction about its
/// normal). A couple has no arm: it is the same moment about any point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Couple {
    pub moment: V3,
    pub piece: usize,
}

/// What happened to a body in a step.
pub struct Step<'a> {
    /// All of the body, its pieces together.
    pub whole: MassSum,
    pub before: Rigid,
    pub after: Rigid,
    pub dt: f64,
    /// The acceleration of every point that does not come from the constraints: gravity and the fields, in the world.
    pub accel: V3,
    /// The contacts, with the impulse that each put on the body over the step.
    pub contacts: &'a [Located],
    /// The twists of the contacts' friction: moments with no force.
    pub couples: &'a [Couple],
    /// The piece that the one joint that holds the body to something else is anchored in. Its load is what the balance of the whole body leaves (a force and
    /// the moment of it about the centre of mass, wherever the anchor is: the load on a part is the same as long as the part has the anchor or not).
    pub anchor: Option<usize>,
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn solve3(m: &[V3; 3], b: V3) -> Option<V3> {
    let det = dot(m[0], cross(m[1], m[2]));
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let c0 = cross(m[1], m[2]);
    let c1 = cross(m[2], m[0]);
    let c2 = cross(m[0], m[1]);
    Some(std::array::from_fn(|a| (c0[a] * b[0] + c1[a] * b[1] + c2[a] * b[2]) / det))
}

impl Step<'_> {
    /// The vector from the centre of mass of the body to a point of the body that is given in the world at the end of the step, as it is in the world at the middle of the step: the
    /// loads are those of the middle of the step, and so must be the arms that their moments are made with (an arm of the end of the step turned half the angle that the body
    /// turns in a step against a force of the middle makes a moment of half that angle times the arm times the force).
    fn arm_to(&self, world_point: V3) -> V3 {
        let local = sub(self.after.local(world_point), self.after.centre);
        rotate(&mid_rotation(&self.before.rotation, &self.after.rotation), local)
    }

    /// The vector from the centre of mass of the body to the centre of mass of the part, in the world at the middle of the step.
    fn part_arm(&self, part: &MassSum) -> V3 {
        rotate(&mid_rotation(&self.before.rotation, &self.after.rotation), sub(part.centre(), self.after.centre))
    }

    /// What the balance of the whole body leaves after the loads that are known (the weight and the fields, the contacts): an impulse and its moment about the centre of
    /// mass. With one joint of the world that is its load; with none, what is not a load on the points of the body (damping, a velocity that was set) and, when a load is not
    /// known (the friction at a contact, whose total the solver does not give), what it is.
    pub fn unbalanced(&self) -> (V3, V3) {
        let dv = sub(self.after.linear, self.before.linear);
        let mut known = self.accel.map(|a| a * self.whole.mass * self.dt);
        let mut known_moment = [0.0; 3];
        for c in self.contacts {
            known = add(known, c.impulse);
            known_moment = add(known_moment, cross(self.arm_to(c.at), c.impulse));
        }
        for c in self.couples {
            known_moment = add(known_moment, c.moment);
        }
        let left = sub(dv.map(|v| v * self.whole.mass), known);
        let left_moment =
            sub(sub(spin_momentum(&self.whole, &self.after), spin_momentum(&self.whole, &self.before)), known_moment);
        (left, left_moment)
    }

    /// The load that the rest of the body puts on the part `part` of it, whose pieces are those for which `in_part` is true, as a force (the impulse over
    /// the step, per second of it) and the moment of it about the point `q` of the world (a point of the body at the end of the step).
    ///
    /// The balance is made about the centre of mass of the body, in the motion relative to it: for the whole body `dL = K`, with `L = I w` and `K` the
    /// moment of the impulses about the centre of mass; for a part `d(I_S w) + m_S (c_S - c) x dv = K_S + K_cut`, where the second term is the part's centre
    /// of mass being carried by the acceleration of the body's, and `K_cut` is what is asked for, brought to `q` by the force's own lever.
    pub fn on_part(&self, part: &MassSum, in_part: &dyn Fn(usize) -> bool, q: V3) -> Wrench {
        let (force, about_com) = self.part_load(part, in_part);
        Wrench { force, moment: add(about_com, cross(sub(self.after.com(), q), force)), point: q }
    }

    /// The same, in the frame of the body and about the point `q` of it (`q` in the body's frame). The load of a step is that of its middle, and so is the frame that it is taken
    /// to: the rotation of the body halfway between the start and the end of the step (a frame of the end turns the direction of a force of a spinning body half a step against it:
    /// a shear of half the angle turned in a step, and a bending moment that a slender body turns into a large stress), and the moment is brought from the centre of mass to
    /// `q` by the body's own vector between them, which no pose enters.
    pub fn on_part_local(&self, part: &MassSum, in_part: &dyn Fn(usize) -> bool, q: V3) -> Wrench {
        let (force, about_com) = self.part_load(part, in_part);
        let middle = mid_rotation(&self.before.rotation, &self.after.rotation);
        let local = |v: V3| rotate_back(&middle, v);
        let (force, about_com) = (local(force), local(about_com));
        Wrench { force, moment: add(about_com, cross(sub(self.after.centre, q), force)), point: q }
    }

    /// The force and the moment about the centre of mass, in the world, that the rest of the body puts on the part.
    fn part_load(&self, part: &MassSum, in_part: &dyn Fn(usize) -> bool) -> (V3, V3) {
        let dt = self.dt;
        let whole_mass = self.whole.mass;
        let dv = sub(self.after.linear, self.before.linear);
        // the whole body: what the balance leaves after the loads that are known (the weight and the fields, the contacts)
        let (left, left_moment) = self.unbalanced();
        // the part's own loads that are known
        let arm = self.part_arm(part);
        let mut on_force = self.accel.map(|a| a * part.mass * dt);
        let mut on_moment = cross(arm, on_force);
        for c in self.contacts.iter().filter(|c| in_part(c.piece)) {
            on_force = add(on_force, c.impulse);
            on_moment = add(on_moment, cross(self.arm_to(c.at), c.impulse));
        }
        for c in self.couples.iter().filter(|c| in_part(c.piece)) {
            on_moment = add(on_moment, c.moment);
        }
        match self.anchor {
            Some(piece) => {
                // a joint: its force at its anchor, and the couple that makes up the moment that is left
                if in_part(piece) {
                    on_force = add(on_force, left);
                    on_moment = add(on_moment, left_moment);
                }
            }
            None => {
                // no joint: what is left is a rigid acceleration of the body, a uniform one for the force and a spin about the centre of mass for the moment
                let share = part.mass / whole_mass;
                on_force = add(on_force, left.map(|v| v * share));
                on_moment = add(on_moment, cross(arm, left.map(|v| v * share)));
                // the spin that makes up the moment that is left is a small thing next to the loads (its angular momentum is the part's share of the body's I times the turn that the moment gave in
                // the step), and it is taken about the pose of the end of the step, whose half-step's turn from the middle is a second-order change of it: not formed at the middle as the arms are
                if let Some(omega) = solve3(&self.after.inertia(&self.whole), left_moment) {
                    let spin = Rigid { linear: [0.0; 3], angular: omega, ..self.after };
                    on_force = add(on_force, momentum(part, &spin));
                    on_moment = add(on_moment, spin_momentum(part, &spin));
                }
            }
        }
        let delta_force = sub(momentum(part, &self.after), momentum(part, &self.before));
        let delta_moment = add(
            sub(spin_momentum(part, &self.after), spin_momentum(part, &self.before)),
            cross(arm, dv.map(|v| v * part.mass)),
        );
        let force = sub(delta_force, on_force).map(|v| v / dt);
        let about_com = sub(delta_moment, on_moment).map(|v| v / dt);
        (force, about_com)
    }
}
