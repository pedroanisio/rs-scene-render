import numpy as np
def quads(g):
    pad=np.pad(g,1); out=[]
    for axis in range(3):
        for s in (-1,1):
            nb=np.roll(pad,-s,axis=axis)
            m=np.where((pad>0)&(nb==0),pad,0)[1:-1,1:-1,1:-1]
            m=np.moveaxis(m,axis,0)
            for k in range(m.shape[0]):
                sl=m[k]
                if not sl.any(): continue
                # sl[o0,o1]; cyclic: x:(u=y=o0,v=z=o1) y:(u=z=o1,v=x=o0) z:(u=x=o0,v=y=o1)
                # want sl2[v,u]
                sl2 = sl.T if axis in (0,2) else sl
                nv,nu=sl2.shape; done=np.zeros(sl2.shape,bool)
                for v in range(nv):
                    for u in range(nu):
                        if done[v,u] or sl2[v,u]==0: continue
                        c=sl2[v,u]; w=1
                        while u+w<nu and sl2[v,u+w]==c and not done[v,u+w]: w+=1
                        h=1
                        while v+h<nv and (sl2[v+h,u:u+w]==c).all() and not done[v+h,u:u+w].any(): h+=1
                        done[v:v+h,u:u+w]=True
                        out.append((axis,s,k,u,v,w,h,int(c)))
    return out
W=np.zeros((40,4,25),np.uint8)
for x in range(40):
  for z in range(25): W[x,:,z]=1+((x//8+z//4)%2)
print("wall 40x4x25 tiles 8(x) x 4(z):",len(quads(W)))
g=np.ones((8,8,8),np.uint8); g[3:5,3:5,:]=0; print("hole",len(quads(g)))
# T-shape order dependence demo: slab 3x2x1 with one bump
T=np.zeros((3,2,1),np.uint8); T[:,0,0]=1; T[1,1,0]=1
print("T slab quads (cyclic rule)",len(quads(T)))
# 3-colour block 4x4x4 with index 2 on one cell
B=np.ones((4,4,4),np.uint8); B[1,1,3]=2; print("block with one different top cell",len(quads(B)))
