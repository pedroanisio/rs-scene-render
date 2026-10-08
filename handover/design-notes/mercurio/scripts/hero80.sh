#!/bin/bash
export SR_AGENT=mercurio
S=/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad
cd /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-base1
git log --oneline -1
~/.local/share/scene-render/bin/sr-build cargo build --release -p scene-render 2>&1 | grep -E "^error|Finished"
cp target/release/scene-render $S/rel_80f70d5
cp $S/hf/nofollow80.scene.xml examples/cinematic-impact/zz_nofollow.scene.xml
sed -i 's|^out\["t3"\].*||' $S/hf/run.py
python3 - <<'PY'
S="/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad"
s=open(f"{S}/hf/run.py").read()
if 't3' not in s.split('hero = ')[1]:
    s=s.replace('out = {}\n','out = {}\nout["t3"] = render(hero, "3.0", "hero_t3.0")\n',1)
open(f"{S}/hf/run80.py","w").write(s)
PY
python3 $S/hf/run80.py $S/rel_80f70d5 rel80
rm -f examples/cinematic-impact/zz_nofollow.scene.xml
echo HERO80 DONE
