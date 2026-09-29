"""usd-core script: writes crates/sr-3d/tests/fixtures/values.usdc, one attribute per value
coding the crate reader handles, and values.expected.json with the values as USD reads them.

    python tools/fixtures/make_usdc_values.py
"""
import json
import os

from pxr import Gf, Sdf, Usd, Vt

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "sr-3d", "tests", "fixtures")
T = Sdf.ValueTypeNames
VALUES = {
    # scalars: inlined (small) and not
    "b": (T.Bool, True),
    "i": (T.Int, -7),
    "i64": (T.Int64, 1 << 40),
    "f": (T.Float, 0.25),
    "d_inline": (T.Double, 0.5),
    "d_wide": (T.Double, 0.1),
    "h": (T.Half, 1.5),
    "v3f_inline": (T.Float3, Gf.Vec3f(1, -2, 3)),
    "v3f": (T.Float3, Gf.Vec3f(1.5, -2.25, 1000)),
    "v2d": (T.Double2, Gf.Vec2d(0.1, 0.2)),
    "v4i": (T.Int4, Gf.Vec4i(1, 2, 300, -4)),
    "m4_diag": (T.Matrix4d, Gf.Matrix4d(2)),
    "m4": (T.Matrix4d, Gf.Matrix4d(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)),
    "m3": (T.Matrix3d, Gf.Matrix3d(1, 0, 0, 0, 0, -1, 0, 1, 0)),
    "qf": (T.Quatf, Gf.Quatf(0.5, 0.5, -0.5, 0.5)),
    "qd": (T.Quatd, Gf.Quatd(0.1, 0.2, 0.3, 0.9)),
    "s": (T.String, "hello"),
    "tok": (T.Token, "world"),
    "asset": (T.Asset, Sdf.AssetPath("textures/wood.png")),
    # arrays: short (stored plainly) and long (compressed)
    "ints_short": (T.IntArray, Vt.IntArray([3, 1, 4, 1, 5])),
    "ints_long": (T.IntArray, Vt.IntArray([(k * 37) % 1000 - 500 for k in range(200)])),
    "ints_big": (T.IntArray, Vt.IntArray([k * 100000 for k in range(40)] + [-(1 << 31) + 5, (1 << 31) - 5])),
    "i64s": (T.Int64Array, Vt.Int64Array([k * (1 << 36) - 3 for k in range(-20, 20)])),
    "floats_integral": (T.FloatArray, Vt.FloatArray([float(k % 7) for k in range(64)])),
    "floats_table": (T.FloatArray, Vt.FloatArray([[0.1, 0.25, 0.7][k % 3] for k in range(64)])),
    "floats_plain": (T.FloatArray, Vt.FloatArray([k * 0.013 for k in range(64)])),
    "doubles_long": (T.DoubleArray, Vt.DoubleArray([k * 0.5 for k in range(40)])),
    "halfs": (T.HalfArray, Vt.HalfArray([0.5 * k for k in range(20)])),
    "v3fs": (T.Float3Array, Vt.Vec3fArray([(k, -k, 0.5 * k) for k in range(30)])),
    "strings": (T.StringArray, Vt.StringArray(["a", "bb", "hello"])),
    "tokens": (T.TokenArray, Vt.TokenArray(["x", "world", "zz"])),
    "empty": (T.IntArray, Vt.IntArray([])),
}


def plain(v):
    if isinstance(v, bool):
        return [1.0 if v else 0.0]
    if isinstance(v, (int, float)):
        return [float(v)]
    if isinstance(v, str):
        return v
    if isinstance(v, Sdf.AssetPath):
        return v.path
    if isinstance(v, (Gf.Quatf, Gf.Quatd)):
        return [v.GetReal(), *v.GetImaginary()]
    if isinstance(v, (Gf.Matrix4d, Gf.Matrix3d)):
        return [float(x) for row in v for x in row]
    if hasattr(v, "__len__") and len(v) and isinstance(v[0], str):
        return list(v)
    if hasattr(v, "__len__"):
        out = []
        for x in v:
            out.extend(plain(x) if not isinstance(x, (int, float)) else [float(x)])
        return out
    raise TypeError(type(v))


path = os.path.join(OUT, "values.usdc")
if os.path.exists(path):
    os.remove(path)
stage = Usd.Stage.CreateNew(path)
prim = stage.DefinePrim("/V")
for name, (ty, v) in VALUES.items():
    prim.CreateAttribute(name, ty).Set(v)
stage.GetRootLayer().Save()
reopened = Usd.Stage.Open(path)
back = reopened.GetPrimAtPath("/V")
expected = {name: plain(back.GetAttribute(name).Get()) for name in VALUES}
with open(os.path.join(OUT, "values.expected.json"), "w") as f:
    json.dump(expected, f, indent=0, sort_keys=True)
print("WROTE", path)
