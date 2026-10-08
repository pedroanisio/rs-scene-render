---
name: feedback-ledger-conflicts-strict-parse
description: After resolving a conflict in tools/evidence/cinematic-impact.json, validate with a strict parser (no duplicate keys, name on every milestone); json.load alone is not enough
metadata:
  type: feedback
---

When a merge conflicts at the end of the ledger's milestone list, resolve it by re-reading both sides' complete entries and re-serialising the list, then validate with a parser that refuses duplicate keys and requires `name`/`evidence`/`validation` on every milestone.

**Why:** on 2026-10-07 I spliced the two conflict blocks by hand in the schema merge (c829ca3); `json.load` accepted the result, but one entry had duplicated keys (a second entry pasted inside it, so the parser silently kept the last values), one object had only `limits`, and two entries were repeated. Netuno found it by a strict parse (1b927e4 repaired it; a hygiene check was added afterwards).

**How to apply:** `python3 -c 'import json; json.load(open(p), object_pairs_hook=<reject-duplicates>)'` plus the per-milestone key check, before committing any ledger merge; count the milestones and compare with the sum of both sides. See [[feedback-merge-then-verify-separately]].
