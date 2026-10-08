import numpy as np, itertools, hashlib, struct
def quads(g, ufirst=True):
    pad=np.pad(g,1); out=[]
    for axis in range(3):
        for s in (-1,1):
            nb=np.roll(pad,-s,axis=axis)
            m=np.where((pad>0)&(nb==0),pad,0)[1:-1,1:-1,1:-1]
            m=np.moveaxis(m,axis,0)
            for k in range(m.shape[0]):
                sl=m[k]
                if not sl.any(): continue
                if not ufirst: sl=sl.T
                nv,nu=sl.shape  # rows=v, cols=u
                done=np.zeros(sl.shape,bool)
                for v in range(nv):
                    for u in range(nu):
                        if done[v,u] or sl[v,u]==0: continue
                        c=sl[v,u]; w=1
                        while u+w<nu and sl[v,u+w]==c and not done[v,u+w]: w+=1
                        h=1
                        while v+h<nv and (sl[v+h,u:u+w]==c).all() and not done[v+h,u:u+w].any(): h+=1
                        done[v:v+h,u:u+w]=True
                        out.append((axis,s,k,u,v,w,h,int(c)))
    return out
for n,h,a in [(5,1,2),(8,2,3),(9,3,3),(10,4,3),(12,3,2),(7,2,1),(7,5,1),(3,1,1)]:
    g=np.ones((n,n,n),np.uint8); g[a:a+h,a:a+h,:]=0
    q1=quads(g,True); q2=quads(g,False)
    print("hole n=%d h=%d a=%d: ufirst=%d vfirst=%d"%(n,h,a,len(q1),len(q2)))
for n in (1,2,3,7,8,9,16,17,33): print("cube",n,len(quads(np.ones((n,)*3,np.uint8))))
# L / non-order-dependent example: block with off-centre blind notch
g=np.ones((6,6,6),np.uint8); g[0:2,0:2,:]=0  # corner notch through
print("corner notch through", len(quads(g,True)),len(quads(g,False)))
# two-index domino and wall
g=np.zeros((2,1,1),np.uint8); g[0,0,0]=1; g[1,0,0]=2
print("domino differ (both opaque, face between culled):",len(quads(g)))
# wall 160x6x1000? use smaller wall: 40x4x25 with 8x4 tiles
W=np.zeros((40,4,25),np.uint8)
for x in range(40):
  for z in range(25): W[x,:,z]=1+((x//8+z//4)%2)
print("wall 40x4x25 tiles", len(quads(W)))
# vertex hash golden construction for a block: define bytes order
