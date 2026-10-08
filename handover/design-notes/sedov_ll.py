import math
def ll(g, n=400000):
    nu1 = -(13*g*g - 7*g + 12)/((3*g-1)*(2*g+1)); nu2 = 5*(g-1)/(2*g+1)
    nu3 = 3/(2*g+1); nu4 = -nu1/(2-g); nu5 = -2/(2-g)
    def lam(V):
        a=(g+1)*V/2; b=(g+1)/(7-g)*(5-(3*g-1)*V); c=(g+1)/(g-1)*(g*V-1)
        return (a**-2 * b**nu1 * c**nu2)**0.2
    def G(V):
        b=(g+1)/(7-g)*(5-(3*g-1)*V); c=(g+1)/(g-1)*(g*V-1); d=(g+1)/(g-1)*(1-V)
        return (g+1)/(g-1) * c**nu3 * b**nu4 * d**nu5
    Vc, Vs = 1/g, 2/(g+1)
    # parametrize V = Vc + (Vs-Vc)*s^k to resolve the centre
    pts=[]
    for i in range(n+1):
        s=i/n; V = Vc + (Vs-Vc)*(s**4)
        if i>0 and V<=Vc: continue
        if i==0: pts.append((0.0,0.0,0.0)); continue
        Z = g*(g-1)*(1-V)*V*V/(2*(g*V-1))
        l = lam(V); Gv=G(V)
        f = Gv*l**4*(V*V/2 + Z/(g*(g-1)))
        m = Gv*l**2
        pts.append((l,f,m))
    I=0; M=0
    for (l0,f0,m0),(l1,f1,m1) in zip(pts,pts[1:]):
        I += 0.5*(f0+f1)*(l1-l0); M += 0.5*(m0+m1)*(l1-l0)
    alpha = 16*math.pi*I/25
    # central pressure ratio: p = rho c^2/g, c^2 = 4 r^2 Z/25 -> P ~ G lam^2 Z ; shock P1
    def Pn(V):
        Z = g*(g-1)*(1-V)*V*V/(2*(g*V-1)); return G(V)*lam(V)**2*Z
    pc = Pn(Vc*(1+1e-7))/Pn(Vs)
    return alpha**-0.2, alpha, M, pc
for g in (1.4, 5/3, 1.2):
    print(g, ll(g))
