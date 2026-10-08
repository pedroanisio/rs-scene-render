import math, random
def lattice(center, origin, h, reach):
    idx=lambda v,a:(v-origin[a])/h-0.5
    lox,hix=math.ceil(idx(center[0]-reach,0)),math.floor(idx(center[0]+reach,0))
    loy,hiy=math.ceil(idx(center[1]-reach,1)),math.floor(idx(center[1]+reach,1))
    t=0
    for i in range(lox,hix+1):
        dx=origin[0]+(i+0.5)*h-center[0]
        for j in range(loy,hiy+1):
            dy=origin[1]+(j+0.5)*h-center[1]
            rest=reach*reach-dx*dx-dy*dy
            if rest<0: continue
            half=math.sqrt(rest)
            lo=math.ceil(idx(center[2]-half,2)); hi=math.floor(idx(center[2]+half,2))
            t+=max(hi-lo+1,0)
    return t
def brute(center, origin, h, reach):
    n=int(reach/h)+3
    c=[round((center[a]-origin[a])/h) for a in range(3)]
    t=0
    for i in range(c[0]-n,c[0]+n+1):
      for j in range(c[1]-n,c[1]+n+1):
        for k in range(c[2]-n,c[2]+n+1):
          p=[origin[0]+h*(i+0.5),origin[1]+h*(j+0.5),origin[2]+h*(k+0.5)]
          if sum((p[a]-center[a])**2 for a in range(3))<=reach*reach: t+=1
    return t
random.seed(1)
bad=0
cases=[([0.25,0.25,0.25],[0,0,0],0.5,0.5),([7,0,0],[-8,-8,-8],0.5,0.5),([0.1*3,0,0],[-8]*3,0.1,0.1),([7,0,0],[-8,-8,-8],0.5,1.0)]
for _ in range(300):
    h=random.choice([0.5,0.25,0.1,1/3])
    o=[random.choice([-8,-4.0,-16/3]) for a in range(3)]
    # centers on cell centres / faces
    c=[o[a]+h*(random.randint(0,20)+random.choice([0,0.5])) for a in range(3)]
    r=h*random.choice([1,2,math.sqrt(2),math.sqrt(3),3,5])
    cases.append((c,o,h,r))
for c,o,h,r in cases:
    a,b=lattice(c,o,h,r),brute(c,o,h,r)
    if a!=b: bad+=1; print(c,o,h,r,a,b) if bad<8 else None
print("mismatches",bad,"of",len(cases))
