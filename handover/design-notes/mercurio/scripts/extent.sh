#!/bin/bash
export SR_AGENT=mercurio
cd /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-base1
S=/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad
for v in hero zz_nofollow; do
  echo "== $v"
  ~/.local/share/scene-render/bin/sr-build cargo run -q --profile ci -p sr-eval --example pyro_extent -- examples/cinematic-impact/$v.scene.xml plume 5 5.96 2>&1 | grep -E "^EXTENT|^CELLS|^error"
done
echo EXTENT DONE
