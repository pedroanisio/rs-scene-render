---
name: feedback-assertion-behind-a-skip
description: "An assertion that sits after an early return on a missing adapter never ran here (no GL adapter on this machine); put such checks in their own adapter-free test and prove both directions"
metadata:
  node_type: memory
  type: feedback
---

On this machine the GL adapter is absent, so every `gl_*` test in sr-gpu `backends.rs` returns at its first line: a "backends group ran, passed" is no evidence for anything behind that return.

**Why:** I put the list of corpus documents refused by E23 at the end of the GL/native corpus test; it never ran, and the list was wrong (a seventh document). Urano had asked for a list that fails in both directions.

**How to apply:** put CPU-only invariants in a test of their own that needs no adapter, and run both directions myself (remove an item, add one) before saying it is verified. Also: `cargo fmt --all` before `cargo fmt --check` in the same command (a `;` chain does not gate the commit), and never bare `git stash pop`.
