#!/bin/bash
cd /home/pals/src/rs-scene-render/.claude/worktrees/mercurio-foam
export SR_AGENT=mercurio
sr=~/.local/share/scene-render/bin/sr-build
$sr cargo build --profile ci -p scene-render 2>&1 | grep -E "^error|Finished" 
$sr cargo test --profile ci -p sr-gpu --test water_light --test water_oracle --test light_grid --test volume --test three --test scene3d --test geodesic_scene --test horizon_specks --test foam_albedo --no-run 2>&1 | grep -E "^error|Finished|Executable"
echo BUILD DONE
