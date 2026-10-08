---
name: engine-not-staging
description: "User wants the engine to simulate cause and effects; do not improve results by restaging scenes (camera, lights, authored timed events)"
metadata:
  node_type: memory
  type: feedback
  originSessionId: ffccf0e1-cf7c-4ae2-b4f0-5d18f8b755ca
  modified: 2026-10-03T23:28:07.576Z
---

On 2026-10-03, while I was tuning lighting and camera of the cinematic impact example to make frames look better, the user (Pedro) stopped me: "objetivo não é fazer uma encenação, e sim um motor que execute a simulação e seus efeitos".

**Why:** the goal of the realism work in rs-scene-render is a causal simulation engine (the impactor causes crater, wave, ejecta, plume), not a nicer-looking authored scene.

**How to apply:** when asked to make simulations "realistic", work on solvers, coupling and renderer capabilities. Scene authoring is only a test workload. My interpretation: neutral test scenes for measuring are fine; hand-timed effect events and look tweaks are not progress.

Related: [[shared-machine-agents]]
