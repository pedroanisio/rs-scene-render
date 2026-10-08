#!/bin/bash
export SR_AGENT=mercurio
S=/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad
cd /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-voxel
git log --oneline -1
~/.local/share/scene-render/bin/sr-build cargo build --release -p scene-render 2>&1 | grep -E "^error|Finished"
cp target/release/scene-render $S/rel_voxel
mkdir -p $S/hf/rel_v_hero
python3 tools/probe_render.py examples/cinematic-impact/hero.scene.xml --times 3.0 --size 1280x720 --agent mercurio --short --binary $S/rel_voxel --json $S/hf/rel_v_hero/p.json --timeout 900 --work $S/hf/rel_v_hero 2>&1 | tail -3
python3 -c "
import json;f=json.load(open('$S/hf/rel_v_hero/p.json'))['frames'][0];print('hero_t3.0',f['frame_png_sha256'],'wall',f['wall_seconds'])"
echo HERO DONE
