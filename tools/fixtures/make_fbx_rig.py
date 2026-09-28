"""Blender script: writes crates/sr-3d/tests/fixtures/rig.fbx and rig.expected.json.

    blender -b --factory-startup --python tools/fixtures/make_fbx_rig.py

The scene (24 fps, frames 0..24, so t = frame / 24):
  * Rig: an armature with bones root (z 0..1) and tip (z 1..2); the object moves 1 m along x and
    the tip bone turns 90 degrees about x.
  * Bar: a 0.2 x 0.2 x 2 m bar in 8 segments, skinned to the rig (weights blend around z = 1).
  * Pivot / Spinner: an empty moving 2 m along y with a cube child turning 180 degrees about z.
  * Blob: a cube with a shape key "Grow" (1.5x) keyed 0 -> 1.
The expected file holds Blender's evaluated world-space vertices of every mesh at t = 0, 0.5 and
1 s, converted to the importer's Y-up metres (x, z, -y), as a bounding box and the centroid of the
distinct vertex positions.
"""
import json
import math
import os

import bpy
import bmesh
from mathutils import Vector

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "sr-3d", "tests", "fixtures")

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.render.fps = 24
scene.frame_start, scene.frame_end = 0, 24

# ---- armature
arm_data = bpy.data.armatures.new("RigData")
rig = bpy.data.objects.new("Rig", arm_data)
scene.collection.objects.link(rig)
bpy.context.view_layer.objects.active = rig
bpy.ops.object.mode_set(mode="EDIT")
root = arm_data.edit_bones.new("root")
root.head, root.tail = (0, 0, 0), (0, 0, 1)
tip = arm_data.edit_bones.new("tip")
tip.head, tip.tail = (0, 0, 1), (0, 0, 2)
tip.parent = root
tip.use_connect = True
bpy.ops.object.mode_set(mode="OBJECT")

# ---- skinned bar
me = bpy.data.meshes.new("BarMesh")
bm = bmesh.new()
segs = 8
rings = []
for k in range(segs + 1):
    z = 2.0 * k / segs
    rings.append([bm.verts.new((x, y, z)) for x, y in ((-0.1, -0.1), (0.1, -0.1), (0.1, 0.1), (-0.1, 0.1))])
for k in range(segs):
    a, b = rings[k], rings[k + 1]
    for i in range(4):
        j = (i + 1) % 4
        bm.faces.new((a[i], a[j], b[j], b[i]))
bm.faces.new(list(reversed(rings[0])))
bm.faces.new(rings[-1])
bm.to_mesh(me)
bm.free()
bar = bpy.data.objects.new("Bar", me)
scene.collection.objects.link(bar)
g_root, g_tip = bar.vertex_groups.new(name="root"), bar.vertex_groups.new(name="tip")
for v in me.vertices:
    w = min(1.0, max(0.0, (v.co.z - 0.75) / 0.5))
    if w < 1.0:
        g_root.add([v.index], 1.0 - w, "REPLACE")
    if w > 0.0:
        g_tip.add([v.index], w, "REPLACE")
bar.parent = rig
mod = bar.modifiers.new("Armature", "ARMATURE")
mod.object = rig

rig.location = (0, 0, 0)
rig.keyframe_insert("location", frame=0)
rig.location = (1, 0, 0)
rig.keyframe_insert("location", frame=24)
pb = rig.pose.bones["tip"]
pb.rotation_mode = "XYZ"
pb.rotation_euler = (0, 0, 0)
pb.keyframe_insert("rotation_euler", frame=0)
pb.rotation_euler = (math.radians(90), 0, 0)
pb.keyframe_insert("rotation_euler", frame=24)

# ---- parented spinner
pivot = bpy.data.objects.new("Pivot", None)
scene.collection.objects.link(pivot)
pivot.location = (3, 0, 0)
pivot.keyframe_insert("location", frame=0)
pivot.location = (3, 2, 0)
pivot.keyframe_insert("location", frame=24)
bpy.ops.mesh.primitive_cube_add(size=0.5, location=(0, 0, 0))
spin = bpy.context.active_object
spin.name = "Spinner"
spin.parent = pivot
spin.location = (0.5, 0, 0)
spin.rotation_euler = (0, 0, 0)
spin.keyframe_insert("rotation_euler", frame=0)
spin.rotation_euler = (0, 0, math.radians(180))
spin.keyframe_insert("rotation_euler", frame=24)

# ---- shape key
bpy.ops.mesh.primitive_cube_add(size=0.5, location=(-3, 0, 0))
blob = bpy.context.active_object
blob.name = "Blob"
blob.shape_key_add(name="Basis")
grow = blob.shape_key_add(name="Grow")
for d in grow.data:
    d.co = d.co * 1.5
grow.value = 0.0
grow.keyframe_insert("value", frame=0)
grow.value = 1.0
grow.keyframe_insert("value", frame=24)

# linear keys everywhere, so both sides interpolate the same way
for obj in bpy.data.objects:
    for ad in (obj.animation_data, getattr(getattr(obj.data, "shape_keys", None), "animation_data", None)):
        if ad and ad.action:
            for fc in ad.action.fcurves:
                for kp in fc.keyframe_points:
                    kp.interpolation = "LINEAR"

bpy.ops.export_scene.fbx(
    filepath=os.path.join(OUT, "rig.fbx"),
    axis_forward="-Z", axis_up="Y", apply_scale_options="FBX_SCALE_UNITS",
    add_leaf_bones=False, use_armature_deform_only=True,
    bake_anim=True, bake_anim_use_all_actions=False, bake_anim_use_nla_strips=False,
    bake_anim_simplify_factor=0.0, bake_anim_step=1.0,
)

expected = {}
for t in (0.0, 0.5, 1.0):
    scene.frame_set(int(round(t * 24)))
    dg = bpy.context.evaluated_depsgraph_get()
    frame = {}
    for obj in scene.objects:
        if obj.type != "MESH":
            continue
        ev = obj.evaluated_get(dg)
        mesh = ev.to_mesh()
        pts = {tuple(round(c, 5) for c in (ev.matrix_world @ v.co)) for v in mesh.vertices}
        ev.to_mesh_clear()
        yup = [(x, z, -y) for x, y, z in pts]
        lo = [min(p[k] for p in yup) for k in range(3)]
        hi = [max(p[k] for p in yup) for k in range(3)]
        c = [sum(p[k] for p in yup) / len(yup) for k in range(3)]
        frame[obj.name] = {"min": lo, "max": hi, "centroid": c, "distinct": len(yup)}
    expected[f"{t:.1f}"] = frame
with open(os.path.join(OUT, "rig.expected.json"), "w") as f:
    json.dump(expected, f, indent=1, sort_keys=True)
print("WROTE", OUT)
