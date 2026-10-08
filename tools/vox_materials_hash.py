#!/usr/bin/env python3
"""The fingerprint of the materials of a .vox fixture, worked out from the description of the hash in crates/sr-3d/src/voxel/material.rs
and not from its code, to pin the value that the tests of the loader expect.

The hash is FNV-1a over 64 bits (offset 0xcbf29ce484222325, prime 0x100000001b3) of: the number of materials (u64, little endian); then for
each palette index in ascending order: the index (one byte); the kind (one byte: diffuse 0, metal 1, glass 2, emit 3, blend 4, media 5, other
6 followed by the string); the media type (0, or 1 and the string); each of metal, rough, spec, ior, ri, att, flux, emit, ldr, trans, alpha, d, sp,
g, media (0 if absent, else 1 and the 8 bytes of the IEEE double, little endian, with -0 made +0); the number of keys that are not read (u64) and
each such key and value as strings. A string is its length in bytes (u64, little endian) and its bytes. The result is mixed:
x ^= x >> 33; x *= 0xff51afd7ed558ccd; x ^= x >> 33 (64 bits).

Usage: tools/vox_materials_hash.py FIXTURE.expected.json   (the "materials" object of a fixture written by tools/make_vox.py; a fixture with none gives the fingerprint of no materials)
"""
import json
import struct
import sys

M64 = (1 << 64) - 1
KINDS = {"_diffuse": 0, "_metal": 1, "_glass": 2, "_emit": 3, "_blend": 4, "_media": 5}
NUMBERS = ["_metal", "_rough", "_spec", "_ior", "_ri", "_att", "_flux", "_emit", "_ldr", "_trans", "_alpha", "_d", "_sp", "_g", "_media"]


def fingerprint(materials):
    h = 0xCBF29CE484222325

    def eat(data):
        nonlocal h
        for byte in data:
            h = ((h ^ byte) * 0x100000001B3) & M64

    def string(s):
        b = s.encode()
        eat(struct.pack("<Q", len(b)))
        eat(b)

    eat(struct.pack("<Q", len(materials)))
    for index in sorted(materials, key=int):
        d = dict(materials[index])
        eat(bytes([int(index)]))
        kind = d.pop("_type", "_diffuse")
        if kind in KINDS:
            eat(bytes([KINDS[kind]]))
        else:
            eat(bytes([6]))
            string(kind)
        media_type = d.pop("_media_type", None)
        if media_type is None:
            eat(bytes([0]))
        else:
            eat(bytes([1]))
            string(media_type)
        for key in NUMBERS:
            if key in d:
                eat(bytes([1]))
                eat(struct.pack("<d", float(d.pop(key)) + 0.0))
            else:
                eat(bytes([0]))
        eat(struct.pack("<Q", len(d)))
        for k in sorted(d):
            string(k)
            string(d[k])
    x = h
    x ^= x >> 33
    x = (x * 0xFF51AFD7ED558CCD) & M64
    x ^= x >> 33
    return x


if __name__ == "__main__":
    materials = json.load(open(sys.argv[1])).get("materials", {})
    print(fingerprint(materials))
