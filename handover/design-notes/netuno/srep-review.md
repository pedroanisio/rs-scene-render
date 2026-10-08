# SREP review at fa63e5d (srep-0000-cinematic-impact.md, 2996 lines). Lines are of that file.

Read in full: 757-2000 (particles, ocean, crater, coupled solvers), 2296-2373 (light through water), 2545-2775 (inventory), 2856-2996 (conformance, status); the rest (volume assets, pyro, light grids, fracture, mesh sequences) skimmed for the three questions and grepped for temporal words, measured numbers and rule ids. Nothing edited.

## (a) Provisional or stale statements

A1. L1439-1440 "The momentum given during the last whole step is kept for a reaction on the body; nothing is applied to the body yet."
    Stale: `bodyCoupling="full"` applies it (L1970-1980).
    Proposal: "The momentum given during the last whole step is kept per body and read by the group for the reaction on the body (`ocean@bodyCoupling="full"`, below)."

A2. L1442-1447 "A body is kinematic with infinite mass: the momentum and energy it gives the water are not taken from it".
    True only with `bodyCoupling="none"`; with `buoyancy`/`full` the body is loaded.
    Proposal: prefix "With `bodyCoupling="none"`,"; add "with `full` the body gets back what it gave the water, one canonical step late (below)."

A3. L1431-1440 describes the relaxation of the water toward the body's velocity and the lift of the column by the thickness as THE model; since bedResponse (default `depthFiltered`) both are the `hydrostatic` mode only (L1136-1230).
    Proposal: open the paragraph with "In `bedResponse="hydrostatic"`:" and add one sentence pointing to the filtered lift and the form-drag push.

A4. L1454-1459 "Measured: ... a falling sphere ... 0.22, 0.66 and 1.11 ... 0.637, 0.661 and 0.660".
    Phase-1 numbers of the hydrostatic mode, with no mode, commit or date. Proposal: "(hydrostatic response, test ocean_bed)" with the commit that first had them, or move to the ledger.

A5. L1459-1461 "Resident memory is charged 24 bytes per cell for the bed vectors and 72 more with bodies".
    Now also +12 per cell with owners, +32 with pushes (`Spec::body_push`), and the depth filter charges `24*cells + 40*window` bytes to `meshMemoryMiB`. Proposal: replace by the full list: 24 (bed vectors), +72 (bodies), +12 (owners), +32 (pushes), filter 24 per cell + 40 per window cell against `meshMemoryMiB`.

A6. L1126-1134 "What it does and does not give": "... the combined far wave ... is not monotone (0.0326, 0.0390 and 0.0276 ...) ... that wants the horizontal reaction on the body, which is not here."
    Stale on three counts: measured in hydrostatic mode with the cavity at once; the reaction on the body (`full`) and the drag exist; the sea sweeps (L2895-2924) now say what grows. Proposal: delete the combined-far-wave sentences; keep the cavity-alone numbers with their mode, or move all of it to the ledger and point there.

A7. L1215-1221 "What the sphere crossing deep water now makes": "with the cavity and `bodyCoupling="full"` it is 9.92 m and 9.48 m, the ring of the cavity".
    Stale since the cavity forms over the law's time in a wider ring (0.86 m at 2.5 s, e425a1d / 7e9264c). NOTE: the 0.86 m was measured with the former kernel (before 7e9264c) and has not been re-measured at 1.5 m since. Proposal: replace by "with the cavity formed over the law's time, 0.86 m at (24.8, -3.8) (before it formed at once: 9.49 m at (0.8, 0.8); both with the first kernel); re-measure with the current one and give the commit; the table of the sea sweeps below gives the current numbers".

A8. L1110-1124 "Where the limit bound, and the profile that removes it": a history of the first version (old kernel, old sweep numbers 6.41/8.03/7.30, 5.43/8.69/7.81).
    Proposal: keep two sentences of current rule (disc `R = sqrt(3V/(pi d))`, 90% is a protection) and move the history and the old numbers to the ledger milestone "The cavity of a water entry has the law's depth, not twice it" (already there). The old numbers are repeated at L1116-1117 and L2912-2914 (see A12).

A9. L1075-1076 "Without `source` nothing changes, so a scene with bodies in `colliders` stays as it was until it opts in."
    See B1: false for the response since bedResponse defaults to depthFiltered. Proposal: "Without `source` no cavity is made."

A10. L1805 "Limits of this first version" and L1809-1810 "the asymmetry of an oblique impact is expected to show in the ejecta".
    The asymmetry is implemented in the ejecta (L1879-1881). Proposal: title "Limits", and "the asymmetry of an oblique impact shows in the ejecta (below)".

A11. L1861-1889 (Ejecta from an impact) does not say where a particle is born now: "each at its launch distance from the impact point in the tangent plane of the contact" and "The crater's place and axis are the owner's at the impact".
    Since 68e9dd2 (merged in 847d9d8) each is launched from the nearest point of the owner's surface, mapped by the crater as grown at its own instant, clear of it by its collision radius (collisionRadius x (1 + scaleVariance) + collisionTolerance), along the owner's geometric normal. This paragraph was never updated (only the ledger). Proposal: add after L1885: "Each particle starts from the nearest point of the owner's surface to the contact point (the contact point lies inside the surface by how far the body went on before the contact was found), taken through the crater's map at the progress of its own instant (the rim has risen over places that were level by then), and clear of the surface by the radius it collides with; a particle born at the old height was under the rising rim and was pushed through the ground: with a collision radius of 0.17 m, 678 of 4000 ended below it, none now."

A12. L2883-2892 (land scene limit bullet): "It is not in the scene because it stops the particle solver, with "more than 16 collisions in one particle step", at 2 s for the heaviest rock ... an ignored test keeps the case. The contact radius is ... with 0.17 m 754 of them end under the pit".
    Both defects are fixed (68e9dd2; a00bd2d test of the 270000 kg rock with friction passes; the ignored test of the sea team passes too when run with --ignored). Proposal: replace the last two sentences by: "The heaviest rock of the sweeps with friction 0.7 and restitution 0.15 runs to 5.9 s (the stop at 2 s was ejecta born under the rising rim, fixed by 68e9dd2, regression test crater_ejecta_ground), and with the rocks' own radius 0.17 m none of the 4000 ends below the ground (754 did). Below about 0.02 m, smaller than the facets of the 1 m ground mesh differ from the true bowl, about 1500 still do." and drop "an ignored test keeps the case" (the test can lose its #[ignore]).

A13. L2904-2907 "The highest surface anywhere does not grow with the mass at 60 degrees, nor with the speed of a vertical plunge, now that the cavity forms over the law's time in a wide ring ... recorded in an ignored test ... the cause is the cavity model's, measured by its owner."
    Stale: with the disc of the law's depth (7e9264c) the highest surface grows in both modes (by mass 4.72, 4.74, 8.22; plunge by speed 4.53, 5.82, 8.07) and the four tests pass. The cause (the kernel's peak about twice the law's depth against 90% of the layer) is in L1110-1124 / the ledger. Proposal: delete the sentence and the ignored-test remark; say the highest surface is asserted again (with the lightest pair, 4.72 and 4.74, within 0.5%) if the sea team re-enables it.

A14. L2901-2903 "...three points (40, 60 and 80 m/s; 20 000, 40 000 and 60 000 kg) whose cavity the law makes shallower than the water".
    The reason the three points were chosen (cavity depth under the layer) is now known to be the wrong criterion (A13/A8: the kernel peak, not the law's depth, bound). Proposal: say "chosen because the cavity of the first version was limited at the heavier points".

A15. L1006-1009 "Implementation status" of the ocean section lists solver, waves, impulses, bathymetry, replay, meshes, rendering and foam; omits colliders, bodies, water entry, depth response, form drag, per-body samples, pressure, splash, coupling.
    Proposal: add them in one sentence with the rule ids OCN6-OCN13.

A16. L1019-1038 (ocean attribute table) lacks `bodyCoupling`, `bodyDrag`, `bedResponse`, `splash` (they appear only in the XSD inventory and in prose). Proposal: add four rows.

A17. L2658-2701 and L2748-2772 (inventory): the executable XSD has `oceanType@splash`, `particles3DType@gas`, `craterType@capture`, none of which is in the inventory tables (the diff of the XSD against the tables, script in the scratchpad). Proposal: add three rows (and `credit`, `license`, `proxy`, `sha256` of meshSequenceAssetType if `assetProvenance` is not meant to cover them).

A18. L938-943 "Rules `P3D1`-`P3D6` cover ..." and "invalid/p3d1 through p3d6".
    P3D7-P3D10 (burst@crater) and P3D11 (gas) exist. Proposal: "P3D1-P3D11" with a sentence for each of P3D7-P3D10 (burst from a crater) and the corpus names p3d7-derived ... p3d10-angle.

A19. L1256-1258 "OCN1-OCN4 enforce ...; OCN6 and OCN7 enforce what `colliders` may name". OCN3 is never named; OCN8/OCN9 (bodyCoupling, bodyDrag), OCN10-OCN12 (waterImpulse@source), OCN13 (splash) are in separate paragraphs. Proposal: one list OCN1-OCN13 here, one line each. Also OCN9 now reads "bodyDrag belongs to an ocean with colliders" (it was "with bodyCoupling" until fdfb0b0's predecessor 4308a71; check any copy of the old wording in the corpus README or tests).

A20. L1709-1714 "CRT1-CRT5 ... with CRT6 to CRT8 for a crater from an impact ... crt1 through crt8-self": CRT9 (capture) exists (L1735) and has a corpus case. Proposal: "CRT6 to CRT9" and "through crt9...".

A21. L2993 "**the final 31-rule scorecard remain pending**": the number of rules has changed (OCN1-13, CRT1-9, P3D1-11, PYC1-4, VOL1-10, ...); the "31" no longer corresponds to anything countable. Proposal: state the count at a commit (script: count assert ids in the .sch) or drop the number.

A22. L2938 "Not measured: the cost of that scheduling per frame." Still true; the ledger block "Phase budget" (53a9b6b) has the per-frame cost of the whole ocean side but not of this scheduling. Proposal: leave, add "(whole-frame cost of the ocean side: ledger milestone Phase budget)".

A23. L1703-1707 "Remaining work" of the crater section, and L2271 (mesh sequences "allocation bounds remain pending"), L2217 ("unverified"): not touched by today's merges; kept as they are, listed for completeness.

## (b) Contradictions between sections after today's merges

B1. Default of `bedResponse` vs "nothing changes": L1075-1076 (A9) and L1409-1411 ("without the attribute nothing changes, bit for bit", fine: that one is about `colliders`) against L1140 (depthFiltered is the default, which changes every existing ocean with colliders). Proposal: A9 plus, in the colliders paragraph, "an ocean with colliders answers by `bedResponse` (default `depthFiltered`; `hydrostatic` reproduces the first version bit for bit)".

B2. The buoyancy measurements (L1959-1968: ball of 2094 kg, within 3 cm of its draft from 8.75 s / 24 s; L1976-1978: 13%, 10%, 4.8%) were made in the hydrostatic response (the tests that make them pin it). The text never says so, and the default is now the filtered one, where the lift under the ball is attenuated. Proposal: add "(`bedResponse="hydrostatic"`)" to both.

B3. L1979-1980 "The water's momentum exceeds what the bodies are credited with giving it by a few per cent that the per-body samples do not attribute." Contradicted by `BodySample::pressure` (L1200-1213, fdfb0b0) which attributes that term; and "the bodies are credited" is now `impulse` plus `pressure`. Proposal: "... by the pressure of the water on the bodies, which the per-body samples now carry as `pressure` (Pressure of the water on a body); whether `full` reads it is the coupling's choice". Needs the Saturno side: I did not check whether 6d3850d (or any commit) wires `pressure` into the load; if it does, say so here.

B4. L1754-1756 "In the ocean scene `full` already gives the water the rock's horizontal momentum, so the rock stops on the bed with or without it (hydrostatic: 0.4 and 0.5 m ..., 2.243 and 2.238 m; depth-filtered: 0.1 m and 9.8 m, 2.274 and 2.261 m: with the depth filter the free rock rolls out of its crater)".
    In the filtered mode `full` gives the form-drag push, not the relaxation, and after the cavity change the test prints 0.1 m and 8.5 m, 2.097 and 2.083 m (impact_scenes, this branch). The sentence is wrong in its premise for `depthFiltered` and its numbers are old. Proposal: re-measure with `cargo test ... in_the_sea_the_rock_that_reaches_the_bed_rests_in_its_crater -- --nocapture` and rewrite with the commit.

B5. ERROR OF MINE. L1114 ("Measured on the impact-ocean scene (1.5 m cells)") and the same words in the ledger milestones "The cavity of a water entry has the law's depth, not twice it" and in my report: the numbers (wanted 266.5/367.5/862.3, limits, waves 6.41/8.03/7.30, 4.72/4.74/8.22, ...) came from the acceptance tests, which run the sea on 3 m cells (`coarse_sea`, `cellSize="1.5"` replaced by `"3"`, as L2897 says), not on the authored 1.5 m scene. The only 1.5 m measurements I made are the crest probes at 2.5 s (10.62 -> 0.16 m, 9.92/9.48 -> 0.86 m) and the cost table. Proposal: correct L1114 and the ledger to "on 3 m cells (the acceptance sweeps)"; the conclusion (the kernel's peak, not the law's depth, bound) is unaffected but the limit numbers at 1.5 m were not measured.

B6. L2909-2924 table (far wave, highest surface, crater by speed) — far-wave and highest-surface values are those before 7e9264c (they include 6.41/8.03/7.30 and 5.43/8.69/7.81) while the crater values are unchanged. The text says "the sea's amplitudes change with the cavity's kernel", i.e. it knows it is a dated measurement, but the kernel changed the same day. Proposal: re-run `tools/impact_sweeps.sh` on fa63e5d and replace the table, with the commit.

B7. L1128-1130 quotes the far wave of the cavity alone as 0.0050, 0.0121, 0.0211 (formed over time) and, as history, the instantaneous ones; L2909-2911 gives another "far wave" (0.95, 1.32, 1.85 m) for the impact-ocean sweeps: different scenes and metrics under the same words "far wave". Proposal: name them ("far wave of a 2 m body in water 100 m deep, 20-40 m ring, cavity minus no cavity" against "far wave of the sea in the authored scene").

B8. L1040-1050 (default budgets, "400 bytes per cell") against L1404 (256 and 400 bytes per cell, measured peak 158 and at most 278): two statements of the same charge, one with "measured"; fine, but the 198 MiB figure is first order only for 518,400 cells; no contradiction, listed to be checked at the next change of the charge (A5).

## (c) Numbers without date or commit (those worth fixing; the rest of the 44 "measured" lines are from earlier work)

Mine (all in the ledger with the commit of the entry; the SREP lacks it): L1114-1123 (cavity limit and sweeps), L1126-1134 (cavity far wave), L1176-1178 (cost per step, load 8-12), L1195-1198 (drag 104.05 vs 104.72, "the relaxation gave about four times that": that factor was derived from the formula, not measured; say so), L1208-1213 (pressure residuals), L1215-1221 (crest before/after), L1454-1459 (phase 1), L2909-2924 (sweeps; Saturno).
   Proposal for all: a trailing "(ledger: <milestone name>, <commit>)" and, for timings, the load.

Saturno's: L945-971 (particles@gas: "Measured cost (release, one core ...) 0.77 s ... 1.5 microseconds per particle step"; "two to five measured"; no commit); L985-1002 (splash: 80.7815 m3, 37.2/80.8/148.2 m3, 3879 of 4000, 121, 2.44 m3, momentum to 1e-6); L1749-1756 (capture energies); L1930-1937 (internal edges: "-0.103, -0.045, -0.994", "7.35 m", "0.42 m", "up to 0.15"); L1959-1978 (buoyancy); L2925-2938 (everything in one ocean: 11 of 3000, 0.20 m3); L2939-2943 (dust 19.19, 19.07, 19.08 K).
Mercurio's: L2337-2371 ("Measured against brute force ... 0.604 against 0.613 ...", "394 pixels", "0.00663 to 0.00909"); L438-490 (light grids: dB figures); L918-923 (UHD frame 44.977 s, 573,220 KiB RSS on RTX 6000 Ada: single observations on an earlier build, no commit, the text itself says they are single observations).
Older: L550-553 (pyro peak resident memory, "measured on a loaded machine"), L746-754 (hero plume impulse), L1381-1383 (2.8-3.0 on 256 x 256), L1404 (158 and 278 bytes per cell), L1450-1452 (crater deeper than the water).
Proposal: one convention for the whole document: each measured figure carries "(commit abcdef0, date, machine and load)" or points to the ledger milestone that has them; where the commit is not known, say "measured before 2026-10-04, not repeated".

## Smaller things seen

S1. L2312-2314, L2346, L2362 (Mercurio) use "now" / "Before these changes" / "What changes in images": narrative of a change, not a contract. Proposal: state the contract in the present ("a path that crosses a refracting surface and reaches nothing sees the visible dome") and keep the before/after in the ledger.
S2. L1614 "Rigid replay now admits ...", L1627 "Particle collider references ... now use", L2190, L2197: same "now" wording.
S3. L2986 "Runtime interfaces not yet integrated remain explicitly pending in that ledger" and L35 "This draft is not a claim that all proposed features already execute": consistent with each other, no change.
S4. Rule ids check (script): defined in the .sch and not mentioned anywhere in the SREP: OCN3, VOL2, VOL3, VOL4. Proposal: mention them where the references and sources they enforce are described.
