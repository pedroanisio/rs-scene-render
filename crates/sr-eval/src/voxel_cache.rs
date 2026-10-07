//! What is derived from the cells of a body, kept by the content of the cells.
//!
//! The mass properties, the connected components and the body for the world (see [`crate::voxels`]) are functions of the cells and of nothing else,
//! so they are kept by what the cells are and not by the revision of the grid that held them (a revision compares states of one grid only, and a
//! replay that builds the grid again has other numbers for the same cells).
//!
//! **A hit is never a guess.** The key is the 64-bit fingerprint of the content, the number of cells and the number of bricks, and an entry also keeps
//! the content itself (the bricks, byte for byte); a lookup that finds a key compares the grid it is given with that, brick by brick, and only an
//! equal one is a hit. A fingerprint that is the same for two different contents (which a 64-bit hash cannot rule out) costs a second entry and not a
//! wrong answer; [`DerivedCache::with_fingerprint`] lets a test force that. The comparison is a memory compare of the bricks, which is cheaper than
//! any of the products. What is kept is charged to the budget, the content included, and the least recently asked entry goes first when the budget is
//! passed: by a counter of the requests and not by a clock, so that the same requests give the same cache. An entry that alone is over the budget is
//! not kept (a budget of 0 keeps nothing and every answer is still right). What cannot be derived (an error) is not kept.
use crate::voxels::{body, Body};
use sr_3d::occupancy::{Occupancy, Properties, BRICK};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Cells in a brick.
const BRICK_CELLS: usize = (BRICK as usize) * (BRICK as usize) * (BRICK as usize);

type Brick = ([i32; 3], Box<[u8; BRICK_CELLS]>);

/// The content of a grid as it was asked: its bricks, to compare a grid with.
struct Snapshot(Vec<Brick>);

impl Snapshot {
    fn of(o: &Occupancy) -> Self {
        Snapshot(o.bricks().map(|(k, c)| (k, Box::new(*c))).collect())
    }

    fn matches(&self, o: &Occupancy) -> bool {
        let mut bricks = o.bricks();
        for (key, cells) in &self.0 {
            match bricks.next() {
                Some((k, c)) if k == *key && c == &**cells => {}
                _ => return false,
            }
        }
        bricks.next().is_none()
    }

    fn bytes(&self) -> usize {
        self.0.len() * (BRICK_CELLS + 16)
    }
}

#[derive(Clone)]
enum Product {
    Properties(Arc<Properties>),
    Components(Arc<Vec<Vec<[i32; 3]>>>),
    Body(Arc<Body>),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    kind: u8,
    fingerprint: u64,
    cells: u64,
    bricks: usize,
    /// Cell size (three), density and scale, as the bits of the numbers.
    params: [u64; 5],
}

struct Entry {
    snapshot: Snapshot,
    product: Product,
    bytes: usize,
    used: u64,
}

/// Derived products of an [`Occupancy`] (mass properties, connected components, the body for the world), by content.
pub struct DerivedCache {
    budget: usize,
    fingerprint: fn(&Occupancy) -> u64,
    entries: BTreeMap<Key, Vec<Entry>>,
    bytes: usize,
    tick: u64,
    computed: u64,
    hits: u64,
    evicted: u64,
}

impl std::fmt::Debug for DerivedCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DerivedCache")
            .field("budget", &self.budget)
            .field("bytes", &self.bytes)
            .field("computed", &self.computed)
            .field("hits", &self.hits)
            .field("evicted", &self.evicted)
            .finish()
    }
}

impl DerivedCache {
    /// A cache that keeps at most `budget` bytes.
    pub fn new(budget: usize) -> Self {
        Self::with_fingerprint(budget, Occupancy::fingerprint)
    }

    /// The same with another fingerprint of the content, to show that a hit does not rest on it: a test gives one that is the same for every grid.
    pub fn with_fingerprint(budget: usize, fingerprint: fn(&Occupancy) -> u64) -> Self {
        DerivedCache {
            budget,
            fingerprint,
            entries: BTreeMap::new(),
            bytes: 0,
            tick: 0,
            computed: 0,
            hits: 0,
            evicted: 0,
        }
    }

    fn key(&self, kind: u8, o: &Occupancy, params: [f64; 5]) -> Key {
        Key {
            kind,
            fingerprint: (self.fingerprint)(o),
            cells: o.count(),
            bricks: o.bricks().count(),
            params: params.map(f64::to_bits),
        }
    }

    /// The product of `kind` of `o` for `params`, from the cache if an entry has the same content, else made by `make` (which says its size in bytes).
    fn get(
        &mut self,
        kind: u8,
        o: &Occupancy,
        params: [f64; 5],
        make: impl FnOnce() -> Result<(Product, usize), String>,
    ) -> Result<Product, String> {
        self.tick += 1;
        let key = self.key(kind, o, params);
        if let Some(entry) = self.entries.get_mut(&key).and_then(|v| v.iter_mut().find(|e| e.snapshot.matches(o))) {
            entry.used = self.tick;
            self.hits += 1;
            return Ok(entry.product.clone());
        }
        let (product, size) = make()?;
        self.computed += 1;
        let snapshot = Snapshot::of(o);
        let bytes = size + snapshot.bytes();
        if bytes <= self.budget {
            while self.bytes + bytes > self.budget {
                self.evict_one();
            }
            self.bytes += bytes;
            self.entries.entry(key).or_default().push(Entry {
                snapshot,
                product: product.clone(),
                bytes,
                used: self.tick,
            });
        }
        Ok(product)
    }

    /// Drops the entry asked least recently.
    fn evict_one(&mut self) {
        let oldest = self
            .entries
            .iter()
            .flat_map(|(k, v)| v.iter().enumerate().map(move |(i, e)| (e.used, *k, i)))
            .min_by_key(|(used, _, _)| *used);
        let Some((_, key, index)) = oldest else { return };
        let list = self.entries.get_mut(&key).expect("the key was just found");
        let gone = list.remove(index);
        if list.is_empty() {
            self.entries.remove(&key);
        }
        self.bytes -= gone.bytes;
        self.evicted += 1;
    }

    /// The exact mass, centre of mass and inertia of the cells, `size` metres a side, `density` kilograms a cubic metre.
    pub fn properties(&mut self, o: &Occupancy, size: [f64; 3], density: f64) -> Result<Arc<Properties>, String> {
        let params = [size[0], size[1], size[2], density, 0.0];
        let product = self.get(0, o, params, || {
            let p = o.moments().properties(size, density)?;
            Ok((Product::Properties(Arc::new(p)), std::mem::size_of::<Properties>() + 64))
        })?;
        match product {
            Product::Properties(p) => Ok(p),
            _ => unreachable!("the kind is in the key"),
        }
    }

    /// The connected components of the cells, labelled by their first cell in the scan.
    pub fn components(&mut self, o: &Occupancy) -> Arc<Vec<Vec<[i32; 3]>>> {
        let product = self
            .get(1, o, [0.0; 5], || {
                let parts = o.components();
                let bytes = parts.iter().map(|p| p.len() * 12 + 24).sum::<usize>() + 64;
                Ok((Product::Components(Arc::new(parts)), bytes))
            })
            .expect("the components of a grid are not an error");
        match product {
            Product::Components(p) => p,
            _ => unreachable!("the kind is in the key"),
        }
    }

    /// The body for the world: the cells in the order of the scan and their mass ([`crate::voxels::body`]).
    pub fn body(
        &mut self,
        o: &Occupancy,
        size: [f64; 3],
        density: f64,
        pixels_per_meter: f64,
    ) -> Result<Arc<Body>, String> {
        let params = [size[0], size[1], size[2], density, pixels_per_meter];
        let product = self.get(2, o, params, || {
            let b = body(o, size, density, pixels_per_meter)?;
            let bytes = o.count() as usize * 12 + 128;
            Ok((Product::Body(Arc::new(b)), bytes))
        })?;
        match product {
            Product::Body(b) => Ok(b),
            _ => unreachable!("the kind is in the key"),
        }
    }

    /// Products worked out (a miss that was not an error).
    pub fn computed(&self) -> u64 {
        self.computed
    }

    /// Requests answered from the cache.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Entries dropped to keep the budget.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Bytes kept now, the contents the entries are compared with included.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}
