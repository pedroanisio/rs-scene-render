import math
exec(open('port2.py').read().split('for g in map')[0])
def prof(g, target, steps=40000):
    xe=math.log(1e-5); h=xe/steps; y=[2/(g+1),math.log((g+1)/(g-1)),math.log(2/(g+1))]; x=0
    der=lambda x,y:[v*math.exp(x) for v in slopes(g,math.exp(x),y)]
    while x > math.log(target)+1e-12:
        k1=der(x,y);k2=der(x+h/2,[y[j]+h/2*k1[j] for j in range(3)]);k3=der(x+h/2,[y[j]+h/2*k2[j] for j in range(3)]);k4=der(x+h,[y[j]+h*k3[j] for j in range(3)])
        y=[y[j]+h/6*(k1[j]+2*k2[j]+2*k3[j]+k4[j]) for j in range(3)]; x+=h
    return math.exp(x), y[0], math.exp(y[1]), math.exp(y[2])
def ll(g,lam_t):
    nu1 = -(13*g*g - 7*g + 12)/((3*g-1)*(2*g+1)); nu2 = 5*(g-1)/(2*g+1)
    nu3 = 3/(2*g+1); nu4 = -nu1/(2-g); nu5 = -2/(2-g)
    def lam(V):
        a=(g+1)*V/2; b=(g+1)/(7-g)*(5-(3*g-1)*V); c=(g+1)/(g-1)*(g*V-1)
        return (a**-2 * b**nu1 * c**nu2)**0.2
    def G(V):
        b=(g+1)/(7-g)*(5-(3*g-1)*V); c=(g+1)/(g-1)*(g*V-1); d=(g+1)/(g-1)*(1-V)
        return (g+1)/(g-1) * c**nu3 * b**nu4 * d**nu5
    lo,hi=1/g*(1+1e-14),2/(g+1)
    for _ in range(200):
        m=(lo+hi)/2
        if lam(m)<lam_t: lo=m
        else: hi=m
    V=(lo+hi)/2; Z=g*(g-1)*(1-V)*V*V/(2*(g*V-1)); l=lam(V)
    return l, l*V, G(V), G(V)*l*l*Z/g
for g in (1.2,1.4):
    for t in (0.9,0.5):
        print(g,t,prof(g,t)); print('  LL',ll(g,prof(g,t)[0]))
