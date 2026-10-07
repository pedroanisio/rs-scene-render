#!/usr/bin/env python3
"""Writes the MagicaVoxel `.vox` fixtures of crates/sr-3d/tests/fixtures/vox with the cells, the palette and the placed
positions that each one must give, as `<name>.expected.json`.

The writer shares no code with the importer. What the expected files say is what this script puts in the files and
computes by its own arithmetic; it is not what a real MagicaVoxel writes, and the facts that no real file has proved are
named in `UNPROVED` (the importer's tests and the proposal say the same):

  * the palette stored in the file is offset by one: the colour of palette index c (1 to 255) is the file's entry c - 1;
  * the cells of a model are placed about its centre, size / 2 with integer division, by the scene graph;
  * the rotation byte: bits 0-1 the column of the nonzero entry of the first row, bits 2-3 that of the second, the third is
    the column left, bits 4, 5, 6 the signs (1 negative) of the three rows, and a point is placed by p[k] = sign[k] * q[c[k]];
  * a MATL chunk names the palette index it is for, from 1.

Usage: tools/make_vox.py [OUT_DIR]   (default crates/sr-3d/tests/fixtures/vox)
"""
import itertools
import json
import os
import struct
import sys

UNPROVED = [
    "palette offset by one",
    "model centre under the scene graph",
    "rotation byte",
    "MATL id is the palette index",
]


def chunk(name, content=b"", children=b""):
    return name.encode() + struct.pack("<ii", len(content), len(children)) + content + children


def string(s):
    b = s.encode()
    return struct.pack("<i", len(b)) + b


def dictionary(pairs):
    out = struct.pack("<i", len(pairs))
    for k, v in pairs:
        out += string(k) + string(v)
    return out


def size_chunk(x, y, z):
    return chunk("SIZE", struct.pack("<iii", x, y, z))


def xyzi_chunk(voxels):
    return chunk("XYZI", struct.pack("<i", len(voxels)) + b"".join(struct.pack("<BBBB", *v) for v in voxels))


def rgba_chunk(palette):
    assert len(palette) == 256
    return chunk("RGBA", b"".join(struct.pack("<BBBB", *c) for c in palette))


def matl_chunk(index, pairs):
    return chunk("MATL", struct.pack("<i", index) + dictionary(pairs))


def ntrn(node, child, rotation=None, translation=None, name=None):
    attributes = [("_name", name)] if name else []
    frame = []
    if rotation is not None:
        frame.append(("_r", str(rotation)))
    if translation is not None:
        frame.append(("_t", "%d %d %d" % tuple(translation)))
    content = struct.pack("<i", node) + dictionary(attributes) + struct.pack("<iiii", child, -1, 0, 1) + dictionary(frame)
    return chunk("nTRN", content)


def ngrp(node, children):
    return chunk("nGRP", struct.pack("<i", node) + dictionary([]) + struct.pack("<i", len(children)) + b"".join(struct.pack("<i", c) for c in children))


def nshp(node, model):
    return chunk("nSHP", struct.pack("<i", node) + dictionary([]) + struct.pack("<ii", 1, model) + dictionary([]))


def file(version, body):
    return b"VOX " + struct.pack("<i", version) + chunk("MAIN", b"", body)


# ---------------------------------------------------------------- rotations

def rotations():
    """The 24 proper rotations as (byte, matrix rows), each row a (column, sign)."""
    out = []
    for perm in itertools.permutations(range(3)):
        for signs in itertools.product((1, -1), repeat=3):
            # the matrix has p[k] = sign[k] * q[perm[k]]; its determinant is the sign of the permutation times the product of the signs
            parity = sum(1 for i in range(3) for j in range(i + 1, 3) if perm[i] > perm[j]) % 2
            det = (-1 if parity else 1) * signs[0] * signs[1] * signs[2]
            if det != 1:
                continue
            byte = perm[0] | perm[1] << 2 | (1 << 4 if signs[0] < 0 else 0) | (1 << 5 if signs[1] < 0 else 0) | (1 << 6 if signs[2] < 0 else 0)
            out.append((byte, [(perm[k], signs[k]) for k in range(3)]))
    assert len(out) == 24 and len({b for b, _ in out}) == 24
    return out


def rotate(rows, q):
    return [rows[k][1] * q[rows[k][0]] for k in range(3)]


def centre(size):
    return [size[0] // 2, size[1] // 2, size[2] // 2]


def place(voxel, size, rows, t):
    local = [voxel[i] - centre(size)[i] for i in range(3)]
    r = rotate(rows, local)
    return [r[i] + t[i] for i in range(3)]


def scene_cell(p):
    """The scene's cell of a MagicaVoxel cell: x right, y forward, z up becomes x right, y down, z forward."""
    return [p[0], -p[2] - 1, p[1]]


def rgba(i):
    return (i * 7 % 256, i * 13 % 256, i * 29 % 256, 255)


def default_palette():
    return [rgba(i) for i in range(256)]


def expected(name, models, palette, cells, extra=None):
    out = {
        "models": [{"size": list(size), "voxels": [list(v) for v in voxels]} for size, voxels in models],
        "palette": None if palette is None else [list(c) for c in palette],
        "cells": sorted([list(c) + [i] for c, i in cells], key=lambda c: (c[2], c[1], c[0])),
        "unproved": UNPROVED,
    }
    if extra:
        out.update(extra)
    return out


def main():
    out_dir = sys.argv[1] if len(sys.argv) > 1 else "crates/sr-3d/tests/fixtures/vox"
    os.makedirs(out_dir, exist_ok=True)
    files = {}

    def emit(name, data, expect=None):
        with open(os.path.join(out_dir, name + ".vox"), "wb") as f:
            f.write(data)
        if expect is not None:
            with open(os.path.join(out_dir, name + ".expected.json"), "w") as f:
                json.dump(expect, f, indent=1, sort_keys=True)
                f.write("\n")
        files[name] = len(data)

    # one model of 3 x 4 x 5, with an RGBA chunk and no scene graph
    size = (3, 4, 5)
    voxels = [(0, 0, 0, 1), (2, 3, 4, 2), (1, 2, 3, 255), (0, 3, 0, 7), (2, 0, 4, 1)]
    palette = default_palette()
    body = size_chunk(*size) + xyzi_chunk(voxels) + rgba_chunk(palette)
    cells = [(scene_cell(v[:3]), v[3]) for v in voxels]
    emit("one-model", file(150, body), expected("one-model", [(size, voxels)], palette, cells))
    emit("one-model-v200", file(200, body), expected("one-model-v200", [(size, voxels)], palette, cells))

    # unknown chunks are skipped, whatever they hold and wherever they are
    body = chunk("NOTE", b"\x01\x00\x00\x00" + string("a note")) + size_chunk(*size) + xyzi_chunk(voxels) + chunk("LAYR", b"\x00" * 20, b"") + rgba_chunk(palette) + chunk("rOBJ", dictionary([("_type", "_env")]))
    emit("unknown-chunks", file(150, body), expected("unknown-chunks", [(size, voxels)], palette, cells))

    # no RGBA chunk: the importer has no default palette to give
    emit("no-rgba", file(150, size_chunk(*size) + xyzi_chunk(voxels)), expected("no-rgba", [(size, voxels)], None, cells))

    # materials
    body = size_chunk(*size) + xyzi_chunk(voxels) + rgba_chunk(palette) + matl_chunk(1, [("_type", "_metal"), ("_rough", "0.25"), ("_metal", "0.9")]) + matl_chunk(2, [("_type", "_glass"), ("_trans", "0.5"), ("_ior", "0.3")]) + matl_chunk(7, [("_type", "_emit"), ("_flux", "2"), ("_emit", "0.75")])
    emit(
        "materials",
        file(150, body),
        expected(
            "materials",
            [(size, voxels)],
            palette,
            cells,
            {"materials": {"1": {"_type": "_metal", "_rough": "0.25", "_metal": "0.9"}, "2": {"_type": "_glass", "_trans": "0.5", "_ior": "0.3"}, "7": {"_type": "_emit", "_flux": "2", "_emit": "0.75"}}},
        ),
    )

    # two models, placed by a scene graph: a translation, and a rotation with a translation
    a_size, a_voxels = (3, 2, 2), [(0, 0, 0, 1), (2, 1, 1, 2), (1, 0, 1, 3)]
    b_size, b_voxels = (2, 3, 4), [(0, 0, 0, 4), (1, 2, 3, 5), (1, 0, 2, 6)]
    rows_b = rotations()[5][1]
    byte_b = rotations()[5][0]
    graph = ntrn(0, 1) + ngrp(1, [2, 4]) + ntrn(2, 3, translation=(10, -3, 7)) + nshp(3, 0) + ntrn(4, 5, rotation=byte_b, translation=(-4, 6, 2), name="b") + nshp(5, 1)
    body = chunk("PACK", struct.pack("<i", 2)) + size_chunk(*a_size) + xyzi_chunk(a_voxels) + size_chunk(*b_size) + xyzi_chunk(b_voxels) + graph + rgba_chunk(palette)
    ident = [(0, 1), (1, 1), (2, 1)]
    cells = [(scene_cell(place(v[:3], a_size, ident, (10, -3, 7))), v[3]) for v in a_voxels]
    cells += [(scene_cell(place(v[:3], b_size, rows_b, (-4, 6, 2))), v[3]) for v in b_voxels]
    emit("two-models", file(150, body), expected("two-models", [(a_size, a_voxels), (b_size, b_voxels)], palette, cells))

    # nested transforms: a rotation inside a rotation, and a group of groups
    m_size, m_voxels = (3, 4, 2), [(0, 0, 0, 1), (2, 3, 1, 2), (1, 2, 0, 3), (2, 0, 1, 4)]
    r1, r2 = rotations()[9], rotations()[17]
    graph = ntrn(0, 1, translation=(100, 0, 0)) + ngrp(1, [2]) + ntrn(2, 3, rotation=r1[0], translation=(5, 5, 5)) + ngrp(3, [4]) + ntrn(4, 5, rotation=r2[0], translation=(-2, 7, 1)) + nshp(5, 0)
    body = size_chunk(*m_size) + xyzi_chunk(m_voxels) + graph + rgba_chunk(palette)
    cells = []
    for v in m_voxels:
        inner = place(v[:3], m_size, r2[1], (-2, 7, 1))  # the model's own transform
        outer = [rotate(r1[1], inner)[i] + (5, 5, 5)[i] for i in range(3)]
        outer = [outer[i] + (100, 0, 0)[i] for i in range(3)]
        cells.append((scene_cell(outer), v[3]))
    emit("nested", file(150, body), expected("nested", [(m_size, m_voxels)], palette, cells))

    # all 24 rotations of one asymmetric model, side by side
    s_size, s_voxels = (3, 4, 5), [(0, 0, 0, 1), (2, 0, 0, 2), (0, 3, 0, 3), (0, 0, 4, 4), (2, 3, 4, 5), (1, 1, 2, 6)]
    graph = ntrn(0, 1) + ngrp(1, [2 + 2 * k for k in range(24)])
    cells = []
    for k, (byte, rows) in enumerate(rotations()):
        graph += ntrn(2 + 2 * k, 3 + 2 * k, rotation=byte, translation=(40 * k, 0, 0)) + nshp(3 + 2 * k, 0)
        cells += [(scene_cell(place(v[:3], s_size, rows, (40 * k, 0, 0))), v[3]) for v in s_voxels]
    body = size_chunk(*s_size) + xyzi_chunk(s_voxels) + graph + rgba_chunk(palette)
    emit("rotations", file(150, body), expected("rotations", [(s_size, s_voxels)], palette, cells, {"rotation_bytes": [b for b, _ in rotations()]}))

    # the example of the description of the file format (MagicaVoxel-file-format-vox-extension.txt, ROTATION type):
    #   R = [[0, 1, 0], [0, 0, -1], [-1, 0, 0]]  ==>  _r = (1 << 0) | (2 << 2) | (0 << 4) | (1 << 5) | (1 << 6)
    spec_matrix = [[0, 1, 0], [0, 0, -1], [-1, 0, 0]]
    spec_byte = (1 << 0) | (2 << 2) | (0 << 4) | (1 << 5) | (1 << 6)
    assert spec_byte == 105
    assert any(b == spec_byte and [[0] * 3 for _ in range(3)] and all(sum(abs(v) for v in row) == 1 for row in spec_matrix) for b, _ in rotations())
    for k, (row, (col, sign)) in enumerate(zip(spec_matrix, dict((b, r) for b, r in rotations())[spec_byte])):
        assert row[col] == sign and sum(abs(v) for v in row) == 1, k
    r_size, r_voxels = (3, 3, 3), [(0, 0, 0, 1), (2, 1, 0, 2), (1, 2, 2, 3)]
    graph = ntrn(0, 1) + ngrp(1, [2]) + ntrn(2, 3, rotation=spec_byte, translation=(5, 6, 7)) + nshp(3, 0)
    body = size_chunk(*r_size) + xyzi_chunk(r_voxels) + graph + rgba_chunk(palette)
    cells = []
    for v in r_voxels:
        q = [v[i] - r_size[i] // 2 for i in range(3)]
        p = [sum(spec_matrix[k][c] * q[c] for c in range(3)) + (5, 6, 7)[k] for k in range(3)]
        cells.append((scene_cell(p), v[3]))
    emit("spec-rotation", file(150, body), expected("spec-rotation", [(r_size, r_voxels)], palette, cells))

    print(json.dumps(files, sort_keys=True))


main()
