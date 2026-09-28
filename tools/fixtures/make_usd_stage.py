"""usd-core script: writes one stage as crates/sr-3d/tests/fixtures/stage.usda, stage.usdc and
stage.usdz (holding the usdc), for the binary importer's parity test against the USDA importer,
and stage.expected.json: every mesh's points in world space (stage units) as USD computes them.

    python -m pip install usd-core && python tools/fixtures/make_usd_stage.py

The stage is Z-up in centimetres, with an Xform hierarchy using translate, rotateXYZ, orient,
scale and transform ops, a torus-like mesh with over 16 faces (so the crate compresses its integer
arrays) with face-varying UVs and normals, a quad with vertex UVs, a cube with a time-sampled
translate (no default), and UsdPreviewSurface materials bound through material:binding.
"""
import json
import math
import os

from pxr import Gf, Sdf, Usd, UsdGeom, UsdShade, UsdUtils, Vt

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "sr-3d", "tests", "fixtures")


def material(stage, path, color, metallic, roughness, opacity=1.0):
    mat = UsdShade.Material.Define(stage, path)
    sh = UsdShade.Shader.Define(stage, path + "/Surface")
    sh.CreateIdAttr("UsdPreviewSurface")
    sh.CreateInput("diffuseColor", Sdf.ValueTypeNames.Color3f).Set(Gf.Vec3f(*color))
    sh.CreateInput("metallic", Sdf.ValueTypeNames.Float).Set(metallic)
    sh.CreateInput("roughness", Sdf.ValueTypeNames.Float).Set(roughness)
    sh.CreateInput("opacity", Sdf.ValueTypeNames.Float).Set(opacity)
    mat.CreateSurfaceOutput().ConnectToSource(sh.ConnectableAPI(), "surface")
    return mat


def build(path):
    stage = Usd.Stage.CreateNew(path)
    UsdGeom.SetStageUpAxis(stage, UsdGeom.Tokens.z)
    UsdGeom.SetStageMetersPerUnit(stage, 0.01)
    world = UsdGeom.Xform.Define(stage, "/World")
    stage.SetDefaultPrim(world.GetPrim())
    red = material(stage, "/World/Looks/Red", (0.8, 0.1, 0.05), 0.0, 0.4)
    chrome = material(stage, "/World/Looks/Chrome", (0.9, 0.9, 0.92), 1.0, 0.15, 0.5)

    arm = UsdGeom.Xform.Define(stage, "/World/Arm")
    arm.AddTranslateOp().Set(Gf.Vec3d(10, -20, 30))
    arm.AddRotateXYZOp().Set(Gf.Vec3f(15, 30, 45))
    arm.AddScaleOp().Set(Gf.Vec3f(2, 2, 2))

    # a ring of quads: 12 x 4 = 48 faces, face-varying st and normals
    ring = UsdGeom.Mesh.Define(stage, "/World/Arm/Ring")
    nu, nv, R, r = 12, 4, 20.0, 5.0
    pts, counts, idx, st, nrm = [], [], [], [], []
    for i in range(nu):
        for j in range(nv):
            u, v = 2 * math.pi * i / nu, 2 * math.pi * j / nv
            pts.append(Gf.Vec3f((R + r * math.cos(v)) * math.cos(u), (R + r * math.cos(v)) * math.sin(u), r * math.sin(v)))
    for i in range(nu):
        for j in range(nv):
            quad = [i * nv + j, ((i + 1) % nu) * nv + j, ((i + 1) % nu) * nv + (j + 1) % nv, i * nv + (j + 1) % nv]
            counts.append(4)
            idx.extend(quad)
            for k, q in enumerate(quad):
                st.append(Gf.Vec2f((i + (k in (1, 2))) / nu, (j + (k in (2, 3))) / nv))
                p = pts[q]
                c = Gf.Vec3f(R * p[0] / math.hypot(p[0], p[1]), R * p[1] / math.hypot(p[0], p[1]), 0)
                nrm.append((p - c).GetNormalized())
    ring.CreatePointsAttr(Vt.Vec3fArray(pts))
    ring.CreateFaceVertexCountsAttr(Vt.IntArray(counts))
    ring.CreateFaceVertexIndicesAttr(Vt.IntArray(idx))
    ring.CreateNormalsAttr(Vt.Vec3fArray(nrm))
    ring.SetNormalsInterpolation(UsdGeom.Tokens.faceVarying)
    pv = UsdGeom.PrimvarsAPI(ring).CreatePrimvar("st", Sdf.ValueTypeNames.TexCoord2fArray, UsdGeom.Tokens.faceVarying)
    pv.Set(Vt.Vec2fArray(st))
    UsdShade.MaterialBindingAPI.Apply(ring.GetPrim()).Bind(red)

    pivot = UsdGeom.Xform.Define(stage, "/World/Pivot")
    pivot.AddTransformOp().Set(Gf.Matrix4d(1, 0, 0, 0, 0, 0, 1, 0, 0, -1, 0, 0, 50, 5, 0, 1))
    pivot.AddOrientOp().Set(Gf.Quatf(math.cos(0.3), 0, 0, math.sin(0.3)))
    quad = UsdGeom.Mesh.Define(stage, "/World/Pivot/Quad")
    quad.CreatePointsAttr(Vt.Vec3fArray([(-5, -5, 0), (5, -5, 0), (5, 5, 0), (-5, 5, 0)]))
    quad.CreateFaceVertexCountsAttr(Vt.IntArray([4]))
    quad.CreateFaceVertexIndicesAttr(Vt.IntArray([0, 1, 2, 3]))
    UsdGeom.PrimvarsAPI(quad).CreatePrimvar("st", Sdf.ValueTypeNames.TexCoord2fArray, UsdGeom.Tokens.vertex).Set(
        Vt.Vec2fArray([(0, 0), (1, 0), (1, 1), (0, 1)]))
    UsdShade.MaterialBindingAPI.Apply(quad.GetPrim()).Bind(chrome)

    cube = UsdGeom.Mesh.Define(stage, "/World/Mover")
    cube.CreatePointsAttr(Vt.Vec3fArray([(x, y, z) for z in (0, 4) for y in (0, 4) for x in (0, 4)]))
    cube.CreateFaceVertexCountsAttr(Vt.IntArray([4] * 6))
    cube.CreateFaceVertexIndicesAttr(Vt.IntArray([0, 2, 3, 1, 4, 5, 7, 6, 0, 1, 5, 4, 2, 6, 7, 3, 0, 4, 6, 2, 1, 3, 7, 5]))
    op = cube.AddTranslateOp()
    op.Set(Gf.Vec3d(-40, 0, 0), 0)
    op.Set(Gf.Vec3d(-40, 10, 0), 24)
    stage.GetRootLayer().Save()
    return stage


os.makedirs(OUT, exist_ok=True)
build(os.path.join(OUT, "stage.usda"))
build(os.path.join(OUT, "stage.usdc"))
usdz = os.path.join(OUT, "stage.usdz")
if os.path.exists(usdz):
    os.remove(usdz)
assert UsdUtils.CreateNewUsdzPackage(Sdf.AssetPath(os.path.join(OUT, "stage.usdc")), usdz)
stage = Usd.Stage.Open(os.path.join(OUT, "stage.usda"))
cache = UsdGeom.XformCache(Usd.TimeCode.Default())
expected = {}
for prim in stage.Traverse():
    if prim.IsA(UsdGeom.Mesh):
        m = cache.GetLocalToWorldTransform(prim)
        # a time-sampled transform without a default: the first sample
        if UsdGeom.Xformable(prim).TransformMightBeTimeVarying():
            m = UsdGeom.XformCache(Usd.TimeCode.EarliestTime()).GetLocalToWorldTransform(prim)
        pts = [m.Transform(Gf.Vec3d(p)) for p in UsdGeom.Mesh(prim).GetPointsAttr().Get()]
        expected[str(prim.GetPath())] = [[round(c, 6) for c in p] for p in pts]
with open(os.path.join(OUT, "stage.expected.json"), "w") as f:
    json.dump(expected, f, indent=0, sort_keys=True)
with open(os.path.join(OUT, "stage.usdc"), "rb") as f:
    head = f.read(16)
print("WROTE", OUT, head[:8], list(head[8:11]))
