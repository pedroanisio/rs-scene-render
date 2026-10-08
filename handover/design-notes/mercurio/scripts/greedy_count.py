import numpy as np, sys

def greedy_counts(g):
    """g: 3D uint8 array, 0 empty. Returns (occupied, exposed_faces, quads)."""
    occ = int((g > 0).sum())
    pad = np.pad(g, 1)
    faces = 0
    quads = 0
    for axis in range(3):
        for s in (-1, 1):
            # neighbour along axis
            nb = np.roll(pad, -s, axis=axis)
            m = np.where((pad > 0) & (nb == 0), pad, 0)
            m = m[1:-1, 1:-1, 1:-1]
            faces += int((m > 0).sum())
            m = np.moveaxis(m, axis, 0)
            for k in range(m.shape[0]):
                sl = m[k]
                if not sl.any():
                    continue
                nu, nv = sl.shape
                done = np.zeros_like(sl, dtype=bool)
                us, vs = np.nonzero(sl)
                for u, v in zip(us.tolist(), vs.tolist()):
                    if done[u, v]:
                        continue
                    c = sl[u, v]
                    w = 1
                    while v + w < nv and sl[u, v + w] == c and not done[u, v + w]:
                        w += 1
                    h = 1
                    while u + h < nu and (sl[u + h, v:v + w] == c).all() and not done[u + h, v:v + w].any():
                        h += 1
                    done[u:u + h, v:v + w] = True
                    quads += 1
    return occ, faces, quads

def report(name, g):
    occ, faces, quads = greedy_counts(g)
    print(f"{name:34s} cells={occ:8d} faces={faces:8d} quads={quads:8d} faces/cell={faces/occ:6.3f} quads/cell={quads/occ:6.3f} tris/cell={2*quads/occ:6.3f}")

rng = np.random.default_rng(1)
# 1 solid cube
report("solid cube 100^3, 1 colour", np.ones((100,)*3, np.uint8))
# 2 sphere ~1M cells
r = 62.04; n = 130
x = np.arange(n) - n/2 + 0.5
X, Y, Z = np.meshgrid(x, x, x, indexing="ij")
R = np.sqrt(X**2 + Y**2 + Z**2)
sph = (R <= r)
report("solid sphere r=62, 1 colour", sph.astype(np.uint8))
bands = (1 + (np.floor(Z / 8).astype(int) % 4)).astype(np.uint8)
report("solid sphere, 4 horizontal bands", (sph * bands).astype(np.uint8))
noise = rng.integers(1, 17, size=sph.shape).astype(np.uint8)
report("solid sphere, 16 random colours", (sph * noise).astype(np.uint8))
# 3 shell
n2 = 420; x2 = np.arange(n2) - n2/2 + 0.5
X, Y, Z = np.meshgrid(x2, x2, x2, indexing="ij"); R = np.sqrt(X**2+Y**2+Z**2)
shell = (R <= 199.5) & (R > 197.5)
report("sphere shell t=2, 1 colour", shell.astype(np.uint8))
del X, Y, Z, R, shell
# 4 terrain, 316x316 columns avg height ~10
nx = 316
xs = np.arange(nx)
hx = (np.sin(xs / 23.0)[:, None] + np.cos(xs / 31.0)[None, :]) * 3 + np.sin((xs[:, None] + xs[None, :]) / 11.0) * 1.5 + 10
h = np.clip(np.round(hx), 1, None).astype(int)
H = int(h.max()) + 1
zz = np.arange(H)[None, None, :]
terr = (zz < h[:, :, None])
report_n = terr.sum()
report("terrain 316^2 cols, 1 colour", terr.astype(np.uint8))
band = (1 + (zz // 4)).astype(np.uint8) * np.ones_like(terr, dtype=np.uint8)
report("terrain, colour by height/4", (terr * band).astype(np.uint8))
# 5 noise clouds
for dens in (0.5, 0.2):
    cl = (rng.random((40, 40, 40)) < dens)
    report(f"random cloud density {dens}, 1 col", cl.astype(np.uint8))
# 6 checkerboard
i = np.indices((20, 20, 20)).sum(axis=0)
report("checkerboard 20^3", (i % 2 == 0).astype(np.uint8))
