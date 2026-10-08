---
name: saturno-work-queue
description: "My (Saturno, session f3931a, listed as rs-scene-render-47) branches, delivered items and the queue Urano gave me, as of 2026-10-08; read after a context reset"
metadata:
  node_type: memory
  type: project
  originSessionId: 6e5c014b-ac81-48e3-835f-be0323e7c6a6
  modified: 2026-10-08T00:00:00.000Z
---

State on 2026-10-08 (peer messages from Urano `rs-scene-render-a9` are the only instructions; Mercurio `-e6` render/surface module, Netuno `-ee` physics, owner of sr_3d::occupancy):

- Merged by Urano into main: V.1 (voxel asset reader/SRVOL/voxelizer/schema/VOX1-7), fix/vox-walk, docs/follow-ledger (follow milestone).
- State 2026-10-08 (user order 11:00: PAUSED, all my work integrated in origin/main): fracture-stress, manifold twist (ec0a3c5), fixtures all merged; my branches deleted; worktree saturno-pyro REMOVED by Urano. On resume (Urano says so): `git -C /home/pals/src/rs-scene-render worktree add /home/pals/src/rs-scene-render/.claude/worktrees/saturno-<tema> -b feat/<tema> origin/main` (target recompiles). Next: feat/pyro-vapor (3.5, approved design: oceano manda a massa, `evaporated` kg no Frame, pyroVapor/vaporFraction, oracles 1-8 incl. closure with condensation, order A pyro channel -> B ocean sink -> C schema/eval -> D render 2 VolumeDraw by Mercurio), red first. Friction scale item closed (ratio covers it). No builds/tests/branches while paused.
- Open: W6 (Netuno reads fracture@mode/strength, builds via sr_eval::voxels::stress, registers World3::with_stress, deletes E24 + stress_refused.rs and the E24 exception in voxels/corpus.rs); twist reader and friction-with-joint scale are known limits; pyroBlast coupling to bodies/ocean not done; `sea_cost.rs` only if asked.
- Rules learned: ci profile, sr-build queue, local target; never `pkill -f`; gate commits on clippy exit; release builds wrap on overflow (use range tests); NEVER put sr-gpu in verify-ci (GPU only via sr-gpu queue; I slipped once and disclosed it); before a full verification `df -h /home` and below 40 GB free delete only my own target/debug and target/release (and target/ci/incremental); do not rewrite commits Urano already reviewed (new commits on top; keep rebased copies on separate branch names); commit messages end with the Co-Authored-By Claude Sonnet 5.5 line; Urano does not touch a worktree HEAD during a verification; use `git stash push -u -m <tag>` + apply <sha> + drop when a branch switch is blocked (never bare stash); the worktree is /home/pals/src/rs-scene-render/.claude/worktrees/saturno-pyro and the shell cwd can reset: always cd there; check `git branch --show-current` before editing (I once edited on the wrong branch).
