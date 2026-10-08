import numpy as np
from greedy import faces_and_quads
rng=np.random.default_rng(1)
for k in (2,4,16):
    g=rng.integers(1,k+1,(100,100,100)).astype(np.int32); print("random",k,"colours 100^3",faces_and_quads(g))
# wall 200x4x1250 ~1M cells? use 160x6x1000 = 0.96M with brick pattern colours: two colours alternating in 8x4 bricks
n=(160,6,1000)
g=np.ones(n,np.int32)
X,Y,Z=np.meshgrid(*[np.arange(s) for s in n],indexing='ij')
g=(1+(((X//8)+(Z//4))%2)).astype(np.int32)
print("wall 160x6x1000 two-colour 8x4 tiles",int(g.size),faces_and_quads(g))
g=np.ones(n,np.int32); print("wall uniform",faces_and_quads(g))
