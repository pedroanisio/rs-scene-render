import numpy as np, time
exec(open('v3_hole.py').read().split("for n,h,a in")[0])
t=time.time()
r=62.04;n=130;x=np.arange(n)-n/2+.5
X,Y,Z=np.meshgrid(x,x,x,indexing='ij');R=np.sqrt(X**2+Y**2+Z**2)
sph=(R<=r).astype(np.uint8)
print("sphere cells",int(sph.sum()),"quads ufirst",len(quads(sph,True)),"vfirst",len(quads(sph,False)),time.time()-t,flush=True)
n2=420;x2=np.arange(n2)-n2/2+.5
X,Y,Z=np.meshgrid(x2,x2,x2,indexing='ij');R=np.sqrt(X**2+Y**2+Z**2)
sh=((R<=199.5)&(R>197.5)).astype(np.uint8)
print("shell cells",int(sh.sum()),"quads ufirst",len(quads(sh,True)),"vfirst",len(quads(sh,False)),time.time()-t,flush=True)
