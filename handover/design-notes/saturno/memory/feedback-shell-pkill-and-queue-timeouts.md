---
name: feedback-shell-pkill-and-queue-timeouts
description: "Two pitfalls in this session's shell: pkill -f matches my own command line and kills the shell; a timeout in front of sr-gpu counts the queue wait"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 8e0c69e6-b97d-44af-abdc-223a64772d14
---

- `pkill -f <pattern>` kills my own shell when the pattern appears in the command line (it happened twice, exit 144). Kill by pid taken from `ps`/`pgrep -P`, or use TaskStop for background tasks.
- `timeout N sr-gpu BIN` includes the time waiting for the GPU lock (two jobs hit rc=124 behind another agent's sweep); put the timeout inside: `sr-gpu timeout N BIN`.
- `pgrep -x cargo` matches every agent's cargo (all sessions are the same user): never wait on it; wait on my own log file's final line.

**Why:** both cost real time in the black-hole verification sweep.

**How to apply:** before a long sweep, write a script that logs one line per binary and ends with a sentinel line; watch the sentinel with Monitor.

- 2026-10-08: never name commit-message or helper files generically in /tmp/claude-1000/ (c2.txt, m1.txt...): other Claude sessions on this machine (other projects) write there too, and a stale file of theirs was committed as my message once (amended). Use the session scratchpad directory with a unique name, and write the file in the same command that commits it. Also: `pkill -f "until grep ..."` killed my own shell (exit 144) again; stop background waits with TaskStop or let them end.
