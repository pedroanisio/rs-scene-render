#!/bin/bash
export SR_AGENT=mercurio SR_GPU_SHORT=1
cd /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-gl
~/.local/share/scene-render/bin/sr-build cargo test --profile ci -p sr-gpu -p scene-render --no-run 2>&1 | grep -E "^error|Executable" > /tmp/claude-1000/gl_bins.txt
grep -c Executable /tmp/claude-1000/gl_bins.txt
: > /tmp/claude-1000/gl_sweep.log
while read -r line; do
  bin=$(echo "$line" | sed 's/.*(\(.*\))/\1/'); name=$(basename "$bin" | sed 's/-[0-9a-f]\{16\}$//')
  case "$line" in *scene_render*|*cli*|*solver_failures*) d=crates/scene-render;; *) d=crates/sr-gpu;; esac
  t0=$(date +%s)
  out=$(cd $d && ~/.local/share/scene-render/bin/sr-gpu timeout 900 /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-gl/$bin 2>&1); rc=$?
  res=$(echo "$out" | grep -E "^test result" | tr '\n' ' '); fails=$(echo "$out" | grep -E "^test .* FAILED" | head -5 | tr '\n' '|')
  echo "$name: rc=$rc $(( $(date +%s)-t0 ))s $res $fails" >> /tmp/claude-1000/gl_sweep.log
done < <(grep Executable /tmp/claude-1000/gl_bins.txt)
echo "SWEEP DONE" >> /tmp/claude-1000/gl_sweep.log
