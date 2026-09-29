"""Writes the still-image format fixtures and their reference values.

    python -m pip install pillow pillow-heif imagecodecs OpenEXR numpy
    python tools/fixtures/make_still_formats.py

Every lossless format stores the same 12×8 pattern (see `pattern`), which the
tests recompute. Lossy formats (JPEG, HEIC) are compared against the reference
decoders' output (Pillow/libjpeg, libheif), stored in `expected.json`.

The ICC fixtures carry profiles generated here from their primaries and tone
curves; `expected.json` holds what LittleCMS (Pillow's ImageCms) makes of each
pixel in sRGB, the oracle for the engine's own profile handling.
"""

import io
import json
import struct
import subprocess
from pathlib import Path

import numpy as np
import imagecodecs
import OpenEXR
import pillow_heif
from PIL import Image, ImageCms

OUT = Path(__file__).resolve().parents[2] / "crates/sr-media/tests/fixtures/still"
W, H = 12, 8


def pattern(alpha=False):
    y, x = np.mgrid[0:H, 0:W]
    px = np.stack([x * 21, y * 33, 200 - x * 15], -1)
    if alpha:
        px = np.concatenate([px, (255 - (x + y) * 12)[..., None]], -1)
    return px.clip(0, 255).astype(np.uint8)


# ------------------------------------------------------------------ ICC

D50 = np.array([0.9642, 1.0, 0.8249])
BRADFORD = np.array([[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]])


def xyz(x, y):
    return np.array([x / y, 1.0, (1 - x - y) / y])


def colorants(prim, white=(0.3127, 0.3290)):
    """RGB → XYZ columns, chromatically adapted to D50 (Bradford), as ICC stores them."""
    m = np.stack([xyz(*p) for p in prim], 1)
    s = np.linalg.solve(m, xyz(*white))
    m = m * s
    src, dst = BRADFORD @ xyz(*white), BRADFORD @ D50
    adapt = np.linalg.inv(BRADFORD) @ np.diag(dst / src) @ BRADFORD
    return adapt @ m


def s15(v):
    return struct.pack(">i", int(round(v * 65536)))


def curv_gamma(g):
    return b"curv" + b"\0" * 4 + struct.pack(">IH", 1, int(round(g * 256))) + b"\0\0"


def curv_table(f, n=1024):
    return b"curv" + b"\0" * 4 + struct.pack(">I", n) + b"".join(
        struct.pack(">H", int(round(f(i / (n - 1)) * 65535))) for i in range(n))


def para_srgb():
    return b"para\0\0\0\0\0\x03\0\0" + b"".join(
        s15(v) for v in (2.4, 1 / 1.055, 0.055 / 1.055, 1 / 12.92, 0.04045))


def text(s):
    return b"desc\0\0\0\0" + struct.pack(">I", len(s) + 1) + s.encode() + b"\0" + b"\0" * 79


def profile(desc, curve, prim=None, gray=False, version=4):
    tags = [(b"desc", text(desc)), (b"wtpt", b"XYZ \0\0\0\0" + b"".join(s15(v) for v in D50))]
    if gray:
        tags.append((b"kTRC", curve))
    else:
        m = colorants(prim)
        for i, sig in enumerate((b"rXYZ", b"gXYZ", b"bXYZ")):
            tags.append((sig, b"XYZ \0\0\0\0" + b"".join(s15(v) for v in m[:, i])))
        for sig in (b"rTRC", b"gTRC", b"bTRC"):
            tags.append((sig, curve))
    data, table = b"", b""
    base = 132 + 12 * len(tags)
    for sig, t in tags:
        table += sig + struct.pack(">II", base + len(data), len(t))
        data += t + b"\0" * (-len(t) % 4)
    head = bytearray(128)
    head[8] = version
    head[12:16] = b"mntr"
    head[16:20] = b"GRAY" if gray else b"RGB "
    head[20:24] = b"XYZ "
    head[36:40] = b"acsp"
    head[68:80] = b"".join(s15(v) for v in D50)
    out = bytes(head) + struct.pack(">I", len(tags)) + table + data
    return struct.pack(">I", len(out)) + out[4:]


SRGB_PRIM = ((0.64, 0.33), (0.30, 0.60), (0.15, 0.06))
P3_PRIM = ((0.680, 0.320), (0.265, 0.690), (0.150, 0.060))
ADOBE_PRIM = ((0.64, 0.33), (0.21, 0.71), (0.15, 0.06))


def icc_pattern():
    y, x = np.mgrid[0:H, 0:W]
    return np.stack([60 + x * 12, 80 + y * 15, 150 - x * 5], -1).astype(np.uint8)


def lcms_srgb(img, icc):
    src = ImageCms.ImageCmsProfile(io.BytesIO(icc))
    dst = ImageCms.createProfile("sRGB")
    mode = img.mode
    t = ImageCms.buildTransform(src, dst, mode, "RGB", renderingIntent=ImageCms.Intent.RELATIVE_COLORIMETRIC)
    return np.asarray(ImageCms.applyTransform(img, t)).tolist()


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    expected = {}
    rgb, rgba = Image.fromarray(pattern()), Image.fromarray(pattern(True))

    rgba.save(OUT / "pattern.png")
    rgba.save(OUT / "pattern.webp", lossless=True)
    rgba.save(OUT / "pattern.tiff")
    rgba.save(OUT / "pattern.tga")
    rgba.save(OUT / "pattern.qoi")
    rgb.save(OUT / "pattern.bmp")
    rgb.save(OUT / "pattern.ppm")
    rgb.convert("P", palette=Image.Palette.ADAPTIVE, colors=256).save(OUT / "pattern.gif")
    assert (np.asarray(Image.open(OUT / "pattern.gif").convert("RGB")) == pattern()).all(), "GIF palette lost colours"
    rgba.save(OUT / "pattern.dds")
    # farbfeld: magic, width, height, 16-bit big-endian RGBA.
    ff = b"farbfeld" + struct.pack(">II", W, H) + (pattern(True).astype(">u2") * 257).tobytes()
    (OUT / "pattern.ff").write_bytes(ff)
    (OUT / "pattern.jxl").write_bytes(imagecodecs.jpegxl_encode(pattern(), lossless=True))
    (OUT / "pattern.avif").write_bytes(imagecodecs.avif_encode(pattern(), level=100))
    assert (imagecodecs.avif_decode((OUT / "pattern.avif").read_bytes()) == pattern()).all(), "AVIF not lossless"

    # Animated GIF: three 100 ms frames, red, green, blue (played as a <video>).
    frames = [Image.new("RGB", (W, H), c) for c in [(255, 0, 0), (0, 255, 0), (0, 0, 255)]]
    frames[0].save(OUT / "anim.gif", save_all=True, append_images=frames[1:], duration=100, loop=0)

    # A CMYK JPEG tagged with a CMYK profile: decoded, its profile not usable. (The profile is
    # only a header claiming CMYK, which is all the engine reads before refusing it.)
    cmyk_icc = bytearray(profile("CMYK (test)", curv_gamma(1.0), SRGB_PRIM))
    cmyk_icc[16:20] = b"CMYK"
    rgb.convert("CMYK").save(OUT / "cmyk.jpg", quality=95, icc_profile=bytes(cmyk_icc))

    # PSD (raw, three channels): FFmpeg decodes it; the in-process decoders do not.
    psd = b"8BPS" + struct.pack(">H6xHIIHH", 1, 3, H, W, 8, 3) + struct.pack(">III", 0, 0, 0)
    psd += struct.pack(">H", 0) + np.moveaxis(pattern(), -1, 0).tobytes()
    (OUT / "pattern.psd").write_bytes(psd)
    # JPEG 2000, lossless (5/3 wavelet), through FFmpeg's encoder: also FFmpeg-only.
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(OUT / "pattern.png"), "-pix_fmt", "rgb24",
                    "-c:v", "jpeg2000", "-pred", "dwt53", str(OUT / "pattern.jp2")], check=True)

    # Linear light: Radiance HDR (flat RGBE scanlines) and OpenEXR (float32).
    lin = np.zeros((H, W, 3), np.float32)
    lin[..., 0] = 0.25
    lin[..., 1] = np.linspace(0.0, 4.0, W)[None, :]
    lin[..., 2] = 1.0
    hdr = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n" + f"-Y {H} +X {W}\n".encode()
    for v in lin.reshape(-1, 3):
        m = v.max()
        if m < 1e-32:
            hdr += b"\0\0\0\0"
        else:
            mant, e = np.frexp(m)
            hdr += bytes([int(c / m * mant * 256) for c in v] + [int(e) + 128])
    (OUT / "linear.hdr").write_bytes(hdr)
    OpenEXR.File({"type": OpenEXR.scanlineimage, "compression": OpenEXR.ZIP_COMPRESSION},
                 {"RGB": lin}).write(str(OUT / "linear.exr"))

    # JPEG with EXIF orientation 6 (rotate 90° clockwise to display): stored 12×8, shown 8×12.
    exif = Image.Exif()
    exif[0x0112] = 6
    rgb.save(OUT / "rotated.jpg", quality=100, subsampling=0, exif=exif)
    from PIL import ImageOps
    expected["rotated.jpg"] = np.asarray(ImageOps.exif_transpose(Image.open(OUT / "rotated.jpg"))).tolist()

    # ICC-tagged PNGs, and the LittleCMS view of them in sRGB.
    icc_rgb = Image.fromarray(icc_pattern())
    profiles = {
        "p3.png": profile("Display P3 (test)", para_srgb(), P3_PRIM),
        "adobe.png": profile("Adobe RGB (1998) compatible (test)", curv_gamma(563 / 256), ADOBE_PRIM, version=2),
        "table.png": profile("sRGB primaries, gamma 1.8 table (test)", curv_table(lambda v: v ** 1.8), SRGB_PRIM),
        "srgb-icc.png": profile("sRGB (test)", para_srgb(), SRGB_PRIM),
    }
    for name, icc in profiles.items():
        icc_rgb.save(OUT / name, icc_profile=icc)
        expected[name] = lcms_srgb(icc_rgb, icc)
    gray = Image.fromarray(icc_pattern()[..., 0])
    gicc = profile("Gray gamma 1.8 (test)", curv_gamma(1.8), gray=True, version=2)
    gray.save(OUT / "gray.png", icc_profile=gicc)
    expected["gray.png"] = lcms_srgb(gray, gicc)
    (OUT / "p3.icc").write_bytes(profiles["p3.png"])

    # HEIC: one with a Display P3 ICC (colr prof), one with P3 code points (colr nclx).
    pillow_heif.register_heif_opener()
    icc_rgb.save(OUT / "p3-icc.heic", quality=100, icc_profile=profiles["p3.png"], chroma=444)
    icc_rgb.save(OUT / "p3-nclx.heic", quality=100, chroma=444, color_primaries=12,
                 transfer_characteristics=13, matrix_coefficients=6, full_range_flag=1)
    for name in ("p3-icc.heic", "p3-nclx.heic"):
        h = pillow_heif.open_heif(OUT / name, convert_hdr_to_8bit=True)
        expected[name] = np.asarray(h).tolist()
        info = h.info
        assert ("icc_profile" in info) == (name == "p3-icc.heic"), (name, info.keys())
    expected["p3-srgb-lcms"] = lcms_srgb(icc_rgb, profiles["p3.png"])

    (OUT / "expected.json").write_text(json.dumps(expected, separators=(",", ":")))
    print("wrote", sorted(p.name for p in OUT.iterdir()))


if __name__ == "__main__":
    main()
