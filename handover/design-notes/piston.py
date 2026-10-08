import numpy as np, sys
n=32; h=0.5; o=-8.0
c=np.array([float(a) for a in sys.argv[1:4]]); R=float(sys.argv[4]); dt=5e-4
V=4/3*np.pi*R**3; Q=V/dt
idx=(np.arange(n)+0.5)*h+o
X,Y,Z=np.meshgrid(idx,idx,idx,indexing='ij')
inside=(X-c[0])**2+(Y-c[1])**2+(Z-c[2])**2<=R*R
N=inside.sum(); d=Q/(N*h**3)
print("covered vol",N*h**3,"sphere",V,"inflation",V/(N*h**3))
def faces(a):
    sh=[n,n,n]; sh[a]+=1
    g=[ (np.arange(s)+(0.0 if i==a else 0.5))*h+o for i,s in enumerate(sh)]
    return np.meshgrid(*g,indexing='ij')
def inject(mode):
    u=[]
    for a in range(3):
        P=faces(a); D=[P[i]-c[i] for i in range(3)]; r2=D[0]**2+D[1]**2+D[2]**2
        if mode=='div': u.append(np.zeros_like(r2)); continue
        v=np.where(r2<=R*R, d/3*D[a], Q/(4*np.pi)/(r2*np.sqrt(r2))*D[a]); u.append(v)
    return u
def div(u): return (np.diff(u[0],axis=0)+np.diff(u[1],axis=1)+np.diff(u[2],axis=2))/h
def lap(p):
    q=np.pad(p,1); return (6*p-q[:-2,1:-1,1:-1]-q[2:,1:-1,1:-1]-q[1:-1,:-2,1:-1]-q[1:-1,2:,1:-1]-q[1:-1,1:-1,:-2]-q[1:-1,1:-1,2:])
def project(u,target):
    b=(target-div(u))*h*h; p=np.zeros_like(b); r=b-lap(p); z=r.copy(); rz=(r*r).sum()
    for it in range(3000):
        Az=lap(z); al=rz/(z*Az).sum(); p+=al*z; r-=al*Az; rn=(r*r).sum()
        if np.sqrt(rn)<1e-12*np.sqrt((b*b).sum()): break
        z=r+rn/rz*z; rz=rn
    q=np.pad(p,1)
    u[0]-= (q[1:,1:-1,1:-1]-q[:-1,1:-1,1:-1])/h
    u[1]-= (q[1:-1,1:,1:-1]-q[1:-1,:-1,1:-1])/h
    u[2]-= (q[1:-1,1:-1,1:]-q[1:-1,1:-1,:-1])/h
    return u
def outflow(u): return ((u[0][-1]-u[0][0]).sum()+(u[1][:,-1]-u[1][:,0]).sum()+(u[2][:,:,-1]-u[2][:,:,0]).sum())*h*h
tgt=np.where(inside,d,0.0)
def ux(u,x): return u[0][int(round((x+8)/h)),n//2-1,n//2-1]
for mode in ['analytic','div']:
    u=inject(mode); u=project(u,tgt)
    pulse=[ux(u,x) for x in (-6,-2,0,2,4)]
    out=outflow(u)
    u=project(u,np.zeros_like(tgt))  # next step, no blast, no advection
    rest=[ux(u,x) for x in (-6,-2,0,2,4)]
    print(mode,"Q",Q,"outflow",out); print(" pulse ux",np.round(pulse,1)); print(" after ux",np.round(rest,1))
    print(" free-space ux (d0)", [round((Q/V)/3*(x-c[0]) if abs(x-c[0])<=R else -Q/(4*np.pi*(x-c[0])**2),1) for x in (-6,-2,0,2,4)])
