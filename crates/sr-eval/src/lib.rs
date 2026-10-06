//! # sr-eval
//!
//! Turns a validated scene-render document into frames of drawable state.
//! [`Evaluator::new`] runs templating once (parameters, variants, data rows,
//! binds, overrides, repeats, instances, includes) and compiles every
//! animation element; [`Evaluator::evaluate`] is then a pure function of
//! time that returns a [`FrameGraph`]: nodes in paint order with world
//! transforms, opacity, local and source times, and current property values.
//!
//! ```no_run
//! use sr_eval::{EvalOptions, Evaluator};
//! let doc = sr_model::load_file("promo.scene.xml", &Default::default()).unwrap();
//! let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap();
//! let frame = ev.evaluate_frame(42);
//! println!("{} nodes", frame.nodes.len());
//! ```

#![forbid(unsafe_code)]
pub mod agents;
pub mod channel;
pub mod codes;
pub mod crater;
pub mod curve;
pub mod data;
pub mod eval;
pub mod expr;
pub mod fracture;
pub mod geo;
mod group;
mod joints;
pub mod layout;
pub mod mesh_sequence;
pub mod ocean;
pub mod particles3d;
pub mod path;
pub mod pending;
mod physcache;
pub mod points;
pub mod program;
pub mod pyro;
pub mod rig;
pub mod rng;
pub mod safe_area;
pub mod sim;
mod sim3d;
pub mod solid;
#[doc(hidden)]
pub mod splash;
pub mod stroke_font;
pub mod terrain;
pub mod value;

/// Closest candidate within a small edit distance ("did you mean").
pub(crate) fn suggest<'c>(word: &str, candidates: impl IntoIterator<Item = &'c str>) -> Option<&'c str> {
    fn distance(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        // optimal string alignment: adjacent transpositions cost 1
        let (n, m) = (a.len(), b.len());
        let mut d = vec![vec![0usize; m + 1]; n + 1];
        for (i, row) in d.iter_mut().enumerate() {
            row[0] = i;
        }
        for (j, x) in d[0].iter_mut().enumerate() {
            *x = j;
        }
        for i in 1..=n {
            for j in 1..=m {
                let cost = usize::from(a[i - 1] != b[j - 1]);
                d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
                if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                    d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
                }
            }
        }
        d[n][m]
    }
    let limit = (word.chars().count() / 3).clamp(1, 3);
    candidates
        .into_iter()
        .map(|c| (distance(word, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, c)| (*d, c.len()))
        .map(|(_, c)| c)
}

pub use eval::{
    Affine, BonePose, ElementState, FrameGraph, FrameNode, FrameTransition, JointFrames, Props, SimSeconds, SkinWeights,
};
pub use physcache::PhysicsTrace;
pub use program::{draws_in_3d, Analysis, EvalOptions, Program, THREE_D_DRAWN};
pub use safe_area::SafeEnforce;
pub use value::Value;

/// A compiled document, ready to evaluate any frame.
#[derive(Debug)]
pub struct Evaluator {
    program: program::Program,
    /// Physics and particles, when the document has any.
    sim: Option<std::sync::Mutex<sim::Runtime>>,
}

/// A body's owner, the momentum it gave the water, and the pressure credited to it: [`Evaluator::around_into`].
#[doc(hidden)]
pub type BodyOffer = (u32, [f64; 2], [f64; 2]);

impl Evaluator {
    /// Templates and compiles `doc`. Errors carry `E01`–`E16` diagnostics.
    pub fn new(doc: &sr_model::Document, opts: &EvalOptions) -> Result<Evaluator, sr_model::Report> {
        program::build(doc, opts).map(|program| {
            let sim = sim::needed(&program).then(|| std::sync::Mutex::new(sim::Runtime::default()));
            Evaluator { program, sim }
        })
    }

    /// State at composition time `t` seconds.
    pub fn evaluate(&self, t: f64) -> FrameGraph {
        let mut g = eval::evaluate(&self.program, t);
        if let Some(sim) = &self.sim {
            let mut rt = sim.lock().unwrap_or_else(|e| e.into_inner());
            rt.apply(&self.program, &mut g, t, &|ts| eval::evaluate(&self.program, ts));
            g.problems.extend(rt.problems.iter().cloned());
            for failure in &rt.failures {
                g.fail(failure.clone());
            }
        }
        g
    }

    /// Where the nodes are and what they are at composition time `t`, without stepping any simulation:
    /// the same graph as [`Evaluator::evaluate`] minus simulated state (particles, physics poses, fluids). A
    /// pure function of `t` at the cost of one walk over the nodes, for checks that only need placement.
    pub fn evaluate_layout(&self, t: f64) -> FrameGraph {
        eval::evaluate(&self.program, t)
    }

    /// Whether frames depend on a simulation (physics, particles, agents). Simulated state
    /// is stepped in time order under a lock, so such a document must be evaluated
    /// serially; without one, [`Evaluator::evaluate`] is a pure function of `t` and frames
    /// may be evaluated in any order or on any thread.
    pub fn has_simulation(&self) -> bool {
        self.sim.is_some()
    }

    /// What the particles that fall into ocean `ocean` give it in its canonical step `step`, by cell: the
    /// reading the ocean makes of the particles, for tests of the particles' side of the coupling.
    #[doc(hidden)]
    pub fn splash_into(&self, ocean: &str, step: u64) -> Result<Vec<splash::Cell>, String> {
        match &self.sim {
            Some(sim) => sim.lock().unwrap_or_else(|e| e.into_inner()).splash.read(ocean, step),
            None => Ok(Vec::new()),
        }
    }

    /// What ocean `ocean` offered about each body in its canonical step `step`, as `(owner, impulse, pressure)`
    /// per body, each the momentum per unit water density in scene units: what the body gave the water, and what
    /// the slope of the bed it raised gave the water, which is credited to the body and not applied to it. None
    /// while the step has not been computed.
    #[doc(hidden)]
    pub fn around_into(&self, ocean: &str, step: u64) -> Option<Vec<BodyOffer>> {
        let sim = self.sim.as_ref()?.lock().unwrap_or_else(|e| e.into_inner());
        let group = sim.physics.as_ref()?.group.as_ref()?;
        let list = group.around_at(group.channel(ocean)?, step)?;
        Some(list.iter().map(|a| (a.owner, a.impulse, a.pressure)).collect())
    }

    /// Simulates the document's physics to its end and returns a physics cache file
    /// (`physics@cache`; its SHA-256 goes in `cacheSha256`).
    pub fn physics_cache(&self) -> Result<Vec<u8>, String> {
        sim::write_cache(&self.program, self.program.duration, &|ts| eval::evaluate(&self.program, ts))
    }

    /// The velocities and contacts of the 3D rigid bodies over the document's duration:
    /// those of the verified `physics@cache` when there is one, otherwise simulated.
    /// A cache baked from other physics is an error.
    pub fn physics_trace(&self) -> Result<PhysicsTrace, String> {
        sim::physics_trace(&self.program, &|ts| eval::evaluate(&self.program, ts))
    }

    /// State at frame `n` of the project frame rate.
    pub fn evaluate_frame(&self, n: u64) -> FrameGraph {
        self.evaluate(self.program.fps.frame_time(n))
    }

    /// Frames in the timeline.
    pub fn frame_count(&self) -> u64 {
        self.program.fps.frame_count(self.program.duration)
    }

    /// The compiled program.
    pub fn program(&self) -> &program::Program {
        &self.program
    }

    /// Warnings raised while templating (for example unknown placeholders).
    pub fn warnings(&self) -> &[sr_model::Diagnostic] {
        &self.program.warnings
    }

    /// Counts: (nodes, animated slots, channels, expressions).
    pub fn stats(&self) -> (usize, usize, usize, usize) {
        let p = &self.program;
        (p.nodes.len(), p.slots.len(), p.channels.len(), p.exprs.len())
    }
}
