#!/bin/bash
# usage: commit_item.sh MSGFILE   (stages crates tools tests srep, runs hygiene, commits)
cd "${WT:-/home/pals/src/rs-scene-render/.claude/worktrees/mercurio-voxel}"
git add -A crates tools tests srep-0000-cinematic-impact.md schema 2>/dev/null
python3 tools/check_release_hygiene.py --staged 2>&1 | grep -v "D1 review"
python3 tools/check_release_hygiene.py --commit-msg "$1" && git commit -q -F "$1" && git log --oneline -1 | cat
