Written for: Urano, who decides the design before I implement it with TDD.

# Design note: `pyro@follow` (a smoke window that follows the plume)

## Problem
The smoke domain is a fixed box centred on the object (width, height, depth, voxelSize give the cells), `boundary="open"` makes its faces a zero-pressure outlet, and material that crosses the top face is simply lost (sample() returns the background beyond one cell), so a tall plume is cut at the domain top. A bigger fixed domain costs cells x 288 B and ~280 to 370 ns per cell per step on 8 threads (1.6 s a step at 192x156x192).

## Proposal
`follow` (boolean, default false) on `<pyro>`, with `followMargin` (cells, default 12, the engine's value: the distance W02 already asks between a source and an open face, and the one at which the SREP measured the top face no longer mattering: peak density 0.481 against 0.525 with the face 26 cells away). Only with `boundary="open"` (new rule PYRO9 in the Schematron and rules.rs, corpus fixtures; W02 no longer warns for a followed face).
The window keeps the same number of cells, so memory, per-step cost and checkpoint size do not change. Before each step, if the smoke (cells with density above 0 or temperature above ambient) comes within `followMargin` cells of a face, the window shifts along that axis by the whole number of cells that restores the margin, and drops the trailing slab only if it holds no smoke (exactly zero density), so no mass is ever lost; if the trailing slab is not empty the window grows no further on that side and the plume is cut there as today (the limit is stated, not hidden). The decision is the min and max index of the active cells: integer, order-independent, the same on any thread count, a pure function of the previous state.

## State, bits, checkpoints
- `State` gains `window: [i64;3]`, the cumulative shift in cells; `origin = base + window * h` is computed afresh, never accumulated. Checkpoints clone the state, so they carry it; a seek replays the same shifts. A shift is a copy of 3 face arrays and 2 cell arrays (the retained cells are bitwise the old ones; entering cells are the background an open face already gives: density 0, ambient temperature, velocity 0).
- Absent or false: no new code runs; the nine goldens of sr-sim `pyro/determinism.rs` (60a458c, x86_64) and every key test stay untouched. True but never shifting: a bounding-box scan only, so the bits equal follow off (a test).
- The turbulence noise is hashed by the linear cell index and the step: with follow on it is hashed by the global cell (i + window), so a point in space keeps its noise when the window moves; with follow off it is the index it was.
- Inputs: sources, colliders, mesh sources and force fields are already sampled at `world_point(origin, ...)`, but the input callback must "depend only on time and index": it now depends on the window, so it reads the pre-step state (`at_with_state`, already used for drag fields) and replays from the checkpoint's window. `Gas` carries `origin` already; a particle outside the window feels still air as today.

## Export, bake and light grid
`State::volume` places the grids with `origin + h/2`, so the density grid's transform moves by exactly `h x shift` and the node transform stays: the smoke does not move in the world. `volume_key` hashes the transform, so group caches invalidate by themselves. SRVSEQ stores a transform for each frame, so a frozen bake of a following plume works; playback interpolates adjacent frames through each grid's own transform, which I have not read and will test. The light grid is rebuilt every frame from the union of bounds (no persistent state), but its lattice is aligned to the bounds minimum, not to the world, so the in-scattering may shimmer when the window steps by a cell: I will measure the frame-to-frame difference first; aligning the lattice to world multiples of its spacing is the renderer's change (Mercurio) and would change pixels of static scenes, so it would be conditioned on a moving volume. Not decided by this note.

## Oracles (each a test that fails first)
1. Absent / true-but-never-shifting = bits of today (determinism goldens, volume_key tests, `checkpointed_seeking_matches_fresh_replay_with_a_tight_memory_budget`).
2. A shift copies: the cells in the overlap are bitwise the pre-shift cells; the density mass before = the mass after (exactly, since only empty slabs drop), and the window moves by whole cells (origin difference is a multiple of h to the bit).
3. A plume that rises 3 times the height of its window is never cut: the highest smoke cell stays at least `followMargin` from the face; against a fixed domain 3 times taller (the reference) the density fields agree in the overlap to the tolerance I measure first (the top-face effect above is the scale: expect a few percent, not bits, because the open boundary is at another distance).
4. Replay: any order of times, a fresh evaluator, a smoke that kept no checkpoint but the first, and `checkpointMemoryMiB` at the tight limit give the same bits, shifts included; a baked cache of the plume plays the same frames.
5. Export: the grid transform at every frame equals the origin by bits; the world position of a smoke feature advected by a uniform wind is the same with follow on and off (to the advection's own error).
6. Cost: follow on with no shift adds one read pass; I will report it against 485 ms at 128x104x128 (limit I would accept: 5 %); a shift is about a state clone (102 ms at 192^3 under load, recorded), amortised over the steps between shifts.

## Steps (one commit each)
1. sr-sim: `window` in State, the shift primitive and oracle 2. 2. The decision, `Spec.follow` and `follow_margin`, determinism oracles 1 and 4. 3. Global-cell turbulence and window-aware inputs. 4. Eval: attribute, export, schema rules, corpus, SREP, W02. 5. The plume of the hero scene past the old top (oracles 3 and 5), light-grid shimmer measurement, ledger.

## For you to decide
a) default `followMargin` 12; b) a non-empty trailing slab: stop following that side and say so (proposed) or drop and record the lost mass; c) whether the light-grid alignment belongs to this item or to Mercurio's queue; d) all three axes follow (proposed), or only the buoyancy axis.
## Limits I already know
The smoke solver is incompressible with one grid: the cells far behind the plume are dropped, not coarsened, so a very long plume still costs one window; the margin is the engine's value; the open face still perturbs flow within a few cells of it, and the follow keeps that face away from the plume rather than removing the effect.
