#!/bin/bash
export SR_AGENT=mercurio SR_GPU_SHORT=1
cd /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-gl
~/.local/share/scene-render/bin/sr-build cargo test --profile ci -p sr-gpu --test adapters --test backends --no-run 2>&1 | grep -E "^error|Executable"
for b in adapters backends; do
  bin=$(ls -t target/ci/deps/$b-* | grep -E -- "-[0-9a-f]{16}$" | head -1)
  echo "== $b"
  (cd crates/sr-gpu && ~/.local/share/scene-render/bin/sr-gpu timeout 300 ../../$bin gl_ a_3d_document_on_opengl every_kind_the_3d --test-threads=1 2>&1 | grep -E "^test |panicked|Parent device|test result" | cut -c1-200)
done
echo RED DONE
