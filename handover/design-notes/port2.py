import math,sys
_e=math.exp
def EXP(v):
    try: return _e(v)
    except OverflowError: return float("inf")
def slopes(g,xi,y):
    u,lng,lnp=y; c2=EXP(lnp-lng); d=u-xi
    du=(1.5*u*d-3*c2+2*g*c2*u/xi)/(d*d-g*c2); dlng=-(du+2*u/xi)/d; dlnp=g*dlng+3/d
    return [du,dlng,dlnp]
def solve(g,steps=40000,ximin=1e-5):
    xe=math.log(ximin); h=xe/steps
    y=[2/(g+1),math.log((g+1)/(g-1)),math.log(2/(g+1))]; x=0.0
    def integ(xi,y):
        w=xi**3; G=EXP(y[1]);P=EXP(y[2]); return [G*w,0.5*G*y[0]**2*w,P*w/(g-1)]
    def der(x,y):
        xi=EXP(x); s=slopes(g,xi,y); return [v*xi for v in s]
    I=[0,0,0]
    for _ in range(steps):
        k1=der(x,y); at=lambda k,f:[y[i]+f*k[i] for i in range(3)]
        k2=der(x+h/2,at(k1,h/2));k3=der(x+h/2,at(k2,h/2));k4=der(x+h,at(k3,h))
        nx=[y[i]+h/6*(k1[i]+2*k2[i]+2*k3[i]+k4[i]) for i in range(3)]
        mid=[(y[i]+nx[i])/2+h/8*(k1[i]-k4[i]) for i in range(3)]
        f0,fm,f1=integ(EXP(x),y),integ(EXP(x+h/2),mid),integ(EXP(x+h),nx)
        for i in range(3): I[i]+=-h/6*(f0[i]+4*fm[i]+f1[i])
        y=nx;x+=h
        if not all(math.isfinite(v) for v in y): raise Exception("left at xi=%g"%EXP(x))
    I[2]+=EXP(y[2])*ximin**3/3/(g-1)
    J=I[1]+I[2]; return (25/(16*math.pi*J))**0.2, I[0], EXP(y[2])/(2/(g+1)), I[1]/J
for g in map(float,sys.argv[1:]):
    try: print(g, solve(g))
    except Exception as e: print(g,'ERR',e)
