import numpy as np, sys
def faces_and_quads(g):
    """g: 3D int array (x,y,z) palette idx, 0 empty. returns (exposed faces, greedy quads) with deterministic scan."""
    nx,ny,nz=g.shape
    p=np.pad(g,1)
    faces=0; quads=0
    for axis in range(3):
        for sign in (-1,1):
            nb=np.roll(p,-sign,axis=axis)  # neighbour in direction sign
            m=np.where((p!=0)&(nb==0),p,0)[1:-1,1:-1,1:-1]
            faces+=int((m!=0).sum())
            # slices along axis
            m=np.moveaxis(m,axis,0)
            for s in range(m.shape[0]):
                a=m[s].copy()  # a[v][u] with u fastest? use (i,j)
                H,W=a.shape
                for j in range(H):   # outer
                    i=0
                    while i<W:
                        c=a[j,i]
                        if c==0: i+=1; continue
                        w=1
                        while i+w<W and a[j,i+w]==c: w+=1
                        h=1
                        while j+h<H and (a[j+h,i:i+w]==c).all(): h+=1
                        a[j:j+h,i:i+w]=0
                        quads+=1
                        i+=w
    return faces,quads
def sphere(cells_target):
    r=(cells_target*3/(4*np.pi))**(1/3)
    R=int(np.ceil(r))+1
    ax=np.arange(-R,R+1)
    X,Y,Z=np.meshgrid(ax,ax,ax,indexing='ij')
    g=((X**2+Y**2+Z**2)<=r*r).astype(np.int32)
    return g,r
if __name__=="__main__":
    for n in (4,8,100):
        g=np.ones((n,n,n),np.int32); print("cube",n,faces_and_quads(g))
    for n,h in ((8,2),(9,3),(100,10),(100,1)):
        g=np.ones((n,n,n),np.int32); a=(n-h)//2
        g[a:a+h,a:a+h,:]=0
        f,q=faces_and_quads(g); print("hole n",n,"h",h,"faces",f,"formula",6*n*n-2*h*h+4*h*n,"quads",q)
    g=np.zeros((3,3,3),np.int32); g[1,1,1]=1; print("single",faces_and_quads(g))
    g=np.zeros((2,1,1),np.int32); g[:,0,0]=1; print("domino same",faces_and_quads(g)); g[1,0,0]=2; print("domino diff",faces_and_quads(g))
    g,r=sphere(1_000_000); print("sphere cells",int(g.sum()),"r",r, faces_and_quads(g))
