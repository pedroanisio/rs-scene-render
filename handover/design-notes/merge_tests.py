#!/usr/bin/env python3
"""Merge the integration-test files of a crate into a few binaries: tests/<area>.rs declares `mod <file>;` for the
files that move to tests/<area>/<file>.rs. Relative include_str!/include_bytes!/include! paths gain one `../`."""
import os, re, subprocess, sys

ROOT = '/home/pals/src/rs-scene-render/.claude/worktrees/saturno-pyro/crates'
PLAN = {
    'sr-sim': {
        'ocean': 'ocean ocean_bed ocean_events ocean_exchange ocean_lift ocean_order2 ocean_owners ocean_pressure ocean_push ocean_splash floating hydrostatics whitewater',
        'particles': 'particles3d particles3d_births particles3d_recent particles3d_recovery particles_water deforming_particles ejecta',
        'pyro': 'pyro pyro_gas pyro_heat pyro_mesh',
        'rigid': 'capture3 contacts3 cratering deforming_colliders exchange3 fracture fracture_contact frames3 impacts3 internal_edges sim surface3 watch_flood',
        'gr': 'gr gr_image',
    },
    'sr-eval': {
        'crater': 'crater crater_capture crater_ejecta crater_ejecta_ground crater_envelope crater_impact crater_impact_ocean crater_normal crater_smoke ejecta_splash impact_block impact_scenes cinematic_impact water_entry',
        'ocean': 'ocean ocean_buoyancy ocean_coupling ocean_coupling_cases ocean_depth_filter ocean_full ocean_group_time',
        'particles': 'particles3d particles_flat_emitter particles_frame_step particles_gas gas_and_splash emitter_mesh scoped_colliders',
        'volume': 'pyro pyro_bake pyro_colliders pyro_physics_failure volume terrain mesh_sequence',
        'fracture': 'fracture fracture_colliders fracture_contact solid_colliders physics_trace simulation simulation_regressions uses_3d',
        'scene': 'crater_envelope_placeholder',
    },
    'sr-model': {
        'rules': 'black_hole caption_lines crater fracture marker_rules model_select_rules ocean_body ocean_rules particles3d_rules physics_rules pyro_crater pyro_faces pyro_rules stroke_text_rules terrain_rules text3d_tracking_rules volume_rules',
        'assets': 'asset_kind_rules image_dimensions mesh_sequence volume_assets',
        'corpus': 'corpus hardening explain_texts severity_info',
    },
    'sr-3d': {
        'assets': 'assets clip_blend import joint_pose malformed materialx sequence skinning',
        'geometry': 'crater fracture fracture_surface terrain',
    },
    'scene-render': {
        'cli': 'aac_ceiling cli debug_gpu_flag eval_json_warnings solver_failures',
    },
}
# the rest of sr-eval's files go to `scene`
STANDALONE = {'sr-sim': ['pyro_export_memory'], 'sr-eval': ['perf', 'hero_hires', 'crater_memory', 'terrain_memory'], 'sr-model': ['perf'], 'sr-3d': [], 'scene-render': []}

def main(apply):
    for crate, areas in PLAN.items():
        tests = os.path.join(ROOT, crate, 'tests')
        files = sorted(f[:-3] for f in os.listdir(tests) if f.endswith('.rs'))
        plan = {a: v.split() for a, v in areas.items()}
        if crate == 'sr-eval':
            used = {f for v in plan.values() for f in v} | set(STANDALONE[crate])
            plan['scene'] = [f for f in files if f not in used]
        used = [f for v in plan.values() for f in v]
        assert len(used) == len(set(used)), (crate, 'a file in two areas')
        missing = [f for f in used if f not in files]
        assert not missing, (crate, missing)
        left = [f for f in files if f not in used and f not in STANDALONE[crate]]
        assert not left, (crate, 'unplaced', left)
        assert not set(plan) & set(files) - set(used), (crate, 'an area named like a file that stays')
        for area, names in plan.items():
            print(f'{crate}: tests/{area}.rs <- {len(names)} files')
            if not apply:
                continue
            os.makedirs(os.path.join(tests, area), exist_ok=True)
            for n in names:
                src, dst = os.path.join(tests, n + '.rs'), os.path.join(tests, area, n + '.rs')
                subprocess.check_call(['git', 'mv', src, dst], cwd=ROOT)
                text = open(dst).read()
                fixed = re.sub(r'(include_str!|include_bytes!|include!)\(\s*"(?!/)', lambda m: f'{m.group(1)}("../', text)
                if fixed != text:
                    open(dst, 'w').write(fixed)
            with open(os.path.join(tests, area, 'main.rs'), 'w') as out:
                out.write(f'//! The integration tests of the `{area}` area, one module for each file they came in.\n\n')
                for n in sorted(names):
                    out.write(f'mod {n};\n')

main('--apply' in sys.argv)
