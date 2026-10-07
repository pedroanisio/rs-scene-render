//! The surface of a grid of voxels: the faces of its cells that are exposed, merged into quads.
//!
//! The extraction is a pure function of an [`Occupancy`](crate::occupancy::Occupancy) and the table of [`Classes`] that says which palette indices look alike and
//! which let light through; it needs no GPU and gives the same bits whatever order the cells were given in and however many
//! threads there are (it uses none).
//!
//! * **Coordinates.** A face lies on a plane of the lattice: the integer coordinate it has along its `axis`; the cell `[i, j, k]` has
//!   its low face on the plane `i` and its high face on the plane `i + 1` along x, and so on. The two other coordinates of a face are
//!   `u` and `v`, in cyclic order (axis 0: y, z; axis 1: z, x; axis 2: x, y), so that `u x v` points along `+axis`.
//! * **Exposure.** The face of cell A toward its neighbour B (by a face; cells that touch by an edge or a corner hide nothing of
//!   each other) is exposed when B is empty; or A is opaque and B see-through; or both are see-through of different classes and A's
//!   class is the lesser, so that an interface has one owner and no two faces coincide. Two cells of the same see-through class are
//!   one body, with no face between them.

mod exposure;
mod greedy;

pub use exposure::{exposed_faces, Face};
pub use greedy::{mesh_quads, quads_hash, Quad};

/// What the palette indices of a grid look like to the extraction: the class of each index (indices of one class merge into one quad,
/// and the face between two cells of one see-through class is not made) and whether a class lets light through. Index 0 is empty and
/// is class 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classes {
    class: [u8; 256],
    see_through: [bool; 256],
}

impl Classes {
    /// Every index its own class, all opaque.
    pub fn identity() -> Self {
        Self { class: std::array::from_fn(|i| i as u8), see_through: [false; 256] }
    }

    /// The class of each index by `class` (the index of the class's lowest member is the usual label; index 0 stays class 0), and the
    /// classes in `see_through` let light through.
    pub fn new(class: impl Fn(u8) -> u8, see_through: &[u8]) -> Self {
        let mut c =
            Self { class: std::array::from_fn(|i| if i == 0 { 0 } else { class(i as u8) }), see_through: [false; 256] };
        for s in see_through {
            c.see_through[usize::from(*s)] = true;
        }
        c
    }

    /// The same classes with the classes `classes` made see-through.
    pub fn with_see_through(mut self, classes: &[u8]) -> Self {
        for s in classes {
            self.see_through[usize::from(*s)] = true;
        }
        self
    }

    /// The class of a palette index.
    pub fn class(&self, index: u8) -> u8 {
        self.class[usize::from(index)]
    }

    /// Whether a class lets light through.
    pub fn see_through(&self, class: u8) -> bool {
        self.see_through[usize::from(class)]
    }

    /// Whether the face of a cell of class `a` toward a neighbour of class `b` (0 for none) is exposed.
    pub fn exposes(&self, a: u8, b: u8) -> bool {
        if a == 0 {
            return false;
        }
        let (sa, sb) = (self.see_through(a), self.see_through(b));
        b == 0 || (!sa && sb) || (sa && sb && a != b && a < b)
    }
}
