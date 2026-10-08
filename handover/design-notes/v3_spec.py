import struct, itertools
FN_OFF=0xcbf29ce484222325; FN_P=0x100000001b3
def fnv(b, h=FN_OFF):
    for x in b: h=((h^x)*FN_P)&0xFFFFFFFFFFFFFFFF
    return h
def mesh(cells, vfirst=False):
    """cells: dict (x,y,z)->idx>0. Spec: axis d, sign s(-,+), layer k asc, rows v asc, cols u asc;
    u=(d+1)%3, v=(d+2)%3; face emitted iff neighbour empty."""
    keys=list(cells); lo=[min(k[i] for k in keys) for i in range(3)]; hi=[max(k[i] for k in keys) for i in range(3)]
    quads=[]
    for d in range(3):
        u_=(d+1)%3; v_=(d+2)%3
        if vfirst: u_,v_=v_,u_
        for s in (-1,1):
            for k in range(lo[d],hi[d]+1):
                m={}
                for c,i in cells.items():
                    if c[d]!=k: continue
                    n=list(c); n[d]+=s
                    if tuple(n) in cells: continue
                    m[(c[u_],c[v_])]=i
                if not m: continue
                done=set()
                for v in range(lo[v_],hi[v_]+1):
                    for u in range(lo[u_],hi[u_]+1):
                        if (u,v) in done or (u,v) not in m: continue
                        c=m[(u,v)]; w=1
                        while m.get((u+w,v))==c and (u+w,v) not in done: w+=1
                        h=1
                        while all(m.get((u+j,v+h))==c and (u+j,v+h) not in done for j in range(w)): h+=1
                        for a in range(w):
                            for b in range(h): done.add((u+a,v+b))
                        quads.append((d,s,k,u,v,w,h,c,u_,v_))
    return quads
def vertex_bytes(q):
    d,s,k,u0,v0,w,h,c,u_,v_=q
    pd=k+1 if s>0 else k
    cs=[(u0,v0),(u0+w,v0),(u0+w,v0+h),(u0,v0+h)]
    if s<0: cs=[cs[0],cs[3],cs[2],cs[1]]
    out=b''
    n=[0.,0.,0.]; n[d]=float(s)
    t=[0.,0.,0.]; t[u_]=1.0
    for (u,v) in cs:
        p=[0.]*3; p[d]=float(pd); p[u_]=float(u); p[v_]=float(v)
        out+=struct.pack('<3f',*p)+struct.pack('<3f',*n)+struct.pack('<2f',(c+0.5)/256,0.5)+struct.pack('<4f',*t,1.0)+struct.pack('<8f',*[0.]*8)+struct.pack('<4f',1,1,1,1)
    return out
def golden(qs):
    vb=b''.join(vertex_bytes(q) for q in qs)
    ib=b''.join(struct.pack('<6I',*[4*i+j for j in (0,1,2,0,2,3)]) for i in range(len(qs)))
    return len(qs),len(vb)//96,len(ib)//4,"%016x"%fnv(vb+ib)
def checks(cells,qs):
    exposed=0
    for c in cells:
        for d in range(3):
            for s in (-1,1):
                n=list(c); n[d]+=s
                if tuple(n) not in cells: exposed+=1
    area=sum(q[5]*q[6] for q in qs)
    cover={}
    ok_adj=True
    for q in qs:
        d,s,k,u0,v0,w,h,c,u_,v_=q
        for a in range(w):
            for b in range(h):
                key=(d,s,k,u0+a,v0+b)
                cover[key]=cover.get(key,0)+1
                cell=[0]*3; cell[d]=k; cell[u_]=u0+a; cell[v_]=v0+b
                nb=list(cell); nb[d]+=s
                if tuple(cell) not in cells or tuple(nb) in cells or cells[tuple(cell)]!=c: ok_adj=False
    return exposed,area,max(cover.values()),ok_adj
B1={ (x,y,z):7 for x in range(-3,5) for y in range(-3,2) for z in range(-3,3)}
q=mesh(B1); print("block 8x5x6 at (-3,-3,-3):",golden(q),checks(B1,q), "vfirst",golden(mesh(B1,True)))
W={ (x,y,z):1+((x//4+z//2)%2) for x in range(16) for y in range(2) for z in range(6)}
q=mesh(W); print("wall 16x2x6 tiles 4x2:",golden(q),checks(W,q),"vfirst count",len(mesh(W,True)))
H={c:1 for c in itertools.product(range(8),range(8),range(8))}
for x in (3,4):
    for y in (3,4):
        for z in range(8): H.pop((x,y,z))
q=mesh(H); print("8^3 with 2x2 centred hole:",golden(q),checks(H,q),"vfirst",len(mesh(H,True)))
for n in (1,2,3,5,8,9,17): 
    C={c:1 for c in itertools.product(range(n),repeat=3)}; print("cube",n,len(mesh(C)),end='; ')
print()
# single-cell and domino
print("single",golden(mesh({(0,0,0):3})),"domino same",len(mesh({(0,0,0):1,(1,0,0):1})),"domino diff",len(mesh({(0,0,0):1,(1,0,0):2})))
