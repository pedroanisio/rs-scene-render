---
name: feedback-no-merge-during-vrun
description: Never move the realism worktree's HEAD (merge/checkout) while a verification script runs in it; the script only checks HEAD at start
metadata:
  type: feedback
---

Never merge, checkout or reset in the realism worktree while a `vrun_<hash>.sh` verification is running there.

**Why:** the script checks HEAD once at the start, then every later step (tests, clippy, probe) runs against whatever the working tree is at that moment. On 2026-10-07 the run for f6750db silently tested 464d3a0 and 27cb496 after I merged mid-run; the log looked clean but was not a record of f6750db.

**How to apply:** queue merges until the running vrun prints DONE (or abort it deliberately and note it), or merge on a temporary branch in a separate worktree. A chained script (`chain_<hash>.sh`) that waits for the previous DONE before starting is fine only if HEAD is not touched in between. See [[render-verification-notes]] and [[shared-machine-agents]].
