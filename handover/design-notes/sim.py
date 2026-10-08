import numpy as np
c=0.25; R=6.65
F=lambda u:0.5+(3*u-u**3)/4
fl,top=F(-0.5),F(1.0); under=lambda u:(F(u)-fl)/(top-fl)
print("F(0)",F(0),"under(0)",under(0),"under(-.4)",under(-.4),"under(.9)",under(.9))
def est(theta_deg,px):
    t=np.tan(np.radians(theta_deg)); n=np.array([np.sin(np.radians(theta_deg)),-np.cos(np.radians(theta_deg))])
    py=12+px*t   # analytic plane point
    reach=int(np.ceil(R/c))+1
    ci=int(px//c); cj=int(py//c)
    I,J,K=np.meshgrid(np.arange(ci-reach,ci+reach+1),np.arange(cj-reach,cj+reach+1),np.arange(-reach,reach+1),indexing='ij')
    x=(I+.5)*c-px; y=(J+.5)*c-py; z=(K+.5)*c-0.0
    filled = J>=np.floor((12+(I+.5)*c*t)/c)
    h=x*n[0]+y*n[1]
    m=(x*x+y*y+z*z<=R*R)&(h>=-0.5*R)
    share=filled[m].sum()/m.sum()
    lo,hi=-0.4,0.9
    for _ in range(60):
        mid=(lo+hi)/2
        if under(mid)<share: lo=mid
        else: hi=mid
    return (lo+hi)/2*R, m.sum()
for th in [1,10,20,30,45]:
    offs=[est(th,15+d)[0] for d in np.linspace(0,1.0,9)]
    print(th, "est offset over analytic plane (normal, m): min %.3f max %.3f; expected stair mean %.3f"%(min(offs),max(offs),0.125*np.cos(np.radians(th))))
print("1.5 deg, snapped: terrace top at the point vs the stair's mean (vertical m)")
for th in [1.0,1.5]:
    t=np.tan(np.radians(th)); res=[]
    for px in np.linspace(10,25,61):
        i=int(px//c); jt=np.floor((12+(i+.5)*c*t)/c)*c   # top of filled cells in that column (y down)
        py=12+px*t
        terrace=py-jt          # terrace above analytic plane (vertical)
        e,_=est(th,px)
        res.append(terrace-e/np.cos(np.radians(th)))
    print(th,"terrace minus mean plane: min %.3f max %.3f m"%(min(res),max(res)))
