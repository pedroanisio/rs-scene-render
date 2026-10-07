#!/usr/bin/env python3
"""A dump of a MagicaVoxel file made without the importer: the chunks, the models, the palette, the materials and the nodes of the scene graph.

Usage: tools/vox_dump.py FILE.vox...
"""
import struct, sys, collections
def rd_dict(b, o):
    n, = struct.unpack_from('<i', b, o); o += 4; d = {}
    for _ in range(n):
        kl, = struct.unpack_from('<i', b, o); o += 4; k = b[o:o+kl].decode('latin1'); o += kl
        vl, = struct.unpack_from('<i', b, o); o += 4; v = b[o:o+vl].decode('latin1'); o += vl
        d[k] = v
    return d, o
def parse(path):
    b = open(path,'rb').read()
    assert b[:4] == b'VOX ', b[:4]
    ver, = struct.unpack_from('<i', b, 4)
    cid = b[8:12]; cs, ch = struct.unpack_from('<ii', b, 12)
    o = 20 + cs; end = o + ch
    models = []; size = None; rgba = None; matl = {}; nodes = {}; ids = collections.Counter()
    while o < end:
        cid = b[o:o+4]; cs, ch = struct.unpack_from('<ii', b, o+4); data = b[o+12:o+12+cs]; o += 12 + cs + ch
        ids[cid.decode('latin1')] += 1
        if cid == b'SIZE': size = struct.unpack_from('<iii', data)
        elif cid == b'XYZI':
            n, = struct.unpack_from('<i', data); vox = [tuple(data[4+4*i:8+4*i]) for i in range(n)]
            models.append((size, vox))
        elif cid == b'RGBA': rgba = [tuple(data[4*i:4*i+4]) for i in range(256)]
        elif cid == b'MATL':
            i, = struct.unpack_from('<i', data); d, _ = rd_dict(data, 4); matl[i] = d
        elif cid == b'nTRN':
            nid, = struct.unpack_from('<i', data); d, p = rd_dict(data, 4)
            child, res, layer, nf = struct.unpack_from('<iiii', data, p); p += 16
            frames = []
            for _ in range(nf):
                f, p = rd_dict(data, p); frames.append(f)
            nodes[nid] = ('T', child, frames, d)
        elif cid == b'nGRP':
            nid, = struct.unpack_from('<i', data); d, p = rd_dict(data, 4)
            n, = struct.unpack_from('<i', data, p); p += 4
            nodes[nid] = ('G', list(struct.unpack_from('<%di' % n, data, p)))
        elif cid == b'nSHP':
            nid, = struct.unpack_from('<i', data); d, p = rd_dict(data, 4)
            n, = struct.unpack_from('<i', data, p); p += 4; ms = []
            for _ in range(n):
                m, = struct.unpack_from('<i', data, p); p += 4; md, p = rd_dict(data, p); ms.append(m)
            nodes[nid] = ('S', ms)
    return dict(ver=ver, models=models, rgba=rgba, matl=matl, nodes=nodes, ids=ids, trailing=len(b)-end)
if __name__ == '__main__':
    for path in sys.argv[1:]:
        r = parse(path)
        print(path.split('/')[-1], 'ver', r['ver'], 'ids', dict(r['ids']), 'trailing', r['trailing'])
        for s, v in r['models']:
            cols = collections.Counter(c[3] for c in v)
            print('  model', s, len(v), 'colours', sorted(cols)[:8], '...', len(cols), 'maxcoord', [max(c[a] for c in v) for a in range(3)] if v else None)
        if r['matl']: print('  matl ids', min(r['matl']), max(r['matl']), len(r['matl']), {k: v for k, v in r['matl'].items() if v.get('_type') != '_diffuse'})
        for k, n in sorted(r['nodes'].items()):
            if n[0] == 'T': print('  node', k, n[0], n[1], n[2], n[3])
            else: print('  node', k, n)
