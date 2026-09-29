"""PyOpenColorIO script: writes crates/sr-gpu/tests/fixtures/ocio/ (a small config with a LUT
file and a look) and expected.json, OpenColorIO's own display code values for sRGB-encoded
colours pushed through a working space, an exposure and a display/view, for two configs:
the built-in ACES CG config and the small one.

    python -m pip install opencolorio && python tools/fixtures/make_ocio_expected.py
"""
import json
import os

import PyOpenColorIO as ocio

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "sr-gpu", "tests", "fixtures", "ocio")
os.makedirs(OUT, exist_ok=True)

# a small config: scene-linear Rec.709, a log shaper, a display with a curve LUT file and a look
with open(os.path.join(OUT, "curve.spi1d"), "w") as f:
    n = 1024
    f.write(f"Version 1\nFrom 0.0 1.0\nLength {n}\nComponents 1\n{{\n")
    for k in range(n):
        x = k / (n - 1)
        f.write(f"  {(x / (x + 0.18)) * 1.18 ** 0.9 * x ** 0.1:.6f}\n")  # a soft shoulder
    f.write("}\n")
cfg = ocio.Config.CreateRaw()
cfg.setSearchPath(".")
cfg.setRole(ocio.ROLE_SCENE_LINEAR, "lin_rec709")
cfg.setRole(ocio.ROLE_COMPOSITING_LOG, "log")
lin = ocio.ColorSpace(name="lin_rec709")
cfg.addColorSpace(lin)
log = ocio.ColorSpace(name="log", encoding="log")
log.setTransform(ocio.LogAffineTransform(logSideSlope=[1 / 18] * 3, logSideOffset=[0.6] * 3,
                                         linSideOffset=[0.005] * 3), ocio.COLORSPACE_DIR_FROM_REFERENCE)
cfg.addColorSpace(log)
film = ocio.ColorSpace(name="film_srgb")
film.setTransform(ocio.GroupTransform([
    ocio.MatrixTransform([1.1, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.9, 0.0, 0.0, 0.0, 0.0, 1.0]),
    ocio.RangeTransform(minInValue=0.0, maxInValue=16.0, minOutValue=0.0, maxOutValue=1.0),
    # a square-root-like lift with a linear toe (a bare power has infinite slope at 0, which no
    # lattice can follow; display curves have toes for that reason)
    ocio.ExponentWithLinearTransform(gamma=[2.0] * 3 + [1.0], offset=[0.05] * 3 + [0.0],
                                     direction=ocio.TRANSFORM_DIR_INVERSE),
    ocio.FileTransform(src="curve.spi1d", interpolation=ocio.INTERP_LINEAR),
    ocio.ExponentWithLinearTransform(gamma=[2.4] * 3 + [1.0], offset=[0.055] * 3 + [0.0],
                                     direction=ocio.TRANSFORM_DIR_INVERSE),
]), ocio.COLORSPACE_DIR_FROM_REFERENCE)
cfg.addColorSpace(film)
cfg.addDisplayView("sRGB", "Film", "film_srgb")
cfg.addDisplayView("sRGB", "Raw", "lin_rec709")
look = ocio.Look(name="warm", processSpace="log",
                 transform=ocio.CDLTransform(slope=[1.1, 1.0, 0.9], offset=[0.01, 0.0, 0.005], power=[1.0, 1.0, 1.0], sat=1.2))
cfg.addLook(look)
cfg.setActiveDisplays("sRGB")
cfg.setActiveViews("Film, Raw")
cfg.validate()
with open(os.path.join(OUT, "small.ocio"), "w") as f:
    f.write(cfg.serialize())
small = ocio.Config.CreateFromFile(os.path.join(OUT, "small.ocio"))

COLORS = ["#000000", "#FFFFFF", "#808080", "#FF0000", "#00FF00", "#0000FF", "#FFC080", "#204060", "#C8102E"]
EXPOSURES = [0.0, 3.0, -2.0]


def srgb_to_lin(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def expect(config, working, display, view, looks=""):
    vt = ocio.DisplayViewTransform(src=working, display=display, view=view)
    if looks:
        lt = ocio.LookTransform(src=working, dst=working, looks=looks)
        proc = config.getProcessor(ocio.GroupTransform([lt, vt])).getDefaultCPUProcessor()
    else:
        proc = config.getProcessor(vt).getDefaultCPUProcessor()
    to_working = config.getProcessor("lin_rec709" if config is small else "Linear Rec.709 (sRGB)", working).getDefaultCPUProcessor()
    out = {}
    for ev in EXPOSURES:
        for h in COLORS:
            rgb = [srgb_to_lin(int(h[k:k + 2], 16) / 255.0) for k in (1, 3, 5)]
            w = [c * 2.0 ** ev for c in to_working.applyRGB(rgb)]
            out[f"{h} {ev:g}"] = [round(v, 6) for v in proc.applyRGB(w)]
    return out


cg = ocio.Config.CreateFromFile("ocio://cg-config-latest")
expected = {
    "cg": {"config": "ocio://cg-config-latest", "working": "acescg", "display": "sRGB - Display",
           "view": "ACES 2.0 - SDR 100 nits (Rec.709)", "looks": "",
           "values": expect(cg, "ACEScg", "sRGB - Display", "ACES 2.0 - SDR 100 nits (Rec.709)")},
    "small": {"config": "small.ocio", "working": "linear-srgb", "display": "sRGB", "view": "Film", "looks": "warm",
              "values": expect(small, "lin_rec709", "sRGB", "Film", "warm")},
}
with open(os.path.join(OUT, "expected.json"), "w") as f:
    json.dump(expected, f, indent=1, sort_keys=True)
print("WROTE", OUT, ocio.__version__)
