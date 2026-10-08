---
name: feedback-merge-then-verify-separately
description: Never chain git merges and the verification launch in one shell command; check for conflicts and a clean tree first
metadata:
  type: feedback
---

Do merges and the verification launch as separate steps: merge, check `git status` has no `UU`/unmerged paths and no conflict markers, then launch `vrun_<hash>.sh`.

**Why:** on 2026-10-07 two merges in a chained command conflicted (ledger JSON, a test module list) and the same command launched the verification on the conflicted tree twice; both runs had to be killed and their logs discarded.

**How to apply:** `git merge … && test -z "$(git status --short | grep -v '^??')"` before any launch; resolve conflicts (ledger end-of-list conflicts keep both entries; module lists keep both `mod` lines), commit, re-check, then launch. See [[feedback-no-merge-during-vrun]].
