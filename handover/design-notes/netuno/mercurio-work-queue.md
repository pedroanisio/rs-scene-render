---
name: mercurio-work-queue
description: "Mercurio (render front, crates/sr-gpu) branches, decisions and queue as of 2026-10-07 night; orders come from Urano by cross-session messages"
metadata:
  node_type: memory
  type: project
  originSessionId: 8e0c69e6-b97d-44af-abdc-223a64772d14
  modified: 2026-10-07T20:01:03.561Z
---

Alias Mercurio (render front). Coordinator Urano gives orders by cross-session message (not user approvals); Saturno = schema/corpus/sim/importer, Netuno = ocean and voxel physics (Occupancy in sr-3d, frames/world voxel API). Worktrees under `.claude/worktrees/`: mercurio-voxel (feat/voxel-surface), mercurio-strat (feat/foam-stratified). Delete finished worktrees (disk is tight; each target is 5-15 GB).

State on 2026-10-07 ~19:50:
- 4.4 foam as albedo mix: MERGED into main (a590300, tip 872ff3f). Model: foam share = area covered, each sample on the foam with that probability (non-metal white diffuse, no transmission), else untouched water; metallic accepted, unlit/emissive/non-opaque refused (renderer error + W08), raster refuses (error), coverage made only for takes-foam material + path-traced camera, dense-grid bins with exact memory. fix/gl-skip and fix/gl-probe-scopes merged too.
- feat/foam-stratified 6493933 (stratified lobe choice at the first hit; cv 0.345 -> 0.156): delivered to Urano, he integrates after reading. NEE of two lobes only when a scene with sun + albedo foam exists (registered in the SREP, not now).
- V.3 INTEGRATED into origin/main (f64664e on 2026-10-08); branch feat/voxel-surface and worktree mercurio-voxel deleted. Design note: /home/pals/renders/cinematic-impact/phase3/voxels/V3-surface-design-note.md.
- ON HOLD by user order (2026-10-08 11:00, via Urano) until told to resume: 4.1 on branch feat/scatter-bounces (worktree mercurio-scatter, tip a3747b5 over origin/main 149badb): a7cf5bd python reference (tools/volume_slab_reference.py --check), 42ee573 schema/rule W10/corpus/SREP, a3747b5 red tests of commit 3 (do not compile; next: needs_scatter, variant_slot/variant_source scatter arg with 40 slots, Optical::scatter_bounces, pack lane row13.w, render_three reads attribute). Design note: /home/pals/renders/cinematic-impact/phase3/volume-scatter/4.1-scatterBounces-design-note.md. Then commits 4-6 of the note's section 8.
- NEXT: 4.1 `medium@scatterBounces` design note (NO CODE before the note; Urano's order). Semantics decided: scatterBounces = max collisions on a path, default 1 = today (single scattering with direct lights; Urano wrote '0' by slip). Estimator validated in numpy (scratchpad ms/slab_ref.py): weighted reservoir over the march steps + HG random walk + NEE at each vertex; oracle = azimuth-resolved analog photon walk and exact isotropic slab; white-furnace test (radiance 1 for albedo 1, n large). Hook only when some medium has scatterBounces>1 so n=1 shader text is byte-identical. Then 1.14 persistent BVH.

Lessons: never `cat >` a path before checking it exists (I overwrote Saturno's voxel.rs once; restored); python slice-replace with an empty `old` inserts at file start; the shared cargo target dir was retired (workspace crates collide across worktrees); `grep -r` over crates matches big fixtures: restrict paths.

**How to apply:** check `git log` of those branches and ask Urano for the current main before rebasing.

Related: [[shared-machine-agents]], [[render-verification-notes]]

Identity-check pitfall: scratchpad m7/sky.hdr is a symlink into a worktree; when that worktree is deleted the plume_base/plume_dome scenes fail with an empty error (exit 1): relink to the current worktree's examples/cinematic-impact/sky.hdr before running ident.py (copy it with the chdir changed).
