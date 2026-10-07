#!/usr/bin/env python3
"""A reference mesher for the surface of a grid of voxels, independent of the Rust one: the oracle of its counts and its hashes.

It extracts the exposed faces of a dict of cells (x, y, z) -> palette index, merges them greedily plane by plane, and prints the
quad counts of the fixtures the Rust tests state (a block of n^3 is 6 quads; a block with a square hole through it is 16 when the
hole has a margin of one cell, 10 when it touches the border) and the FNV-1a hashes of the compact quad list of three of them.

Rule (the same as crates/sr-3d/src/voxel): the face of cell A toward its neighbour B is exposed when B is empty, or A is opaque and B
is see-through, or both are see-through of different classes and class(A) < class(B). Faces are merged with the same class, axis,
sign and plane, scanning planes by (axis, sign, plane), rows by v, and taking the widest run in u first, then growing in v.
The compact list is sorted by (axis, sign, plane, v0, u0) and hashed as, per quad, axis u8, sign u8 (0 -, 1 +), plane i32,
u0 i32, v0 i32, w u16, h u16, class u8, little endian.

usage: tools/voxel_reference_mesher.py
"""
import struct
M64=(1<<64)-1
def fnv(bs,h=0xcbf29ce484222325):
    for b in bs:
        h^=b; h=(h*0x100000001b3)&M64
    return h
def faces(cells, see=frozenset()):
    # cells: dict (x,y,z)->idx ; returns dict (axis,sign,k,u,v)->idx of owner
    out={}
    for c,i in cells.items():
        for a in range(3):
            for s in (-1,1):
                n=list(c); n[a]+=s; j=cells.get(tuple(n),0)
                ok = (j==0) or (i not in see and j in see) or (i in see and j in see and i!=j and i<j)
                if ok:
                    k=c[a]+(1 if s==1 else 0)
                    b=(a+1)%3; cc=(a+2)%3
                    out[(a,s,k,c[b],c[cc])]=i
    return out
def greedy(F, order='uv'):
    quads=[]
    planes={}
    for (a,s,k,u,v),i in F.items(): planes.setdefault((a,s,k),{})[(u,v)]=i
    for key in sorted(planes, key=lambda t:(t[0],t[1],t[2])):
        a,s,k=key; m=dict(planes[key]); cons=set()
        cells=sorted(m, key=lambda p:(p[1],p[0]) if order=='uv' else (p[0],p[1]))
        for (u,v) in cells:
            if (u,v) in cons: continue
            i=m[(u,v)]
            if order=='uv':
                w=1
                while m.get((u+w,v))==i and (u+w,v) not in cons: w+=1
                h=1
                while all(m.get((u+d,v+h))==i and (u+d,v+h) not in cons for d in range(w)): h+=1
                for d in range(w):
                    for e in range(h): cons.add((u+d,v+e))
            else:
                h=1
                while m.get((u,v+h))==i and (u,v+h) not in cons: h+=1
                w=1
                while all(m.get((u+w,v+e))==i and (u+w,v+e) not in cons for e in range(h)): w+=1
                for d in range(w):
                    for e in range(h): cons.add((u+d,v+e))
            quads.append((a,s,k,v,u,w,h,i))
    quads.sort(key=lambda q:(q[0],q[1],q[2],q[3],q[4]))
    return quads
def qbytes(qs):
    b=bytearray()
    for a,s,k,v,u,w,h,i in qs:
        b+=struct.pack('<BBiiiHHB',a,0 if s<0 else 1,k,u,v,w,h,i)
    return bytes(b)
def block(o,n):
    return {(o[0]+x,o[1]+y,o[2]+z):1 for x in range(n) for y in range(n) for z in range(n)}
def hole(n,h,ox=None):
    a=(n-h)//2 if ox is None else ox
    c=block((0,0,0),n)
    for x in range(a,a+h):
        for y in range(a,a+h):
            for z in range(n): c.pop((x,y,z),None)
    return c
if __name__=='__main__':
    for n in list(range(1,13))+[16,17,25]:
        for o in [(0,0,0),(-3,5,-7),(5,5,5)]:
            F=faces(block(o,n)); q=greedy(F); assert len(q)==6 and len(F)==6*n*n,(n,o)
    print('cube ok 6 quads for n=1..12,16,17,25 at 3 origins')
    for n,h in [(3,1),(4,2),(5,1),(5,3),(6,2),(6,4),(8,2),(8,4),(8,6),(9,3),(12,2),(12,6),(16,4)]:
        c=hole(n,h); F=faces(c); q1=greedy(F,'uv'); q2=greedy(F,'vu')
        assert len(F)==6*n*n-2*h*h+4*h*n
        print('hole',n,h,'faces',len(F),'quads uv',len(q1),'vu',len(q2))
    # off-centre interior hole
    c=hole(9,3,ox=1); print('offcentre 9,3 a=1',len(greedy(faces(c))),len(greedy(faces(c),'vu')))
    c=hole(9,3,ox=0); print('hole touching border 9,3 a=0',len(greedy(faces(c))),len(greedy(faces(c),'vu')))
    # T shape order dependence in 2D: single plane
    # wall
    wall={(x,y,z):1+((x//4+z//2)%2) for x in range(24) for y in range(3) for z in range(16)}
    F=faces(wall); q=greedy(F); print('wall faces',len(F),'quads',len(q),'vu',len(greedy(F,'vu')))
    print('wall hash %016x'%fnv(qbytes(q)))
    b=block((0,0,0),8); qb=greedy(faces(b)); print('block8 hash %016x'%fnv(qbytes(qb)), len(qb))
    h8=hole(8,2); qh=greedy(faces(h8)); print('hole8,2 hash %016x'%fnv(qbytes(qh)), len(qh))
    # glass rule: 2x1x1 domino idx1 opaque idx2 glass
    d={(0,0,0):1,(1,0,0):2}
    print('domino opaque/glass',len(greedy(faces(d,see={2}))),'opaque/opaque',len(greedy(faces(d))))
    # coverage check
    F=faces(wall); cov={}
    for a,s,k,v,u,w,h,i in q:
        for du in range(w):
            for dv in range(h): cov[(a,s,k,u+du,v+dv)]=cov.get((a,s,k,u+du,v+dv),0)+1
    assert set(cov)==set(F) and all(x==1 for x in cov.values()); print('coverage exact')
