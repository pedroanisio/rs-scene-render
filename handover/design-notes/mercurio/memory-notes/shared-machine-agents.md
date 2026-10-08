---
name: shared-machine-agents
description: This machine runs many agent sessions at once; GPU jobs go through the sr-gpu queue and repo work goes in a separate worktree
metadata:
  node_type: memory
  type: reference
  originSessionId: ffccf0e1-cf7c-4ae2-b4f0-5d18f8b755ca
  modified: 2026-10-03T23:28:12.477Z
---

Observed 2026-10-03 on the rs-scene-render machine (8 cores, one RTX 6000 Ada):

- Many Codex and Claude sessions run concurrently (film production fleet plus an engine maintainer). Another agent may commit or switch branches in the main checkout `/home/pals/src/rs-scene-render` while I work; use a git worktree under `.claude/worktrees/` (`.claude/` is in `.git/info/exclude`).
- Every GPU command (scene-render render/encode, nvenc ffmpeg) must go through `~/.local/share/scene-render/bin/sr-gpu <command>`; set `SR_AGENT=<alias>` and `SR_GPU_SHORT=1` for jobs under about two minutes. `sr-gpu --status` shows holder and queue.
- Peer Claude sessions are reachable with SendMessage, but their names can include a prefix (for example `Session ID: Saturno`); copy the name exactly from ListAgents.
- Session names as of 2026-10-07 09:05 restart: Urano (me) = rs-scene-render-a9 [2c80ed]; Mercurio = rs-scene-render-20 [c8cab0]; Saturno = rs-scene-render-47 [9f5dad]; Netuno = rs-scene-render-ee [fc2069]. Names change on every restart: run ListAgents and ask aliases.
- Disk is tight (about 60 GB free, a full `target/` is about 31 GB): build with `-p <crate>` rather than the whole workspace.
- Build rules from 2026-10-06 (machine throttled, load 13-19): tests and clippy use `--profile ci` (inherits release, no LTO, 16 codegen units, binaries in target/ci); full builds or unfiltered `cargo test -p <crate>` go through `SR_AGENT=<alias> ~/.local/share/scene-render/bin/sr-build <command>` (`--status` shows the queue); outside it only small incremental builds and single tests with `CARGO_BUILD_JOBS=2`. Network: GitHub (raw.githubusercontent.com, api.github.com) is reachable from the host; crates.io returns 403. Sample .vox files (MIT) live in /home/pals/assets/vox-samples with a README of sources and hashes.
- Never build another checkout into a worktree's `target/` with `CARGO_TARGET_DIR`: the workspace crates hash the same across worktrees, a stale rlib looks fresh and gives false compile errors (seen 2026-10-06; `touch` the sources to recover). Never `pkill -f` a pattern that appears in your own command line: it kills the shell.

Related: [[engine-not-staging]]
