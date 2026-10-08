from fractions import Fraction as F
def props(cells, s=0.25, rho=2400.0):
    m1 = rho*s**3; n=len(cells); M=n*m1
    c=[[x*s+s/2 for x in cc] for cc in cells]
    com=[sum(p[a] for p in c)/n for a in range(3)]
    d=[[p[a]-com[a] for a in range(3)] for p in c]
    I=[[0.0]*3 for _ in range(3)]
    for a in range(3):
        b,e=(a+1)%3,(a+2)%3
        I[a][a]=n*m1/12*(2*s*s)+m1*sum(q[b]**2+q[e]**2 for q in d)
    for a,b in [(0,1),(0,2),(1,2)]:
        I[a][b]=I[b][a]=-m1*sum(q[a]*q[b] for q in d)
    return M,com,I
def det(m): return m[0][0]*(m[1][1]*m[2][2]-m[1][2]*m[2][1])-m[0][1]*(m[1][0]*m[2][2]-m[1][2]*m[2][0])+m[0][2]*(m[1][0]*m[2][1]-m[1][1]*m[2][0])
def solve(m,t):
    D=det(m); r=[]
    for i in range(3):
        mm=[row[:] for row in m]
        for k in range(3): mm[k][i]=t[k]
        r.append(det(mm)/D)
    return r
def box(xs,ys,zs): return [[x,y,z] for z in zs for y in ys for x in xs]
M,com,I=props(box(range(3),range(4),range(4))); print("3x4x4",M,com,[I[i][i] for i in range(3)])
notch=box(range(9,12),range(1,2),range(2))
piece=[c for c in box(range(6,12),range(2),range(2)) if c not in notch]
M,com,I=props(piece); print("L",len(piece),M,com); [print(r) for r in I]
tau=[30,55,-45.]
a=solve(I,tau); print("alpha",a)
def rel(b): return [abs(b[i]-a[i])/max(abs(a[i]),1e-3) for i in range(3)]
P=[0,2,1]; Is=[[I[P[i]][P[j]] for j in range(3)] for i in range(3)]
print("yz swap rel",rel(solve(Is,tau)))
If=[r[:] for r in I]; If[0][1]*=-1; If[1][0]*=-1; print("Ixy flip rel",rel(solve(If,tau)))
Id=[[I[i][j] if i==j else 0 for j in range(3)] for i in range(3)]; print("no products rel",rel(solve(Id,tau)))
for nm,cs in [("stay",box(range(5),range(2),range(2))),("first piece",box(range(6,12),range(2),range(2)))]:
    M,com,I2=props(cs); print(nm,[I2[i][i] for i in range(3)])
    a2=solve(I2,tau); Is2=[[I2[P[i]][P[j]] for j in range(3)] for i in range(3)]; b2=solve(Is2,tau)
    print("  yz swap rel",[abs(b2[i]-a2[i])/max(abs(a2[i]),1e-3) for i in range(3)])
