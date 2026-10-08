---
name: feedback-no-git-stash
description: Never park work in git stash on this machine: the stash stack is shared by every worktree and session; use a WIP commit on a wip/ branch
metadata:
  type: feedback
---

Urano's rule (2026-10-08): do NOT keep unfinished work in a git stash. The stash stack is shared by all worktrees and sessions on this machine.

**Why:** a stash entry of one session can be popped or dropped by another.
**How to apply:** park work as a WIP commit on a branch (`wip/<topic>`), mark it "not for merge" in the message, and pop nothing. Related: [[shared-machine-agents]].
