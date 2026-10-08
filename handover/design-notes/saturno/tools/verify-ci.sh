#!/bin/bash
cd /home/pals/src/rs-scene-render/.claude/worktrees/saturno-pyro
count() { awk '/^test result/ {p+=$4; f+=$6; i+=$8; n++} /^error/ {e++} END {print "binaries",n,"passed",p,"failed",f,"ignored",i,"errors",e+0}'; }
for c in "$@"; do echo "$c $(cargo test --profile ci -p $c --no-fail-fast 2>&1 | grep -E '^(error|test result)' | count)"; done
cargo clippy --profile ci --workspace --all-targets -- -D warnings >/dev/null 2>&1; echo clippy-exit $?
cargo fmt --all --check >/dev/null 2>&1; echo fmt-exit $?
python3 tools/build_corpus.py >/dev/null 2>&1; echo corpus-exit $?
git status --short | grep -v "^R\|^A\|^ M\|^M\|^??" | head -3
echo done
