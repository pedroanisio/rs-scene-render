# Design notes for 3.3 (collapse and deposition of the crater) and 3.6 (granular debris). Netuno, 2026-10-06. Analysis only.

Numbers are from a small program that calls `sr_sim::cratering::crater` (probe law.rs in this folder, not committed) for the authored
rock (90 478 kg, density 2700, 100 m/s at 60 degrees so 86.6 m/s along the normal, soft rock, g = 9.80665) and from the formulas of
`sr_3d::crater` (crates/sr-3d/src/crater.rs:85-131, bump(x) = (1 - x^2)^2 at :194).

## What the code does today (read, not guessed)
- The law (sr-sim cratering.rs:162-197): V = 100.93 m3, radius at the original surface R = 5.121 m, depth 2.794 m, rim crest radius 1.3 R =
  6.658 m (RIM_RADIUS, :121), rim height 0.479 m (0.036 x the rim diameter, :123), formation time 0.551 s, ejecta volume 0.8 V = 80.74 m3
  (EJECTA_SHARE, :125) and a blanket `t(r) = t0 (R_rim / r)^3` with t0 = 0.2899 m at the crest, `t0 = ejecta_volume / (2 pi R_rim^2)` (:196):
  that integrates over r >= R_rim to exactly the ejecta volume.
- The deformation of the ground (sr-eval crater.rs:186, sr-3d crater.rs:100-103): a vertex moves along the axis by
  `progress (-depth bump(r/radius) + rim_height bump((r - radius)/rim_width))`, with `radius = law.rim_radius` and `rim_width = rim_radius - radius`
  (crater.rs:186-194), so the bowl bump reaches out to the CREST. Its volumes: bowl pi R_rim^2 d / 3 = 129.7 m3 = 1.285 V; rim ring
  2 pi R_rim w (16/15) h = 32.9 m3 = 0.326 V. The ground loses 0.959 V net: 4 % less than the law excavates, and the 0.8 V that the
  law says was thrown out is nowhere in the ground: the ejecta are particles that stop on the mesh (friction 0.7, radius 0.17 m, the land scene)
  and are drawn as particles, not as ground.
- Particles interact with the colliders only (sr-sim particles3d), never with each other.

## 3.3 Collapse and deposition
Proposal, in the order I would build it, each behind an attribute that defaults off so that the bits stay (`crater@blanket`, `crater@collapse`):
1. A volume budget as the contract. Excavated V = ejecta in the blanket (0.8 V) + uplift in the rim (0.2 V, the rest). The final topography
   `dh(r)` must satisfy `integral dh dA = 0` when the bulking factor is 1 (ejecta bulk more than rock; make it `crater@bulking`, default 1).
   The kernel above does not meet it (0.959 V removed, 0 added outside): the first piece of work is a bowl and rim profile whose volumes are
   -V and +0.2 V by construction (rescale the amplitudes, keep the bump shape) and a blanket added outside.
2. Deposition as ground elevation, analytically first. `t(r) = t0 (R_rim / r)^3` for `r >= R_rim` (McGetchin et al. 1973, Collins et al. 2005 as the
   law's doc says), truncated where `t < eps` (a millimetre) and renormalised so that the truncated integral is exactly 0.8 V: for a cutoff at 20 R_rim
   the missing tail is 5 % and the correction matters. The elevation goes into the same vertex mapping as the bowl (the surface does not need a new
   mesh); the particles that stop on it are then removed at rest (a settled particle is the blanket, not an extra), or the budget is counted twice.
   An oblique impact: the existing azimuthal factor `1 + b cos(az)` (ejecta.rs header) multiplies the thickness and is renormalised the same way.
3. Deposition from the particles themselves (optional, later): the landing points of the 4000 ejecta splatted into a height field (a smooth kernel
   of two cells, mass over bulk density) conserve the volume by construction, and carry the asymmetry that the launch angles and speeds of Housen and
   Holsapple produce, but give a noisy profile that need not be r^-3 (the empirical law and the launch law are different models). Compare it with
   step 2 as a measurement, not an assertion.
4. Collapse of the transient crater. For a simple crater the final diameter is about 1.2-1.3 times the transient one (Melosh 1989, Collins 2005: I
   am citing from memory and have not read either). Before building anything decide what the law's radius is: Holsapple's calculator gives the
   final (apparent) radius, so multiplying it by 1.25 would double count. If it is final, the collapse is a change of SHAPE over the formation
   time, not of size: the transient cavity is narrower (R / 1.25) and deeper, the rim slumps outward, and `progress` needs separate growth curves for
   radius, depth and rim (the curve is one `curve` today). That is a design for the schema (two or three curves) and wants a reference for the
   transient depth that I do not have; leave it last.

Oracles and tests:
- Volume: `integral (dh) dA` over the whole ground mesh within 1e-6 V of zero (bulking 1), by the divergence of the mesh (sum of triangle areas x
  mean displacement); and `integral over r >= R_rim` of the blanket within 1e-6 of 0.8 V after renormalisation.
- Profile: the azimuthally averaged thickness against `t0 (R_rim / r)^3` to 1e-3 of t0 (analytic version) for a vertical impact; for the particle
  version a tolerance measured, not chosen.
- Determinism and bits: no attribute, same bits (corpus + the crater_impact tests); with it, the same on 1/2/8 threads and after a checkpoint.
- The rigid world feels the raised ground: the sphere that lands on the blanket rests at its elevation (existing rest test with the blanket on).
Risks: the blanket changes the ground the rigid bodies and the ocean see (the ocean's bed is the same mesh): the crater of the sea acceptance scene
is a bed, and a blanket 0.29 m thick under 20 m of water changes the far-wave sweeps; the bodies' contact with a surface that rises outside the rim
changes the impactor's own rest (it would be buried). The cost is the deformation of 51 200 triangles, already paid at each step of the growth.

## 3.6 Granular debris
What the particles with friction give today: a particle that reaches the ground stops (restitution 0.15, friction 0.7, i.e. about tan 35 degrees),
and 3 748 of 4 000 are under 0.2 m/s at 5.9 s in the land scene. But particles do not touch each other, so they lie in ONE layer on the mesh:
no heap forms and nothing flows down a slope that the ground does not already have. What is missing for a bed that slides is a mechanism for
pile-up and avalanche. Three ways, with the cost that I can state:
 (a) rigid spheres in rapier (the engine's own world): 4 000 dynamic bodies, true contact, true piles; cost not measured (the crater and
     the ocean step costs above are of the order of 1.5 ms for the pipeline with a few bodies; thousands of contacting spheres is another regime): measure before
     promising anything; determinism of rapier is good here but the contact count per step makes the log (contact_log) grow.
 (b) a height field of the granular deposit over the ground (a continuum, the same height field as step 2 of 3.3): particles that come to rest add their
     volume (mass / bulk density) to it, and a relaxation moves material downhill wherever the slope exceeds `tan(theta)`, theta the angle of repose
     (`debris@repose`, default off). A scan in a fixed order (row, column) with a fixed number of passes, or until the largest excess slope is under a tolerance,
     is deterministic and costs O(cells) a pass. It conserves volume exactly by construction (moves, never creates) and gives piles at the declared angle.
     The known defect: a 4-neighbour rule gives pyramids (angle depending on direction); an 8-neighbour rule with the slope measured over the true distance
     (`sqrt 2` on the diagonal) is much better but the anisotropy must be measured before asserting 2 degrees.
 (c) hybrid: particles stay particles while they move and are absorbed into (b) when they come to rest (the rest speed already used in the land
     scene's test, 0.2 m/s). This is what I would build: it keeps the flight physics that exists and adds only the bed.
Oracle (the one asked for): pour a volume V at one point of a flat floor. At rest the pile is a cone of the declared angle: `h = r tan(theta)`,
`V = pi r^2 h / 3`, so `r = (3 V / (pi tan(theta)))^(1/3)`; measure the slope on the flank (the fitted plane of the cells at 0.3 to 0.7 of the radius)
along the axes and along the diagonals, and assert the angle within 2 degrees of the declared and the volume to 1e-9 (conservation); the anisotropy of the
stencil is the thing to measure first and the reason the assert says 2 degrees and not less. A second oracle is the angle at which a slab of the same
material on a ramp starts to slide (it must be the same tan theta).
Risks: the deposit as a height field cannot hold overhangs or flow under a body (fine for debris on terrain, not for a rock rolling through a heap); the rigid
world must see the deposit as part of the ground (the same mesh deformation of 3.3), which has the cost above; friction 0.7 and the repose angle are
two numbers for the same physics and should be tied (`tan(theta) = friction` by default).
