#!/bin/bash
export SR_AGENT=mercurio
S=/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad
cd /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-voxel
git log --oneline -1
cargo fmt --all --check && echo FMT-OK
~/.local/share/scene-render/bin/sr-build cargo clippy --profile ci -p sr-3d -p sr-model -p sr-eval -p sr-gpu -p scene-render --all-targets -- -D warnings 2>&1 | grep -E "^(error|warning)" -A6 | head -20; echo CLIPPY-DONE
~/.local/share/scene-render/bin/sr-build cargo test --profile ci -p sr-3d -p sr-eval -p sr-model 2>&1 | grep -E "FAILED|panicked|^error" | head; echo CPU-DONE
~/.local/share/scene-render/bin/sr-build cargo build --profile ci -p scene-render 2>&1 | grep -E "^error|Finished"
cp target/ci/scene-render $S/ci_voxel
ln -sfn $PWD/examples/cinematic-impact/sky.hdr $S/m7/sky.hdr
python3 $S/ident_v.py $S/ci_voxel voxel3 2>&1 | tail -8
python3 - <<'PY'
import json
S="/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad"
a=json.load(open(f"{S}/ident_voxel3.json")); b=json.load(open(f"{S}/ident_cifoam.json"))
print("IDENT", {k:a[k][0]==b[k][0] for k in b})
PY
~/.local/share/scene-render/bin/sr-build cargo build --release -p scene-render 2>&1 | grep -E "^error|Finished"
cp target/release/scene-render $S/rel_voxel
mkdir -p $S/hf/rel_v3
python3 tools/probe_render.py examples/cinematic-impact/hero.scene.xml --times 3.0 --size 1280x720 --agent mercurio --short --binary $S/rel_voxel --json $S/hf/rel_v3/p.json --timeout 900 --work $S/hf/rel_v3 2>&1 | tail -2
python3 -c "
import json;f=json.load(open('$S/hf/rel_v3/p.json'))['frames'][0];print('HERO',f['frame_png_sha256'],'wall',f['wall_seconds'])"
echo ALL-DONE
