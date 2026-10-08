import numpy as np
pi=np.pi
def spec(nv,r,ior=1.333):
    a=r*r; f0=((ior-1)/(ior+1))**2
    v=np.array([np.sqrt(1-nv*nv),0,nv])
    N=40000
    s=(np.arange(N)+0.5)/N
    t=0.5*pi*s**4; dt=0.5*pi*4*s**3/N
    tot=0
    for p in (np.arange(180)+0.5)*pi/180:
        h=np.array([np.sin(t)*np.cos(p),np.sin(t)*np.sin(p),np.cos(t)])
        vh=v[0]*h[0]+v[1]*h[1]+v[2]*h[2]
        l=2*vh*h-v[:,None]
        nl=l[2]
        ok=(vh>0)&(nl>0)
        F=f0+(1-f0)*np.clip(1-vh,0,1)**5
        f=h[2]**2*(a*a-1)+1
        D=a*a/(pi*f*f)
        gv=nl*np.sqrt(nv*nv*(1-a*a)+a*a); gl=nv*np.sqrt(nl*nl*(1-a*a)+a*a)
        V=0.5/(gv+gl+1e-30)
        tot+=np.sum(np.where(ok,F*D*V*nl*4*vh*np.sin(t)*dt,0))*(pi/180)*2
    return tot
print(spec(0.998,0.06),spec(0.998,0.8))
