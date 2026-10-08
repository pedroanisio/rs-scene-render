#!/bin/bash
export SR_AGENT=mercurio SR_GPU_SHORT=1
cd /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-voxel/crates/sr-gpu
B=$(ls -t ../../target/ci/deps/sr_gpu-* | grep -E -- "-[0-9a-f]{16}$" | head -1)
: > /tmp/claude-1000/libhang.log
for i in $(seq 1 10); do
  t0=$(date +%s)
  out=$(~/.local/share/scene-render/bin/sr-gpu timeout 120 $B 2>&1); rc=$?
  echo "run $i rc=$rc $(( $(date +%s)-t0 ))s $(echo "$out" | grep -E '^test result' | tr '\n' ' ')" >> /tmp/claude-1000/libhang.log
  if [ $rc -ne 0 ]; then echo "$out" | grep -E "FAILED|panicked|running" | head -5 >> /tmp/claude-1000/libhang.log; fi
done
echo DONE >> /tmp/claude-1000/libhang.log
