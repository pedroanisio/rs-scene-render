#!/bin/bash
export SR_AGENT=mercurio
cd /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-voxel
git log --oneline -1
cargo fmt --all --check && echo FMT-OK
~/.local/share/scene-render/bin/sr-build cargo clippy --profile ci -p sr-3d -p sr-model -p sr-eval -p sr-gpu -p scene-render --all-targets -- -D warnings 2>&1 | grep -E "^(error|warning)" -A6 | head -20; echo CLIPPY-DONE
~/.local/share/scene-render/bin/sr-build cargo test --profile ci -p sr-3d -p sr-eval -p sr-model 2>&1 | grep -E "^test result|FAILED|panicked|^error" | sort | uniq -c | awk '{print}' | head -30
echo CPU-DONE
~/.local/share/scene-render/bin/sr-build cargo test --profile ci -p sr-gpu -p scene-render --no-run 2>&1 | grep -E "Executable|^error" > /tmp/claude-1000/bins_v.txt
D=target/ci/deps
for pat in "sr_gpu-" adapters- backends- process_env- raster_effects- volume_grids- path_tracer- scene_3d-; do
  b=$(grep -E "$pat" /tmp/claude-1000/bins_v.txt | head -1 | sed -E 's/.*\((.*)\)/\1/')
  echo "== $pat $b"
  (cd crates/sr-gpu; SR_GPU_SHORT=1 ~/.local/share/scene-render/bin/sr-gpu timeout 2400 ../../$b --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked" | head -5)
done
b=$(grep -E "tests/cli" /tmp/claude-1000/bins_v.txt | sed -E 's/.*\((.*)\)/\1/')
echo "== cli $b"
(cd crates/scene-render; ~/.local/share/scene-render/bin/sr-gpu timeout 2400 ../../$b --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked" | head -5)
echo FINAL-DONE
