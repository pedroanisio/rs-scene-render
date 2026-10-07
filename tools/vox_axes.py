#!/usr/bin/env python3
"""The cells that a MagicaVoxel file's scene graph places, worked out by an independent decoder (tools/vox_dump.py, no code shared
with the importer), for files whose nodes have translations only. It is how crates/sr-3d/tests/fixtures/vox/real/dotvox_axes.expected.json
was made: the number of cells, their box, the checksum (a wrapping 64-bit sum over the cells, in the scene's axes) and the cube of the
colour 254.

Usage: tools/vox_axes.py FILE.vox
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from vox_dump import parse

r = parse(sys.argv[1])
N = r['nodes']
cells = {}
def walk(i, t):
    n = N[i]
    if n[0] == 'T':
        f = n[2][0] if n[2] else {}
        tt = [int(v) for v in f.get('_t', '0 0 0').split()]
        assert '_r' not in f
        walk(n[1], [t[k] + tt[k] for k in range(3)])
    elif n[0] == 'G':
        for c in n[1]: walk(c, t)
    else:
        for m in n[1]:
            s, vox = r['models'][m]
            for x, y, z, c in vox:
                p = (x - s[0]//2 + t[0], y - s[1]//2 + t[1], z - s[2]//2 + t[2])
                cells[(p[0], -p[2]-1, p[1])] = c
walk(0, [0,0,0])
print(len(cells), [min(c[a] for c in cells) for a in range(3)], [max(c[a] for c in cells) for a in range(3)])
h = lambda v, p: ((v + 1000) * p) & (2**64-1)
cs = 0
for (x, y, z), c in cells.items():
    cs = (cs + (h(x,73856093) ^ h(y,19349663) ^ h(z,83492791) ^ ((c*1315423911) & (2**64-1)))) & (2**64-1)
print('checksum', cs)
cube = [(x, z, -y-1) for (x, y, z), c in cells.items() if c == 254]
print(len(cube), [min(c[a] for c in cube) for a in range(3)], [max(c[a] for c in cube) for a in range(3)])
