"""Reference for the multiple scattering of a homogeneous slab (optical thickness T0, one albedo, Henyey-Greenstein g), and a numpy model
of the estimator the path tracer uses (the march of the first order, and one step of it chosen by a weighted reservoir to start a random
walk of Henyey-Greenstein scatterings with the lights sampled at every vertex).

  python3 tools/volume_slab_reference.py --tables   the tables of the design (minutes)
  python3 tools/volume_slab_reference.py --check    the golden numbers the tests state, within the noise of each (a minute)

Three independent answers are here: the exact radiance of an isotropic slab (the integral equation of its source function, solved by
product integration), the closed form of the first order, and a brute-force walk of photons resolved in azimuth, which is the oracle for
g other than 0. The model of the estimator is held to them.

Units: extinction 1 per unit length, slab of height T0 (z from 0 to T0), the sun's flux density perpendicular to the beam F = 1, black ground
below the slab. BRF = pi I / (mu0 F): a white Lambertian plate has BRF 1.
"""
import numpy as np, sys

def hg(c, g):
    d = 1 + g * g - 2 * g * c
    return (1 - g * g) / (4 * np.pi * d ** 1.5)

# ------------------------------------------------------------------ exact reference: integral equation of the source function (isotropic)
def exact_isotropic(T0, w, mu0, mus, n=1500):
    """Reflected (z = T0 top, up-going) and transmitted (z = 0, down-going) radiance factors of an isotropic slab."""
    from numpy.polynomial.legendre import leggauss
    # product integration of the kernel E1 on a uniform grid of cells: J_i = w/2 sum_j J_j int_cell_j E1(|t_i - t|) dt + source_i
    h = T0 / n
    t = (np.arange(n) + 0.5) * h
    xs, ws = leggauss(400)
    xs = 0.5 * (xs + 1); ws = 0.5 * ws
    def E2(x):
        # E2(x) = int_0^1 exp(-x / m) dm
        x = np.asarray(x, dtype=float)
        with np.errstate(divide='ignore', over='ignore'):
            return (np.exp(-x[..., None] / xs) * ws).sum(-1)
    # int_a^b E1(|t0-t|) dt for cell [a,b]: antiderivative of E1 is -E2
    A = np.zeros((n, n))
    ti = t[:, None]
    a = (t[None, :] - h / 2); b = (t[None, :] + h / 2)
    # E1 integral from distance x1 to x2 (x2 > x1 >= 0): E2(x1) - E2(x2)
    d1 = np.abs(a - ti); d2 = np.abs(b - ti)
    inside = (a <= ti) & (ti <= b)
    lo = np.minimum(d1, d2); hi = np.maximum(d1, d2)
    A = np.where(inside, (E2(0.0) - E2(d1)) + (E2(0.0) - E2(d2)), E2(lo) - E2(hi))
    src = (w / (4.0 * np.pi)) * np.exp(-(T0 - t) / mu0)         # depth measured from the top: tau = T0 - z
    # tau is depth from the top; the grid index is z; the kernel is symmetric so the equation is the same in z
    J = np.linalg.solve(np.eye(n) - (w / 2.0) * A, src)
    R, T = {}, {}
    for mu in mus:
        # up-going radiance at the top: I = 1/mu sum J(z) exp(-(T0 - z)/mu) h
        I = (J * np.exp(-(T0 - t) / mu)).sum() * h / mu
        R[mu] = np.pi * I / mu0
        # down-going at the bottom (diffuse part): I = 1/mu sum J exp(-z/mu) h
        I = (J * np.exp(-t / mu)).sum() * h / mu
        T[mu] = np.pi * I / mu0
    return R, T

# ------------------------------------------------------------------ the estimator of the design, in numpy
def sample_hg(g, rng, n):
    u = rng.random(n)
    if abs(g) < 1e-3:
        c = 1 - 2 * u
    else:
        s = (1 - g * g) / (1 - g + 2 * g * u)
        c = (1 + g * g - s * s) / (2 * g)
    return np.clip(c, -1, 1)

def frame_dir(o, c, rng):
    """A unit vector at angle acos(c) from o (rows), uniform in azimuth."""
    n = len(o)
    ph = 2 * np.pi * rng.random(n)
    s = np.sqrt(np.maximum(0, 1 - c * c))
    a = np.where(np.abs(o[:, 2:3]) < 0.9, np.array([[0, 0, 1.0]]), np.array([[1.0, 0, 0]]))
    u = np.cross(o, a); u /= np.linalg.norm(u, axis=1, keepdims=True)
    v = np.cross(o, u)
    return c[:, None] * o + s[:, None] * (np.cos(ph)[:, None] * u + np.sin(ph)[:, None] * v)

def estimate(T0, w, g, mu0, mu, nmax, side, mode, paths=400000, steps=48, seed=1):
    """Pixel value of the design's estimator for orders <= nmax; side 'top' (reflection) or 'bottom' (transmission).
    mode 'sun': collimated sun, BRF; mode 'furnace': environment of radiance 1 everywhere, no sun, returns the radiance."""
    rng = np.random.default_rng(seed)
    N = paths
    sun = np.array([np.sqrt(1 - mu0 * mu0), 0.0, mu0])        # direction toward the sun
    # the camera ray: toward the slab. top: travels down; bottom: travels up. d = direction of travel; o = -d
    sgn = -1.0 if side == 'top' else 1.0
    d = np.array([np.sqrt(1 - mu * mu), 0.0, sgn * mu])
    o = -d
    L = T0 / mu                                       # path length through the slab
    z_in = T0 if side == 'top' else 0.0
    ds = L / steps
    s_mid = (np.arange(steps) + 0.5) * ds
    P = np.array([0.0, 0.0, z_in]) + d[None, :] * s_mid[:, None]       # (steps, 3)
    trans = np.exp(-(np.arange(steps) * ds))
    u = trans * (1 - np.exp(-ds))                       # extinction 1
    U = u.sum()

    def inside(p):
        return (p[:, 2] > 0) & (p[:, 2] < T0)

    def L1(p, out):
        """Radiance scattered into direction `out` (rows) at points p (rows) by what the lights give directly (the order-1 term)."""
        if mode == 'sun':
            # visibility: sun ray from p leaves through the top; optical depth (T0 - z) / mu0
            vis = np.exp(-(T0 - p[:, 2]) / mu0)
            # light travels along -sun; cos between the light's travel direction and the outgoing direction
            c = (-sun[None, :] * out).sum(1)
            return 1.0 * hg(c, g) * vis
        # furnace: one uniform direction, as the shader does: 4 pi phase visible env(=1)
        dirs = frame_dir(np.tile([0, 0, 1.0], (len(p), 1)), 1 - 2 * rng.random(len(p)), rng)
        up = dirs[:, 2]
        dist = np.where(up > 0, (T0 - p[:, 2]) / np.maximum(up, 1e-12), p[:, 2] / np.maximum(-up, 1e-12))
        c = (-dirs * out).sum(1)
        return 4 * np.pi * hg(c, g) * np.exp(-dist)

    # order 1: the quadrature over the steps, as the march does (the shader draws one environment direction at each step)
    first = np.zeros(N)
    for k in range(steps):
        p = np.tile(P[k], (N, 1))
        first += u[k] * w * L1(p, np.tile(o, (N, 1)))
    total = first
    if nmax > 1:
        # the reservoir: the step that starts the walk is chosen with probability u_k / U (all the albedos are equal here)
        k = rng.choice(steps, size=N, p=u / U)
        x = P[k]
        out = np.tile(o, (N, 1))
        beta = np.full(N, w * U)           # the weight of the chosen step: w * U, since the selection probability is u_k / U
        alive = np.ones(N, bool)
        acc = np.zeros(N)
        for j in range(nmax - 1):
            c = sample_hg(g, rng, N)
            ell = frame_dir(out, c, rng)             # direction the light travels, at angle acos(c) from the outgoing direction
            to = -ell                                 # from the vertex toward the previous one of the walk
            t = -np.log(1 - rng.random(N))           # free flight, extinction 1
            y = x + to * t[:, None]
            alive &= inside(y)
            if not alive.any():
                break
            beta = np.where(alive, beta * w, 0.0)
            acc += np.where(alive, beta * L1(y, -to), 0.0)
            x, out = y, -to
        total = first + acc
    return total.mean(), total.std() / np.sqrt(N)


def analog_direction(T0, w, g, mu0, mu, N=4000000, delta=0.15, seed=11):
    """Brute force: photons of the sun walked with exact exponential flights, absorbed with probability 1 - w, scattered by the
    Henyey-Greenstein function. The BRF (reflected, transmitted) of the scattered ones that leave within `delta` radians of the direction
    toward a camera at cosine `mu` in the plane of the sun (the camera's azimuth is that of the sun's, as in `estimate`)."""
    rng = np.random.default_rng(seed)
    x = np.zeros((N, 3)); x[:, 2] = T0
    dirn = np.tile([-np.sqrt(1 - mu0 * mu0), 0.0, -mu0], (N, 1))
    alive = np.ones(N, bool); scattered = np.zeros(N, bool)
    sn = np.sqrt(1 - mu * mu)
    targets = {'top': np.array([-sn, 0, mu]), 'bottom': np.array([-sn, 0, -mu])}
    count = {'top': 0, 'bottom': 0}; cd = np.cos(delta)
    while alive.any():
        idx = np.nonzero(alive)[0]
        t = -np.log(1 - rng.random(len(idx)))
        x[idx] += dirn[idx] * t[:, None]
        z = x[idx, 2]; top = z >= T0; bot = z <= 0; out = top | bot
        for name, m in (('top', top), ('bottom', bot)):
            sel = out & m & scattered[idx]
            if sel.any():
                count[name] += int(((dirn[idx[sel]] * targets[name]).sum(1) > cd).sum())
        alive[idx[out]] = False
        k = idx[~out]
        absorbed = rng.random(len(k)) > w
        alive[k[absorbed]] = False
        k = k[~absorbed]
        dirn[k] = frame_dir(dirn[k], sample_hg(g, rng, len(k)), rng)
        scattered[k] = True
    dom = 2 * np.pi * (1 - cd)
    return np.pi * count['top'] / N / (mu * dom), np.pi * count['bottom'] / N / (mu * dom)

def first_order(T0, w, mu0, mu):
    """Closed form of the reflected BRF of the first order (isotropic)."""
    return w / (4 * (mu + mu0)) * (1 - np.exp(-T0 * (1 / mu + 1 / mu0)))

T0, MU0, MU = 2.0, 0.8, 0.7

def tables():
    for w in (1.0, 0.8):
        R, T = exact_isotropic(T0, w, MU0, [MU])
        print(f"exact isotropic T0={T0} w={w} mu0={MU0} mu={MU}: reflected {R[MU]:.5f}, diffuse transmitted {T[MU]:.5f}; closed form of the first order {first_order(T0, w, MU0, MU):.5f}")
        for n in (1, 2, 3, 4, 8, 16, 40):
            r, e = estimate(T0, w, 0.0, MU0, MU, n, 'top', 'sun', paths=200000)
            t, e2 = estimate(T0, w, 0.0, MU0, MU, n, 'bottom', 'sun', paths=200000)
            print(f"   n={n:2d}: reflected {np.pi*r/MU0:.5f} (+-{np.pi*e/MU0:.5f})   transmitted {np.pi*t/MU0:.5f} (+-{np.pi*e2/MU0:.5f})")
    for g in (0.0, 0.6, -0.5):
        a = analog_direction(T0, 1.0, g, MU0, MU)
        r, e = estimate(T0, 1.0, g, MU0, MU, 40, 'top', 'sun', paths=200000)
        t, e2 = estimate(T0, 1.0, g, MU0, MU, 40, 'bottom', 'sun', paths=200000)
        print(f"g={g}: photons reflected {a[0]:.4f} transmitted {a[1]:.4f} | estimator n=40 reflected {np.pi*r/MU0:.4f}+-{np.pi*e/MU0:.4f} transmitted {np.pi*t/MU0:.4f}+-{np.pi*e2/MU0:.4f}")
    bg = np.exp(-T0 / MU)
    for w, g in ((1.0, 0.0), (1.0, 0.6), (0.8, 0.0)):
        row = []
        for n in (1, 2, 3, 5, 10, 40):
            r, e = estimate(T0, w, g, MU0, MU, n, 'top', 'furnace', paths=100000, steps=32)
            row.append(f"n={n}: {bg + r:.4f}+-{e:.4f}")
        print(f"furnace w={w} g={g} (unscattered {bg:.4f}):", " | ".join(row))

def check():
    """The golden numbers of the design, each within what its noise allows. Exits with an error if one is not."""
    failures = []
    def close(name, got, want, tol):
        ok = abs(got - want) <= tol
        print(f"{'ok  ' if ok else 'FAIL'} {name}: {got:.5f} (golden {want:.5f} +- {tol})")
        if not ok:
            failures.append(name)
    for w, refl, trans, first in ((1.0, 0.56361, 0.36564, 0.16588), (0.8, 0.29005, 0.15644, 0.13270)):
        R, T = exact_isotropic(T0, w, MU0, [MU])
        close(f"exact reflected w={w}", R[MU], refl, 2e-5)
        close(f"exact transmitted w={w}", T[MU], trans, 2e-5)
        close(f"closed form first order w={w}", first_order(T0, w, MU0, MU), first, 2e-5)
        r1, _ = estimate(T0, w, 0.0, MU0, MU, 1, 'top', 'sun', paths=2000)
        close(f"estimator n=1 w={w}", np.pi * r1 / MU0, first, 1e-4)
        r, e = estimate(T0, w, 0.0, MU0, MU, 40, 'top', 'sun', paths=100000)
        close(f"estimator n=40 reflected w={w}", np.pi * r / MU0, refl, 5 * np.pi * e / MU0 + 2e-3)
        t, e2 = estimate(T0, w, 0.0, MU0, MU, 40, 'bottom', 'sun', paths=100000)
        close(f"estimator n=40 transmitted w={w}", np.pi * t / MU0, trans, 5 * np.pi * e2 / MU0 + 2e-3)
    for g, refl, trans in ((0.6, 0.389, 1.177), (-0.5, 0.457, 0.281)):
        a = analog_direction(T0, 1.0, g, MU0, MU, N=1500000)
        r, e = estimate(T0, 1.0, g, MU0, MU, 40, 'top', 'sun', paths=100000)
        t, e2 = estimate(T0, 1.0, g, MU0, MU, 40, 'bottom', 'sun', paths=100000)
        close(f"photons reflected g={g}", a[0], refl, 0.02)
        close(f"estimator vs photons reflected g={g}", np.pi * r / MU0, a[0], 0.02)
        # the cone of the photons smooths the forward peak of g = 0.6 (3 % in the transmitted radiance)
        close(f"estimator vs photons transmitted g={g}", np.pi * t / MU0, a[1], 0.05 * a[1] + 0.01)
    bg = np.exp(-T0 / MU)
    r, e = estimate(T0, 1.0, 0.0, MU0, MU, 40, 'top', 'furnace', paths=60000, steps=32)
    close("furnace n=40 w=1 (radiance is 1)", bg + r, 1.0, 5 * e + 0.01)
    r, _ = estimate(T0, 1.0, 0.0, MU0, MU, 1, 'top', 'furnace', paths=20000, steps=32)
    close("furnace n=1 w=1 (the deficit of one scattering)", bg + r, 0.3003, 0.003)
    if failures:
        sys.exit(f"{len(failures)} golden numbers differ: {failures}")
    print("all golden numbers hold")

if __name__ == '__main__':
    np.seterr(all='ignore')  # walks that left the slab are masked out after their values are computed
    if '--check' in sys.argv:
        check()
    elif '--tables' in sys.argv:
        tables()
    else:
        sys.exit(__doc__)
