#!/usr/bin/env python3
"""Write the corpus robot.glb: a small closed mesh for import and collision tests.

Uses only the Python standard library. The corpus tests need real triangles,
not a placeholder GLB header; a unit cube keeps the fixture deterministic.
"""
import json
from pathlib import Path
import struct


def main():
    vertices = [
        (-0.5, -0.5, -0.5), (0.5, -0.5, -0.5),
        (0.5, 0.5, -0.5), (-0.5, 0.5, -0.5),
        (-0.5, -0.5, 0.5), (0.5, -0.5, 0.5),
        (0.5, 0.5, 0.5), (-0.5, 0.5, 0.5),
    ]
    indices = [
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7,
        0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2,
        0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5,
    ]
    positions = b"".join(struct.pack("<3f", *v) for v in vertices)
    triangles = struct.pack(f"<{len(indices)}H", *indices)
    binary = positions + triangles
    model = {
        "asset": {"version": "2.0"},
        "scene": 0, "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "buffers": [{"byteLength": len(binary)}],
        "bufferViews": [
            {"buffer": 0, "byteLength": len(positions)},
            {"buffer": 0, "byteOffset": len(positions), "byteLength": len(triangles)},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": len(vertices),
             "type": "VEC3", "min": [-0.5] * 3, "max": [0.5] * 3},
            {"bufferView": 1, "componentType": 5123, "count": len(indices), "type": "SCALAR"},
        ],
    }
    metadata = json.dumps(model, separators=(",", ":")).encode()
    metadata += b" " * (-len(metadata) % 4)
    binary += b"\0" * (-len(binary) % 4)
    glb = struct.pack("<4sII", b"glTF", 2, 28 + len(metadata) + len(binary))
    glb += struct.pack("<I4s", len(metadata), b"JSON") + metadata
    glb += struct.pack("<I4s", len(binary), b"BIN\0") + binary
    out = Path(__file__).resolve().parents[2] / "tests/corpus/media/robot.glb"
    out.write_bytes(glb)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
