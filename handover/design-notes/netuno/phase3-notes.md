# Notes for Phase 3 (Netuno, 2026-10-06). Analysis only: nothing here is committed.

Probes that produced the numbers are in this folder (not in the repository): `crater_cost.rs` (+ the temporary timers in
sr-sim physics3d.rs and sr-eval sim3d.rs, discarded), `trimesh_cost.rs`, `ref_gauss.rs`, `ref_disp.rs`; raw output in
`crater-cost.txt`, `ref-gauss.txt`, `ref-disp.txt`. Machine load1 about 10 for the crater probe, about 10-15 for the others.
Line numbers are those of main at 5e678b6 unless said.

## 1. A cheap collider for a crater that deforms

### Where the time goes (measured)
Authored land and ocean scenes, ground of 160 x 160 segments = 25 921 vertices, 51 200 triangles, `fixInternalEdges="true"`.
In the rigid steps that rebuild the surface (5 per frame, in the frames where the crater grows: about 9-10 frames of each film):

| per rebuilt step | land | ocean |
|---|---|---|
| `SharedShape::trimesh_with_flags` (physics3d.rs:1233) | 40.9-42.5 ms | 37.8-41.1 ms |
| the driver's `surface` (sim3d.rs:908-928): crater growth 2.0 ms + vertex map 0.5-0.6 ms | 2.6 ms | 2.5-2.6 ms |
| `set_shape` (physics3d.rs:1240) | 0.0-0.15 ms | 0.07-0.25 ms |
| rapier `pipeline.step` (physics3d.rs:1356) | 1.5 ms | 1.5 ms |

So the trimesh build is 93-95 % of the step; the "24-33 ms" of the earlier ledger entry was the same build on a quieter machine.
Whole film: 3.0 s (land) and 1.9 s (ocean) of trimesh builds in 96 frames; the frames that rebuild take 260-390 ms against 60-190 after.
Microbench on a 160 x 160 grid mesh (trimesh_cost.rs): no flags 15.9 ms (the BVH), `FIX_INTERNAL_EDGES_TWO_SIDED` 41.0 ms: the
topology and pseudo-normals for the internal edges are 25 ms of the 41. `TriMesh::set_vertices` on a mesh built with `DEFORMABLE`
(parry3d-f64 0.31.1, src/shape/trimesh.rs:979-1008) takes 17.4 ms whatever number of vertices moved (it refits every leaf and
recomputes all the pseudo-normals); a first build with `DEFORMABLE` 46 ms.

### A second, cheaper finding
After the crater has finished growing the rigid world still asks `surface` every rigid step and the driver computes the crater
(`impact_crater` + `from_impact` + `graphs.at(t)`, sim3d.rs:919-927) BEFORE it checks that the revision is unchanged
(`revision == Some(current)`, sim3d.rs:1002, inside `deformed_surface`). That is 2 ms a step: 9.8 ms a frame on the land film
(frames 52-96, about 5 % of a 180 ms frame) and 10-20 ms on the ocean film (10-15 % of 60-110 ms), for nothing.

### Proposals, by saving and by risk to the bits
- A. Check the revision (the progress of the crater, or that it has finished) before computing the crater. Saves 10 ms a frame
  after the impact. Same values by construction. No risk. Test: the number of `impact_crater` calls after growth is 0 and the
  bodies' bits do not change.
- B. Prefetch: once the impact is known the surface at any later step is a pure function of its time. Build the 5 meshes of a
  frame's rigid steps on 5 threads before the step loop (rayon, one mesh per task, results kept in step order) and install each
  at its step. Same `trimesh_with_flags` calls, so bit-identical; 200 ms of builds become about 45 ms of wall on 5 cores
  (8-9 ms a step), not under 3 ms. Needs a pre-pass before the loop of `step_once` (physics3d.rs:1195-1241 is inside it); the
  step in which the impact is noticed builds synchronously. Risk: thread count must not change bits (it cannot: no sums).
- C. Update in place with `set_vertices` (17 ms instead of 41, 2.4x). Risk: a tree that is a refit of the previous revision's
  differs from a tree built from scratch, and a checkpoint restore asks the driver for the surface with `revision = None`
  (physics3d.rs:1209) and builds from scratch: the contacts' order, the solver's order and the bits could differ between a live
  run and a replay. Mitigation that keeps it a pure function of the vertices: always build the BVH on the REST mesh and refit it
  to the revision's vertices (`refit_without_opt` changes no topology, so refit(v_k) from the rest tree equals refit(v_k) after
  refit(v_1..v_{k-1})). The restore then does one rest build + one refit.
- D. Partial update, only the facets the crater touches (the crater footprint is, by my estimate and not measured, about 5 % of this ground's vertices): `Bvh::refit_partial`
  exists (parry bvh_refit.rs:644) but `TriMesh` exposes its pseudo-normals read-only (trimesh.rs:2174), so this needs a patched or
  vendored parry3d-f64 (own `update_region(vertices, touched)`). Estimate 1-3 ms a step (not measured): the only route to under
  3 ms on this mesh. The pseudo-normal of a vertex is a sum over its triangles in index order (trimesh.rs:1437-1457); a partial
  recompute that sums in the same order is bit-identical to the full one; the edge pseudo-normals use a HashMap in parry and must
  be read before promising anything. Risk: maintaining a fork.
- E. An analytic height field at the contact (rapier `HeightField`): different shape and different internal-edge handling, so the
  contacts would not agree to 1e-9; not recommended. A coarser/limited mesh patch around the crater with a static outer mesh
  would be cheaper (about 6 ms for a 7 200 triangle patch, estimated from the 16 + 25 ms per 51 200) but puts a seam where
  contacts differ and ejecta slide: also not recommended.

### Acceptance test
(1) Contacts: a rock dropped on the growing crater, run with the current rebuild and with the new path, compare every step's
contact points, normals, depths and their order to 1e-9 (bit equality for A and B and for C/D if the restore design holds) and
the bodies' final bits. (2) Determinism: restore a checkpoint at step k, replay to k+n, compare the state bits with the live run;
and 1, 2 and 8 threads for B. (3) Cost: median over the rigid steps in contact of `surface + shape update` (Instant, release):
under 3 ms for D, at most 10 ms for B, 17 ms for C; the test prints the median and the load.

## 2. A physical reference for the ocean

The impact wave has never been compared with anything outside the code. A prototype (ref_gauss.rs) did the first comparison below.

### Case 1 (a unit test): a Gaussian hump in linear shallow water, exact
Initial surface `eta0 = A exp(-r^2 / 2 sigma^2)` over water of depth h at rest, `c = sqrt(g h)`. The linear shallow-water equations
give, from the Hankel transform, `eta(r, t) = A integral_0^inf s exp(-s^2/2) J0(s r / sigma) cos(c s t / sigma) ds`. The prototype
evaluates it by Simpson on s in [0, 14] with 1200 points and J0 by its integral representation (320 nodes, exponentially
convergent), no dependency, 3.5 s for 481 radii, then interpolates by cubic Lagrange.
Setup: g = 10, h = 10 (c = 10), sigma = 8, A = 0.01 (A/h = 1e-3), t = 4 s, a closed 160 x 160 basin (the first reflection is at
r = 80, t = 8 s), dt 0.05; the solver at `Ocean::at(4.0)`; exact peak 0.157 A. Measured (L-infinity error over the cells as a
fraction of A; the order is between consecutive rows):

| sigma/dx | first order | order | second order | order |
|---|---|---|---|---|
| 4 | 6.9e-2 | | 1.0e-2 | |
| 8 | 4.4e-2 | 0.63 | 3.1e-3 | 1.70 |
| 16 | 2.6e-2 | 0.78 | 9.3e-4 | 1.73 |
| 32 | 1.4e-2 | 0.88 | 3.9e-4 | 1.27 |

(second order at 640 x 640 cells takes 70 s: a test stops at sigma/dx = 16, 7.7 s.) The last second-order row sits near the floor of
the linear reference: the solver is nonlinear and A/h = 1e-3; a test should use A/h = 1e-4 or accept the floor. Proposed asserts:
second order at sigma/dx = 16 within 2e-3 A, first order within 4e-2 A, observed orders at least 0.6 and 1.5, plus the volume
conserved to 1e-12 and the L2 error. It measures dissipation and the scheme's order against an exact answer, which nothing does now.

### Case 2 (a documented limit, not an assertion): the same with dispersion
Real water at finite depth is dispersive, `omega^2 = g k tanh(k h)`; a bed uplift is `1 / cosh(k h)` in the surface (Kajiura 1963,
Hammack 1973; the Cauchy-Poisson problem of Kranzer and Keller 1959; all from memory, not read). The same integral with those
two factors is the exact linear reference for `bedResponse="depthFiltered"` plus propagation. The engine propagates without
dispersion, so it cannot agree for narrow humps; computing the exact answers shows how much (ref_disp.rs, peak |eta|/A and where):
- sigma/h = 4 (sigma 40, h 10, t 20): shallow-water peak 0.1568 at r = 220.5, dispersive 0.1516 at 214.5: the leading crest is 3 % lower.
- sigma/h = 0.8 (sigma 8, h 10, t 4): shallow-water crest 0.1568 at r = 44, but the dispersive solution's largest excursion is a
  trough, -0.2374 at r = 14: the solver has the wrong wave there, not a slightly wrong one.
So the test would assert agreement of the leading crest only for sigma/h >= 4 and record the others. This also matters for the
authored sea: the cavity of the 90 478 kg rock at 20 m of water has a radius of the order of the depth (sigma/h of about 0.5 to 1),
where the exact linear answer is a dispersed train whose leading crest the solver does not give; the far-wave figures of the
sweeps are therefore a property of the engine's model and not a prediction of real water. (An estimate: the radius is not
measured here.)

### Case 3 (an external order of magnitude, after reading): Ward and Asphaug 2000
Ward and Asphaug (Icarus 145, 64-78, 2000, from memory, not read) give the tsunami of an asteroid impact from the cavity of the
impact, with the wave amplitude falling with distance and the dispersive correction; I do not remember their formulas well enough
to state them. Proposal: read the paper, take its amplitude-distance law for the cavity of the authored rock (the radius and depth
of the law's cavity are in the SREP), and compare the engine's far-wave heights at 20-40 m and farther to it within a factor of two,
recorded in the ledger as a bracket and not as a criterion; the rock's mass and speed are in the scene. Without reading it,
nothing here should be called a comparison with the literature.

### Cheap extras on the same setup
Energy: the linear solution conserves `E = 1/2 rho g integral eta^2 + 1/2 rho h integral |u|^2`; the numerical loss of the scheme
by order and resolution is a number for the ledger. Arrival time of the front: `r = c t` to a cell.
