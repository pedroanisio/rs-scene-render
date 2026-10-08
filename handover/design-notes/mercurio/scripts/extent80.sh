#!/bin/bash
export SR_AGENT=mercurio
cd /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-base1
S=/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad
git log --oneline -1
python3 - <<'PY'
p="crates/sr-eval/examples/pyro_extent.rs"
s=open(p).read()
s=s.replace('''        println!(
            "EXTENT t={time}:''','''        let count = |top: bool| -> usize {
            let edge = if top { ylo } else { yhi };
            voxels.iter().filter(|v| (v.0 - edge).abs() < 3.0 * h).count()
        };
        println!("CELLS t={time}: stored cells in the 3 cells at the top face {}, bottom face {}", count(true), count(false));
        println!(
            "EXTENT t={time}:''',1)
open(p,"w").write(s)
PY
cp $S/hf/nofollow80.scene.xml examples/cinematic-impact/zz_nofollow.scene.xml
for v in hero zz_nofollow; do
  echo "== $v"
  ~/.local/share/scene-render/bin/sr-build cargo run -q --profile ci -p sr-eval --example pyro_extent -- examples/cinematic-impact/$v.scene.xml plume 5 5.96 2>&1 | grep -E "^EXTENT|^CELLS|^error"
done
rm -f examples/cinematic-impact/zz_nofollow.scene.xml
git checkout crates/sr-eval/examples/pyro_extent.rs
echo EXTENT80 DONE
