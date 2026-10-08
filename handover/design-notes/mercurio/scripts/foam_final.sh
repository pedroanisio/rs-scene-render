#!/bin/bash
export SR_AGENT=mercurio SR_GPU_SHORT=1
S=/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad
W=/home/pals/src/rs-scene-render/.claude/worktrees
sr=~/.local/share/scene-render/bin/sr-build
L=/tmp/claude-1000/foam_final.log
: > $L
cd $W/mercurio-base1 && git checkout -q --detach 966123a && echo "base1 at $(git log --oneline -1)" >> $L
$sr cargo build --profile ci -p scene-render 2>&1 | grep -E "^error|Finished" >> $L; cp target/ci/scene-render $S/fin_base
cd $W/mercurio-foam && echo "foam at $(git log --oneline -1)" >> $L
$sr cargo build --profile ci -p scene-render 2>&1 | grep -E "^error|Finished" >> $L; cp target/ci/scene-render $S/fin_foam
echo "--- fmt" >> $L; cargo fmt --all --check >> $L 2>&1 && echo fmt-ok >> $L
echo "--- clippy workspace" >> $L
$sr cargo clippy --profile ci --workspace --all-targets -- -D warnings 2>&1 | grep -E "^(error|warning)" -A6 | head -30 >> $L; echo clippy-done >> $L
echo "--- cpu tests sr-model sr-3d sr-eval" >> $L
$sr cargo test --profile ci -p sr-model -p sr-3d -p sr-eval --no-fail-fast 2>&1 | grep -E "^test result|FAILED|^error" | awk '/test result/ {p+=$4; f+=$6; next} {print} END {print "passed",p,"failed",f}' >> $L
echo "--- gpu binaries" >> $L
$sr cargo test --profile ci -p sr-gpu -p scene-render --no-run 2>&1 | grep -E "^error|Executable" > /tmp/claude-1000/foam_bins.txt
while read -r line; do
  bin=$(echo "$line" | sed 's/.*(\(.*\))/\1/'); name=$(basename "$bin" | sed 's/-[0-9a-f]\{16\}$//')
  case "$line" in *scene_render*|*cli*|*solver_failures*) d=crates/scene-render;; *) d=crates/sr-gpu;; esac
  t0=$(date +%s)
  out=$(cd $d && ~/.local/share/scene-render/bin/sr-gpu timeout 900 $W/mercurio-foam/$bin 2>&1); rc=$?
  res=$(echo "$out" | grep -E "^test result" | tr '\n' ' '); fails=$(echo "$out" | grep -E "^test .* FAILED" | head -5 | tr '\n' '|')
  echo "$name: rc=$rc $(( $(date +%s)-t0 ))s $res $fails" >> $L
done < <(grep Executable /tmp/claude-1000/foam_bins.txt)
echo "--- identity" >> $L
cd $S; python3 ident.py $S/fin_base fbase >> $L 2>&1; python3 ident.py $S/fin_foam ffoam >> $L 2>&1
python3 - >> $L <<'PY'
import json
S="/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad"
a=json.load(open(f"{S}/ident_fbase.json")); b=json.load(open(f"{S}/ident_ffoam.json"))
print("identity frames identical:", all(a[k][0]==b[k][0] for k in a), {k:(a[k][0],b[k][0]) for k in a if a[k][0]!=b[k][0]})
PY
cd $W/mercurio-foam
python3 tools/probe_render.py examples/cinematic-impact/hero.scene.xml --times 3.0 --size 1280x720 --agent mercurio --short --binary $S/fin_foam --json $S/hf/fin_foam_t3.json > /dev/null 2>&1
python3 -c "
import json; f=json.load(open('$S/hf/fin_foam_t3.json'))['frames'][0]; print('hero t3 foam-branch ci', f['frame_png_sha256'], 'trace_s', round(f['gpu']['passes_ms'].get('pathtrace trace',0)/1000,2))" >> $L
echo FINAL DONE >> $L
