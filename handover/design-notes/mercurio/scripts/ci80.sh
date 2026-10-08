#!/bin/bash
export SR_AGENT=mercurio
S=/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad
until grep -q "EXTENT80 DONE" /tmp/claude-1000/extent80.log; do sleep 10; done
cd /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-base1
git log --oneline -1
~/.local/share/scene-render/bin/sr-build cargo build --profile ci -p scene-render 2>&1 | grep -E "^error|Finished"
cp target/ci/scene-render $S/ci_80f70d5
python3 tools/probe_render.py examples/cinematic-impact/hero.scene.xml --times 3.0 --size 1280x720 --agent mercurio --short --binary $S/ci_80f70d5 --json $S/hf/ci80_t3.json > /dev/null 2>&1
python3 -c "
import json; f=json.load(open('$S/hf/ci80_t3.json'))['frames'][0]; print('ci t3', f['frame_png_sha256'], 'trace_s', round(f['gpu']['passes_ms'].get('pathtrace trace',0)/1000,2))"
echo CI80 DONE
