import numpy as np
h=0.5; dt=5e-4; c=np.array([7.0,0,0]); R=1.033*(1e11*dt**2/1.2)**0.2
V=4/3*np.pi*R**3; Q=V/dt
def lattice(R):
    m=int(R/h)+3; g=(np.arange(-2*m,2*m)+0.5)*h
    X,Y,Z=np.meshgrid(g-7.0,g,g,indexing='ij')  # origin aligned: o=-8 or -16 -> centres at k+0.25? use offsets
    return None
def run(n):
    o=-n*h/2
    idx=(np.arange(n)+0.5)*h+o
    X,Y,Z=np.meshgrid(idx,idx,idx,indexing='ij')
    inside=(X-c[0])**2+(Y-c[1])**2+(Z-c[2])**2<=R*R
    # lattice count by brute force on a big lattice with same origin
    M=int(np.ceil((abs(o)+30)/h)); gi=(np.arange(-M,M)+0.5)*h+o
    gx=gi[(gi>=c[0]-R-h)&(gi<=c[0]+R+h)]; gy=gi[(gi>=-R-h)&(gi<=R+h)]
    GX,GY,GZ=np.meshgrid(gx,gy,gy,indexing='ij')
    whole=((GX-c[0])**2+GY**2+GZ**2<=R*R).sum()
    d=Q/(whole*h**3)
    u=[]
    for a in range(3):
        sh=[n,n,n]; sh[a]+=1
        g=[(np.arange(s)+(0.0 if i==a else 0.5))*h+o for i,s in enumerate(sh)]
        P=np.meshgrid(*g,indexing='ij'); D=[P[i]-c[i] for i in range(3)]; r2=D[0]**2+D[1]**2+D[2]**2
        u.append(np.where(r2<=R*R, d/3*D[a], Q/(4*np.pi)/(r2*np.sqrt(r2))*D[a]))
    tgt=np.where(inside,d,0.0)
    div=lambda u:(np.diff(u[0],axis=0)+np.diff(u[1],axis=1)+np.diff(u[2],axis=2))/h
    def lap(p):
        q=np.pad(p,1); return (6*p-q[:-2,1:-1,1:-1]-q[2:,1:-1,1:-1]-q[1:-1,:-2,1:-1]-q[1:-1,2:,1:-1]-q[1:-1,1:-1,:-2]-q[1:-1,1:-1,2:])
    b=(tgt-div(u))*h*h; p=np.zeros_like(b); r=b.copy(); z=r.copy(); rz=(r*r).sum()
    for it in range(4000):
        Az=lap(z); al=rz/(z*Az).sum(); p+=al*z; r-=al*Az; rn=(r*r).sum()
        if np.sqrt(rn)<1e-11*np.sqrt((b*b).sum()): break
        z=r+rn/rz*z; rz=rn
    q=np.pad(p,1); u[0]=u[0]-(q[1:,1:-1,1:-1]-q[:-1,1:-1,1:-1])/h
    ux=lambda x:u[0][int(round((x-o)/h)),n//2-1,n//2-1]
    return ux(0),ux(2),ux(4),d,whole,inside.sum()
s=run(32); l=run(64)
print("R",R); print("small",s); print("large",l)
print("u0 ratio",s[0]/l[0],"u4 ratio",s[2]/l[2],"u4/u2",s[2]/s[1])
free=Q/V; print("d_meas/free",3*(s[2]-s[1])/2/free, "d/free", s[3]/free)
