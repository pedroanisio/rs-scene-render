#!/usr/bin/env python3
"""Writes tests/corpus/media/block.vox: the body of cells that the valid documents of the corpus break, cut and drop things on.

One model of 16 x 4 x 16 cells (1024), the lowest layer of the file's z (the layer z = 0) with the palette index 1 and the two above with 2, so that
a partition by material has two pieces, with the synthetic palette of tools/make_vox.py. The asset the corpus had before, voxels.vox, has five cells, which a crater of the
corpus removes whole and a fracture of the corpus cuts into pieces that are all smaller than the least that a body takes: a valid document has to be one that does what it says.

Usage: tools/make_corpus_block.py [OUT]   (default tests/corpus/media/block.vox)
"""
import struct
import sys


def chunk(name, content=b"", children=b""):
    return name.encode() + struct.pack("<ii", len(content), len(children)) + content + children


def palette():
    # the synthetic palette of tools/make_vox.py (not MagicaVoxel's): 256 distinct entries, the colour of index c stored at c - 1
    return [(i * 7 % 256, i * 13 % 256, i * 29 % 256, 255) for i in range(256)]


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "tests/corpus/media/block.vox"
    size = (16, 16, 4)  # the file's x, y and z: z is up in MagicaVoxel and the scene's y is down, so the layers are along the file's z
    voxels = [(x, y, z, 1 if z == 0 else 2) for z in range(size[2]) for y in range(size[1]) for x in range(size[0])]
    body = (
        chunk("SIZE", struct.pack("<iii", *size))
        + chunk("XYZI", struct.pack("<i", len(voxels)) + b"".join(struct.pack("<BBBB", *v) for v in voxels))
        + chunk("RGBA", b"".join(struct.pack("<BBBB", *c) for c in palette()))
    )
    with open(out, "wb") as f:
        f.write(b"VOX " + struct.pack("<i", 150) + chunk("MAIN", b"", body))
    print(f"{out}: {len(voxels)} cells")


if __name__ == "__main__":
    main()
