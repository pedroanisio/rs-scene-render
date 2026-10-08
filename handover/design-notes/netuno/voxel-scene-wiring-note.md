# Scene wiring of voxel bodies, cuts and fractures: what the evaluator needs from the schema

Netuno, 2026-10-07. For Saturno (schema, rules) and Urano. Not tracked (the hygiene check rejects plan language); the schema text and rule codes are Saturno's, the names below are proposals.

## 1. What exists on my side (all pure, none wired)

| Piece | Where | State |
|---|---|---|
| Rigid body of cells (`Shape3::Voxels{size, cells}`, exact mass properties, rapier voxel collider) | sr-sim `physics3d` | main |
| Slots and cuts (`World3::with_voxel_splits`, `Driver3::voxel_cut(t, parent, revision, impact) -> VoxelCut3{added, revision, destroyed, parent_mass, pieces}`) | sr-sim `physics3d/voxel_split.rs` | main (`added` pending Urano's merge) |
| What a cut is made of: `voxels::cut` (policy Stay Largest/Anchored, minCells, maxFragments, Overflow Error/Dust), `voxel_crater::excavate` and `crater_cut` (crater law -> removed, thrown cells with speeds, rim) | sr-eval | merged / pending |
| Fracture of cells: `voxels::fracture(occupancy, Partition, FracturePolicy, size, density, ppm) -> Fractured{source, pieces, dust_body, graph}` over Saturno's `sr_3d::pieces`; the world side `Fracture3{.., dust}` + `World3::fracture_lost` | sr-eval / sr-sim | pending Urano's merge |
| Derived products by content (`DerivedCache`) | sr-eval | pending |

## 2. How the evaluator would use them (the wiring, in the order it is built)

1. **Body** (`sim3d::shape_for`): `rigidBody@shape="voxels"` on an `object3D primitive="voxels"` reads the asset (`voxel_asset::load`) and makes `Shape3::Voxels{size: cellSize * object scale, cells}`; `mass` comes from `density x cells x cell volume` (`voxels::body`).
2. **Slots**: an owner that has a `crater` (source) or a `fracture` child gets `maxFragments` extra placeholder bodies (1 cell, dynamic, disabled) appended to the world's body list after the document's bodies, and one `VoxelSplit3{parent, slots}`. An owner with neither is just a body with no slots (no cost). The world's cost grows with the slots (`apply_voxel_cuts` runs 2 to 3 times a step and clones them): the default of 64 is for owners that can break.
3. **Crater on a voxel owner** (`Driver3::voxel_cut`): when the world has noticed the impact on the owner (`impact: Some(..)`), the driver builds the kernel from the existing `CraterSource` (law, units) as the **conserving** kernel with bulking 1, calls `crater_cut(..)`, and returns `Cut.cut` (the revision is a function of the impact time, e.g. its step). The thrown cells (`Excavation::thrown`, one per cell with position, velocity, palette) go to the existing ejecta particles; the rim is `VoxelCut3::added`.
4. **Fracture on a voxel owner**: at build time `voxels::fracture` partitions the asset; the pieces become fragment bodies (hidden, dynamic, shapes of cells, offsets 0) of an ordinary `Fracture3` with `dust: dust_body`; time or contact triggers as for meshes (`minImpulse`, `energyFraction`, `radialImpulse` unchanged).
5. **Frames for the renderer**: a frame has to say which cells every voxel body has at that frame. The world state is not the frame (a frame served from the log says nothing of the world's present), so I will add to `Frame3` a per-body `voxel_revision` and a `World3::voxel_cells_at(body, revision) -> Option<Arc<Vec<[i32;3]>>>` backed by a per-world table kept under the frame budget. Pieces of slots are synthetic voxel objects in the frame (like the mesh pieces of `fracture`): pose = the slot's pose, cells = its revision's. Mercurio's mesher keys its surfaces by that revision (`bricks_changed`, or `Occupancy::lineage`).

## 3. What I need from the schema (attributes, with the rules I will own as CRT13 and following)

`rigidBody` on an object3D of primitive voxels (all new, `version="1.3"`):

| Attribute | Type | Default | Meaning / rule |
|---|---|---|---|
| `shape="voxels"` | enum value | auto = voxels for a voxels object | the cells are the collider. Allowed only on `primitive="voxels"` (error otherwise). |
| `density` | double > 0, kg/m3 | none | **required** with voxels (or taken from the material table when that has a density: the asset's materials carry none today, so required); `mass` with voxels is an error (the mass is the cells'). |
| `maxFragments` | int 1..4096 | 64 | slots for the pieces of cuts; only with a `crater` or `fracture` on the owner (error otherwise: slots cost). The world refuses more than 4096 fragments of one event. |
| `fragmentMinCells` | int >= 1 | 1 | loose parts with fewer cells are dust (leave the body, become particles). |
| `fragmentOverflow` | `error` \| `dust` | `error` | more loose parts than slots: an error that names both numbers, or the smallest are dust. |
| `anchor` | `largest` \| `base` | `largest` for dynamic, `base` for static and kinematic | which part of what a cut leaves stays the body: the largest, or every part that touches the base layer (the cells of the highest y in scene axes, the way terrain is held by what is under it). A static/kinematic owner with `largest` is allowed but then the floor can fall away: warn. |

`crater` on a voxel owner (the element is the one that exists; CRT5 "static/kinematic with auto or trimesh" gains `voxels`):
- `source` as now; the cut is **instantaneous** at the impact (the crater's `start/end/curve` have no meaning for cells: CRT13 refuses `curve` and a non-default `start/end` on a voxel owner, or you tell me they are ignored with a warning: your call).
- `mantle`, `bulking`, `repose`: **refused** on a voxel owner (CRT13): the rim is cells (kernel with bulking 1, 80% thrown, 20% heaped), a mantle or a repose angle are analytic-surface ideas. `capture` is allowed (it is the world's).
- The ejecta are a `particles3D` whose `burst@crater` names the crater (as now). For a voxel owner the burst's particles are the thrown cells, one particle per cell with that cell's position, velocity and palette colour, and the burst's own count/speed attributes have no meaning: CRT13 refuses a `count` on a burst of a voxel crater (or you say it is a cap: with 5 168 cells thrown by the law's crater at 0.25 m, a cap matters).
- `maxMemoryMiB`: the scan box of the crater (cells in the box) is capped at 2^27 cells; say whether the attribute bounds it (I would use it: cells x 12 bytes) or leave the constant.
- Object scale must be **uniform** for a crater or fracture owner (cells are cubes for the removal and the rim): error otherwise.
- A voxel owner cannot have both a `crater` and a `fracture` (the world excludes a body that is a split parent from fracture ownership): error.

`fracture` on a voxel owner (FRX2 says "one closed surface object3D owner": a voxels object is closed by construction):
- `partition` = `voronoi` | `planes` | `labels` (default `voronoi`), with `pieces` (int, voronoi, <= 4096 seeds), `seed` (int), `planes` (a list of `nx ny nz offset`, up to 63, in doubled cell coordinates or in object units: your choice, I prefer object units and convert), `labels="material"` (the palette index of the cell is its label, so a model breaks along its materials).
- `fragmentMinCells`, `fragmentOverflow`, `maxFragments` are those of the owner's `rigidBody`.
- `interiorMaterial` (FRX3) has no meaning for cells (there is no cut surface to paint): ignored or error, your call; the faces of a piece come from its own cells.

## 4. What I need to know that is not in the schema

1. **The frame of the cells.** The exact map object space <-> cell key: `VoxelModel` moves the box of occupied cells to the origin (`origin_cells`) and the scene axes are y down. For the crater (its `center` and `outward` are in the owner's object space, metres after scale) and for slots (a piece's pose and offset) I need one documented function `cell_to_object(key) -> centre` (including the pivot, `origin_cells` and `cellSize`) and its inverse. If the object's origin is the minimum corner of the box, say so.
2. **Render contract (Mercurio):** where synthetic voxel objects (the pieces) enter the frame and how a surface is keyed (revision of the body in the frame, section 2.5).
3. **Ejecta as particles:** how a `burst@crater` is fed today (sampled from the law, `sr_sim::cratering::ejecta`) and where it can take a list of (position, velocity, palette index) instead; how a particle is drawn as a cell-sized cube, and the cap per crater.
4. **Anchor for terrain**: whether `base` is the right word, or the asset should name its anchor cells (a palette index) instead.

## 5. Limits I will state in the ledger when this is wired

- The cut is instantaneous; non-cubic cells and non-uniform scale are errors for craters and fractures.
- Craters were tested with the axis along a lattice axis and at 45 degrees in one plane, on flat ground; **sloping ground and an axis out of that plane are not tested**.
- Dust loses its momentum (recorded in `World3::fracture_lost`); the pieces of a cut take slots in order and the world refuses more pieces than slots.
- Cost: the cut of the law's crater in a 576 000-cell slab is one `excavate` over a box of about 80 cubed cells; a world of 1e6 cells costs milliseconds per edit under checkpoints (1.8 B/cell measured), and a body of more than ~1.9e7 cells keeps no checkpoint at the default budget.
