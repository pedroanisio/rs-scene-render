//! What the ejecta that fall into an ocean give it.
//!
//! A particle emitter listed by an ocean (`ocean@splash`) loses the particles whose centre crosses the
//! ocean's rest level inside its domain, and tells this log, once per fixed step of its own, which
//! fell: summed by cell and by canonical step of the ocean, the volume of the solid they bring and the
//! horizontal momentum they carry per unit of water density. The log is written once per
//! (emitter, step): a step computed again must reproduce its entries to the bit, or it is an error,
//! because the ocean has already used them. The ocean reads, for each canonical step, the entries of
//! the emitters' steps that overlap it, and it is an error to ask for a step the particles have not
//! reached, never a stand-in of no splash.

use std::sync::{Arc, Mutex};

use glam::{DMat4, DVec3};
use sr_sim::exchange::{ExchangeLog, Put, Record};
use sr_sim::particles3d::Absorbed;

/// Bytes the log may hold: an entry is 40 bytes, a step 64 on top of its entries; over it is an error.
const LOG_BYTES: usize = 16 << 20;

/// What the particles of one cell give the water in one canonical step: scene units cubed of solid, and horizontal
/// momentum per unit of water density in scene units to the fourth a second, in the ocean's own axes. The ocean
/// solver's own type, so that what is read is what is given.
pub use sr_sim::ocean::SplashCell as Cell;

/// One cell of one canonical step of the ocean, as a particle step recorded it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Entry {
    pub(crate) ocean_step: u64,
    pub(crate) cell: u32,
    pub(crate) volume: f64,
    pub(crate) momentum: [f64; 2],
}

impl Record for Entry {
    fn same(&self, other: &Self) -> bool {
        self.ocean_step == other.ocean_step
            && self.cell == other.cell
            && self.volume.to_bits() == other.volume.to_bits()
            && self.momentum.iter().zip(&other.momentum).all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

/// An ocean as the particles that fall into it need it, taken at its pose when the emitter starts.
#[derive(Clone, Debug)]
pub(crate) struct Ocean {
    pub(crate) id: Arc<str>,
    /// The ocean's own axes of the corner of its domain, the cell size and the cells along x and z.
    pub(crate) origin: [f64; 2],
    pub(crate) cell_size: f64,
    pub(crate) cells: [usize; 2],
    /// Its canonical step, and the composition time of its local time zero.
    pub(crate) dt: f64,
    pub(crate) start: f64,
    /// World to the ocean's own axes.
    pub(crate) to_local: DMat4,
}

/// An emitter that falls into an ocean.
#[derive(Clone, Debug)]
pub(crate) struct Emitter {
    /// What the log keys it by.
    pub(crate) channel: u32,
    /// The composition time of its fixed step zero, and its fixed step.
    pub(crate) start: f64,
    pub(crate) dt: f64,
    pub(crate) ocean: Ocean,
}

impl Emitter {
    /// The fixed steps `first..=last` of the emitter that overlap the canonical step `step` of the ocean.
    fn steps_in(&self, step: u64) -> (u64, u64) {
        let (low, high) =
            (self.ocean.start + step as f64 * self.ocean.dt, self.ocean.start + (step + 1) as f64 * self.ocean.dt);
        let first = ((low - self.start) / self.dt + 1e-9).floor().max(0.0) as u64;
        let last = (((high - self.start) / self.dt - 1e-9).ceil() - 1.0).max(0.0) as u64;
        (first, last)
    }

    /// The local time of the emitter by which it must have been computed for the ocean to be asked at composition
    /// time `time`: the ocean at a canonical step is given, with the step it is in, the splash of the step that
    /// follows, so what falls until the end of the step that holds `time` has to be known. `own_start` is the
    /// composition time of the emitter's local time zero.
    pub(crate) fn needed_until(&self, time: f64, own_start: f64) -> f64 {
        let (dt, local) = (self.ocean.dt, (time - self.ocean.start).max(0.0));
        // the step the solver counts the time in
        let mut step = (local / dt).floor() as u64;
        while step > 0 && step as f64 * dt > local {
            step -= 1;
        }
        while (step + 1) as f64 * dt <= local {
            step += 1;
        }
        self.needed_for(step, own_start)
    }

    /// The local time of the emitter by which it must have been computed for the ocean's canonical step `step`.
    pub(crate) fn needed_for(&self, step: u64, own_start: f64) -> f64 {
        let fixed = self.steps_in(step).1 + 1;
        self.start - own_start + fixed as f64 * self.dt
    }
}

#[derive(Default)]
struct Inner {
    entries: Option<ExchangeLog<Entry>>,
    emitters: Vec<Emitter>,
}

/// The write-once record of what fell, shared by the emitters and the ocean.
#[derive(Clone, Default)]
pub(crate) struct Log(Arc<Mutex<Inner>>);

impl Log {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Make the emitter known to the ocean's reads.
    pub(crate) fn register(&self, emitter: &Emitter) {
        let mut inner = self.inner();
        if !inner.emitters.iter().any(|e| e.channel == emitter.channel) {
            inner.emitters.push(emitter.clone());
        }
    }

    /// Record what fell in fixed step `step` of the emitter. A replay must reproduce it exactly.
    pub(crate) fn put(&self, channel: u32, step: u64, entries: &[Entry]) -> Result<Put, String> {
        let mut inner = self.inner();
        inner.entries.get_or_insert_with(|| ExchangeLog::new(LOG_BYTES)).put(channel, step, entries)
    }

    /// Whether an emitter that falls into `ocean` has made itself known: until one has, a read of the ocean's steps
    /// would be empty because nobody had said anything, not because nothing fell.
    pub(crate) fn knows(&self, ocean: &str) -> bool {
        self.inner().emitters.iter().any(|e| &*e.ocean.id == ocean)
    }

    /// What the particles give ocean `ocean` in its canonical step `step`, by cell in order of cell: the
    /// entries of every fixed step of every emitter that falls into it that overlaps that step, summed in
    /// the order of the steps. None for an ocean no emitter falls into.
    pub(crate) fn read(&self, ocean: &str, step: u64) -> Result<Vec<Cell>, String> {
        let inner = self.inner();
        let mut sums: std::collections::BTreeMap<u32, Cell> = Default::default();
        for emitter in inner.emitters.iter().filter(|e| &*e.ocean.id == ocean) {
            let (first, last) = emitter.steps_in(step);
            for k in first..=last {
                let log = inner.entries.as_ref();
                let Some(list) = log.and_then(|l| l.get(emitter.channel, k)) else {
                    let reached = log.and_then(|l| l.last_step(emitter.channel));
                    return Err(format!(
                        "the splash into ocean {ocean} at its step {step} needs fixed step {k} of the particles, which has not \
                         been computed (they have reached {})",
                        reached.map_or("no step".to_string(), |r| format!("step {r}"))
                    ));
                };
                for entry in list.iter().filter(|e| e.ocean_step == step) {
                    let cell =
                        sums.entry(entry.cell).or_insert(Cell { cell: entry.cell, volume: 0.0, momentum: [0.0; 2] });
                    cell.volume += entry.volume;
                    cell.momentum[0] += entry.momentum[0];
                    cell.momentum[1] += entry.momentum[1];
                }
            }
        }
        Ok(sums.into_values().collect())
    }
}

/// What the particles that fell in one fixed step of `emitter` give the ocean, by canonical step and cell: the
/// volume is the mass over `solid_density` in scene units cubed (`pixels_per_meter` scene units a metre), the
/// momentum is mass times the horizontal velocity in the ocean's own axes, over `water_density`, in scene
/// units to the fourth a second. Sums run in the order of the particles' ids, so they are the same however the
/// particles were stepped.
pub(crate) fn aggregate(
    list: &[Absorbed],
    emitter: &Emitter,
    pixels_per_meter: f64,
    solid_density: f64,
    water_density: f64,
) -> Vec<Entry> {
    let ocean = &emitter.ocean;
    let mut sums: std::collections::BTreeMap<(u64, u32), Entry> = Default::default();
    let units = pixels_per_meter.powi(3);
    for a in list {
        let local = ocean.to_local.transform_point3(DVec3::from(a.position));
        let velocity = ocean.to_local.transform_vector3(DVec3::from(a.velocity));
        let ix = (((local.x - ocean.origin[0]) / ocean.cell_size).floor().max(0.0) as usize).min(ocean.cells[0] - 1);
        let iz = (((local.z - ocean.origin[1]) / ocean.cell_size).floor().max(0.0) as usize).min(ocean.cells[1] - 1);
        let cell = (iz * ocean.cells[0] + ix) as u32;
        // the canonical step whose window (n dt, (n + 1) dt] holds the instant, in the ocean's local time
        let local_time = a.time + emitter.start - ocean.start;
        let step = ((local_time / ocean.dt - 1e-9).ceil() - 1.0).max(0.0) as u64;
        let entry =
            sums.entry((step, cell)).or_insert(Entry { ocean_step: step, cell, volume: 0.0, momentum: [0.0; 2] });
        entry.volume += a.mass / solid_density * units;
        entry.momentum[0] += a.mass * velocity.x * units / water_density;
        entry.momentum[1] += a.mass * velocity.z * units / water_density;
    }
    sums.into_values().collect()
}
