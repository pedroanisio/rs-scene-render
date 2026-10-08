import itertools, math
exec(open('v3_spec.py').read().split("B1={")[0])
def corners(q):
    d,s,k,u0,v0,w,h,c,u_,v_=q
    pd=k+1 if s>0 else k
    pts=[]
    for (u,v) in [(u0,v0),(u0+w,v0),(u0+w,v0+h),(u0,v0+h)]:
        p=[0]*3; p[d]=pd; p[u_]=u; p[v_]=v; pts.append(tuple(p))
    return pts
def tj(cells):
    qs=mesh(cells)
    P=set()
    for q in qs: P.update(corners(q))
    inc=0
    for q in qs:
        cs=corners(q)
        for a,b in zip(cs,cs[1:]+cs[:1]):
            dv=[b[i]-a[i] for i in range(3)]; L=max(abs(x) for x in dv); st=[x//L for x in dv]
            for t in range(1,L):
                if tuple(a[i]+st[i]*t for i in range(3)) in P: inc+=1
    return len(cells),len(qs),inc
def sphere(r,shell=None,bands=0):
    n=int(r)+2; out={}
    for x,y,z in itertools.product(range(-n,n+1),repeat=3):
        R=math.sqrt(x*x+y*y+z*z)
        if R<=r and (shell is None or R>r-shell):
            out[(x,y,z)]=1+ (z//6)%bands if bands else 1
    return out
for name,g in [("solid r=20",sphere(20)),("shell t=2 r=20",sphere(20,2)),("solid r=20 4 bands",sphere(20,None,4))]:
    c,q,i=tj(g); print(name,"cells",c,"quads",q,"T-incidences",i,"per quad %.2f"%(i/q))
