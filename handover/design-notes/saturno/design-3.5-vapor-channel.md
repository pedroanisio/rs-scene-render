# 3.5 Vapor channel — design note (Saturno, approved by Urano 2026-10-08)

Status: approved, NOT started. Branch to open on resume: `feat/pyro-vapor` from origin/main in a new worktree
(`git -C /home/pals/src/rs-scene-render worktree add /home/pals/src/rs-scene-render/.claude/worktrees/saturno-pyro-vapor -b feat/pyro-vapor origin/main`).
Order: A (sr-sim pyro channel + `pyroVapor` with a given mass) -> B (ocean sink + per-step record) -> C (schema/rules/eval) -> D (render: two VolumeDraw, Mercurio). Red first, one commit per finding.

## Survey of what exists (2026-10-08, code at main b302f30)
- Nothing about vapor/heat of rigid bodies or ocean. pyro.rs `State` has `density` (volume fraction of solids), `temperature` (K), MAC `velocity`, `solid`. Buoyancy depends on temperature only.
- Step order (pyro.rs `step_profiled` ~1439-1602): validate; working copy; voxelize obstacles; boundaries; advect (decay/cooling at ~1677; MacCormack in maccormack.rs shares the traces); sources/heated sources/impulses via `inject` (~2296); blast pulses; forces; boundaries; project; blast piston flow (`inject_piston` ~2352 carries density/temperature in a scratch state via a second `advect` with `calm`, ~1543-1555: a new channel must be swapped in there too).
- `Inputs.heated` / `heated_impulses` (~502-505) convert heat into expansion dT/(T dt) (ideal gas, constant pressure): the hook for an energy-driven source.
- Follow window (`Follow`, `follow_decision`, `lost`): vapor must count as "there is smoke" and in `lost`.
- Determinism: rayon chunks HEAVY=256/LIGHT=8192, serial reductions, noise keyed by cell and step; `pyro/determinism.rs` pins 9 golden hashes over density, temperature, velocity, solid (1/2/8 threads).
- Budget: `288 B/cell + 8192` workspace (pyro.rs ~1372); export budget hard-coded for 5 channels (~1060).
- SRVOL (sr-volume/src/lib.rs): up to 64 named grids, each with its own transform/background; a `vapor` grid needs no format change or version bump. Pyro export writes density, temperature, velocity.x/y/z (pyro.rs ~1057-1139); `volume_key` (pyro/key.rs) hashes all channels.
- Renderer: `sr-gpu/src/render_three.rs:959-1135` builds ONE `Medium` (one density grid + optional temperature + one `Optical{albedo,...}`) per volume; native pyro channel names hard-coded at ~970. Albedo is per volume, from the first `<medium>` child.
- Ocean (sr-sim/src/ocean.rs): `Cell{depth, velocity}`, `Frame{time, cells, bed}`, water in a column = depth * cell_size^2; solver conserves water; sources/forcings in ocean/impulse.rs and `Forcing` (~134). Pyro and ocean do not read each other today (only shared collider geometry).
- Schema: `pyroType` (xsd ~4762), `pyroSource`, `pyroImpulse`, `pyroBlast`; `mediumType` (~4372, one per object3D); rules PYC1-PYC6 in sr-model/src/rules.rs.

## Design
Channel: a third advected scalar `vapor` in the pyro State, kg of water vapor per m^3 (not a volume fraction like `density`). Own decay `condensation` (1/s); condensed mass leaves the balance but is recorded. Rides the same advection trace.

Mass source (v1: an event, not a continuous flux): impact energy E (same 0.5*m*U^2 the crater uses) times `vaporFraction` f. Heat per kg L = c_w*(373.15 - T_water) + L_v with c_w = 4186 J/kg/K, L_v = 2.256e6 J/kg, T_water from the ocean (288 K default, said). Requested mass m = f*E/L; effective mass = min(m, water available in the footprint). The ocean is the authority; the vapor is exactly what it gave back. Continuous hot-gas-over-water flux is V2.

Entry into pyro: new child `pyroVapor` of `pyro` (time, place or crater@source, energy, vaporFraction, duration, spatial profile gaussian with radius, smooth time profile, ocean=IDREF). Each step injects m_dot*dt over the free cells of the footprint into `vapor`. Temperature: vapor leaves at 373 K (reuse the `heated` hook: dT -> expansion at constant pressure). Expansion: rho_v = P M / (R T) = 0.588 kg/m^3 at 373 K, 1 atm (1 kg -> 1.70 m^3), entering as a target divergence of the projection: the same model as the blast piston (incompressible: only the volume, no pressure wave). Needs boundary="open" (PYC5). Must follow `inject_piston` swap and the Follow window.

Ocean coupling (decision i, Urano): mass sink in the ocean (depth removed per footprint cell, momentum-neutral) applied at the canonical step, and the ocean records per step in `Frame` the removed mass `evaporated` (kg). Pyro reads it via `ocean.at(t)` for its step's m_dot => conservation by construction, including the cut when the footprint dries out. Evaluation order ocean -> pyro in the graph is stated and tested (a document with pyro before the ocean in the text gives the same result). Rejected: both sides evaluating the same analytic formula (does not see the water shortfall).

Render (decision ii): sixth SRVOL grid `vapor` only when the channel exists; two VolumeDraw of the same transform, each with its own Medium/albedo (`<medium channel="vapor" albedo=...>`); no shader or GPU-format change. Mercurio's part, after A-C.

Names (decision iii): `pyroVapor`, `vaporFraction`.

## Oracles
1. injected vapor mass = f*E/L to 1e-12 with the stated constants.
2. conservation: sum of water removed from the ocean = sum of vapor injected to 1e-12; with a dry footprint the cut is counted (injected = removed < requested).
3. a scene without the channel is bit-identical: state, export, `volume_key` and the 9 goldens of `pyro/determinism.rs` (1/2/8 threads) unchanged; the channel is an empty Vec when not declared.
4. vapor travels with the blast (piston swap) and with checkpoints (replay = fresh).
5. expansion: vapor volume = m / rho_v to 1e-3 of ideal gas.
6. balance: vapor in the grid + lost by the window + condensed = injected.
7. determinism of the vapor channel on 1/2/8 threads.
8. (Urano) with `condensation` > 0: recorded condensed mass + vapor in grid + lost = injected to 1e-12 (closure with decay).

## Cost
+8 B/cell resident, +8 working copy, +8 blast scratch, only when declared (workspace budget 288 -> 304 B/cell only then; export budget 5 -> 6 channels only then). One more shared trace per advect: measure, target < 10% over the same scene without vapor. Ocean: one term per footprint cell.

## v1 limits to state in the SREP
No latent heat returned on condensation; vapor has no weight of its own (buoyancy follows temperature only); one event per source; no boiling by contact of a hot body (bodies have no temperature); expansion as target divergence is incompressible (volume only, no pressure wave).
