---
name: netuno-w6-and-crater-status
description: State of Netuno's two open tracks on 2026-10-08 - W6 stress wiring delivered, crater oracles group 3 (hill) parked on wip/crater-hill
metadata:
  type: project
---

- test/crater-oblique-and-curved: 8c76d05 (groups 1-2) + 78d8e53 (Dir::new refuses a reused name) + cc48191 (slopes under 2 degrees fitted by mean height over the reach disc, guard tests in plane.rs, centre offsets pinned) delivered to Urano; merges with Mercurio's V.3 after the stress verification.
- feat/voxel-stress-wiring: 4202829 over a54089e = W6 (fracture@mode=stress evaluated, E24 removed, scene_stress.rs cantilever oracle: 0.995 P* nothing, 1.005 P* root only). Delivered to Urano for merge.
- wip/crater-hill (ec1cd13, not for merge): group 3. Findings: axis estimator over half the crest radius (approved, whole radius under 4 cells; AXIS_REACH 0.5, MIN_CELLS_ACROSS 4); heap_rim fails on hills ("room for 0 of 1059") because the rim is levelled against the plane, not the local surface - fix to try first: a rim levelled against the local surface. Then groups 4 (basin, ridge) and 5 (edge with pieces).
**How to apply:** continue group 3 from wip/crater-hill after W6 is merged; record the rim failures as results with their numbers.
