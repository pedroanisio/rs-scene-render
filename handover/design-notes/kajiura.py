import numpy as np, sys
h=float(sys.argv[1]); a=float(sys.argv[2]); g=9.81
N=2048; dx=0.25; L=N*dx
x=(np.arange(N)-N//2)*dx
X,Z=np.meshgrid(x,x)
R2=X**2+Z**2
zeta=np.where(R2<a*a, 2*np.sqrt(np.maximum(a*a-R2,0)),0.0)   # vertical extent of the sphere resting on the bed
print("volume of the footprint thickness", zeta.sum()*dx*dx, "sphere", 4/3*np.pi*a**3)
kx=2*np.pi*np.fft.fftfreq(N,d=dx)
KX,KZ=np.meshgrid(kx,kx); K=np.sqrt(KX**2+KZ**2)
zh=np.fft.fft2(np.fft.ifftshift(zeta))
def surface(t, filt=True):
    omega=np.sqrt(g*K*np.tanh(K*h))
    f=1.0/np.cosh(np.minimum(K*h,700)) if filt else 1.0
    eta=np.fft.ifft2(zh*f*np.cos(omega*t)).real
    return np.fft.fftshift(eta)
c=N//2
for t in [0.0,0.05,0.1,0.2,0.3,0.5,0.75,1.0,1.5,2.0]:
    e=surface(t)
    print(f"t {t}: Kajiura centre {e[c,c]:.4f} peak {e.max():.4f} min {e.min():.4f}")
