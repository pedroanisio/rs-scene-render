# Closing report: acceptance of the two impact scenes (Saturno)

State: `phase2/cosim` at 98bab6c on 90ce0f1. Pressure-in-the-record commit parked at 5bbab1b (passes; held for the exact form of the pressure).
All figures below are from the commits and dates named; none depends on the load unless it is a time.

## 1. What the scenes are

- `examples/cinematic-impact/impact-land.scene.xml`: rock of 2 m radius, 90 478 kg, 100 m/s at 60 degrees on soft rock, metres and seconds, `fixInternalEdges`, `capture`; crater, smoke (48 x 56 x 48 m at 1 m cells) and 4000 ejecta (80 % of the crater's mass) are consequences of the contact. The ejecta declare restitution 0.15, friction 0.7, collision radius 0.17 m (e22458b).
- `examples/cinematic-impact/impact-ocean.scene.xml`: the same rock into a second-order ocean of 20 m (192 m at 1.5 m cells), `bodyCoupling="full"`, `waterImpulse source`, crater in the seabed with `capture`. No smoke, no ejecta (born on the seabed under 20 m of water).
- Neither scene authors a time for any effect; a test checks it.

## 2. What the tests assert (`crates/sr-eval/tests/impact_scenes.rs`, `cargo test -p sr-eval --test impact_scenes`; no GPU)

Land: nothing happens before the contact; crater, dust, heat and ejecta (mass, reach) grow with speed, mass and angle; the arrival is as authored (100.02 m/s, 86.63 m/s along the normal for 86.60); dust is what the law gives; the axis is the ground's; an oblique impact carries the ejecta downrange; ejecta are 4/5 of the crater's mass; ejecta settle (3748 of 4000 under 0.2 m/s at 5.9 s, none through the ground) for the authored rock and for the 270 000 kg rock (1861, none through, 29 past the 80 m edge); same bits in any order, fresh evaluator, replay from the first checkpoint.

Sea, run twice, with `bedResponse` written into the document as `depthFiltered` and as `hydrostatic`: nothing happens before the impact; water conserved to the last cell; crater radius and depth grow with speed and mass; far wave (ring 20 to 40 m from the entry) grows with speed and mass over the whole sweep and over three lighter or slower points, for 60 degrees and for a vertical plunge; highest surface anywhere grows with speed at 60 degrees; the rock rests in its crater with `capture`; same bits in any order, fresh evaluator, no checkpoint kept.

Everything in one ocean (4 m of water, bed and rock as colliders, full coupling, cavity, 3000 ejecta with splash, filtered): no failure, water conserved to 1e-9, same ocean, rock and particles to the bit in any order.

Orders only. No absolute value is asserted anywhere in the sea tests (the amplitudes change with the cavity's kernel).

## 3. Recorded, not asserted (ignored tests, run by `tools/impact_sweeps.sh`)

- Highest surface anywhere by mass at 60 degrees, and by speed and mass of a vertical plunge (by mass 4.72, 4.74, 8.22 m: the first pair is 0.4 % apart).
- Every sweep by angle (30, 60, 90 degrees).
- The sea by speed/mass/angle crater values.
- The dated table of fa63e5d + e22458b (3 m cells, load1 13 to 15; deterministic): far wave by speed 0.91 / 1.51 / 2.30 m (depth filter) and 0.89 / 1.54 / 2.31 m (hydrostatic); by mass 1.14 / 1.51 / 2.71 and 1.18 / 1.54 / 2.76; wave of the crater alone by speed 0.006 / 0.012 / 0.021 m against 0.16 / 0.27 / 0.40 m.
- Rock at 6 s with and without `capture`: 0.1 and 8.5 m (filtered), 0.6 and 0.7 m (hydrostatic).

## 4. Limits the scenes show (stated in the SREP, not hidden)

- Ocean scene: no smoke (the smoke solver has none inside water); no ejecta in the scene (with 20 m of water at most 34 of 3000 ejecta of the heaviest rock reach the surface, 3078153).
- Friction 0.7 is the engine's choice (about tan 35 degrees), not a measurement.
- Capture is a model of the engine (mean force of a penetration), not a published law; the captured rock rocks slowly about 0.3 m (limit of the rigid model).
- The residual of the water's momentum after the impulse and the pressure credit is 7 % (depth-filtered) and 3 to 11 % (hydrostatic) in a closed basin with a ball; its decomposition is being diagnosed and the credit is recorded, not applied to the body.
- Splash in the authored sea is exercised only with a raised seabed (the 4 m, 1 m variants of the tests and of the cost runs).
- Not verified here: GPU rendering of either scene with the splash, the cost of the aggregation/pull/read separately (hidden by the particles that leave the simulation), the first-reference values of the Holsapple and ejecta constants against the papers (the SREP says they are to be checked), UHD frames.

## 5. Defects found while closing, all fixed

- Ocean read the splash of a step before the particles had reached it (frame failure for times off the canonical grid): the emitter is computed to the end of the ocean's step (613e87f).
- Ocean that runs before the particles (group) read an empty splash before any emitter had registered, and its steps differed on a replay; and the particle driver read a rigid world that answered with a problem as one that stood still (ejecta differed by up to 12 m by the order of requests): the ocean asks for the emitters, an unregistered emitter is not an empty splash, a rigid-world problem fails the fixed step (6d3850d).
- The test that discarded the sea's checkpoints named a coupling the scene does not have and discarded nothing (6476eca).
- `cargo fmt` ordering of two module declarations (59e85f1).
- Pressure of a step differed on a replay (Netuno; fixed in 4c102b8, confirmed by 5bbab1b's tests).

## 6. Cost

Scheduling of the particles by the ocean: within noise on the authored sea (0.058 to 0.062 s per frame against 0.057 to 0.071 before; 109 to 112 against 108 to 109 MiB), within noise with 3000 ejecta, and 12 to 19 % cheaper with 9131 of 21 000 falling in (3078153, 2026-10-05, load1 6 to 13).

## 7. Open on my side

Rebase the pressure branch when the exact form lands, confirm the balance tests with the exact values, and adjust the SREP sentence (hash and residual) in the same small commit. Re-run `tools/impact_sweeps.sh` if the cavity kernel or the response changes again.
