"""Reference for the multiple scattering of a homogeneous slab (optical thickness T0, single albedo, Henyey-Greenstein g), and a numpy
model of the estimator designed for the path tracer (marching with a weighted reservoir for the step that starts the random walk).

  python3 slab_ref.py            prints the tables

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

def analog(T0, w, g, mu0, mu, N=3000000, half=0.05, seed=7):
    """Brute force: photons of the sun walked with exact exponential flights, absorbed with probability 1 - w and scattered by the
    Henyey-Greenstein function; the BRF at mu is that of the ones that leave the top (reflected) or the bottom (transmitted, the
    unscattered beam left out) within half of mu, any azimuth."""
    rng = np.random.default_rng(seed)
    x = np.zeros((N, 3)); x[:, 2] = T0
    dirn = np.tile([-np.sqrt(1 - mu0 * mu0), 0.0, -mu0], (N, 1))
    alive = np.ones(N, bool); scattered = np.zeros(N, bool)
    ref = 0; tra = 0
    for _ in range(100000):
        if not alive.any():
            break
        idx = np.nonzero(alive)[0]
        t = -np.log(1 - rng.random(len(idx)))
        x[idx] += dirn[idx] * t[:, None]
        z = x[idx, 2]
        top = z >= T0; bot = z <= 0
        out = top | bot
        cos = np.abs(dirn[idx, 2])
        sel = out & (cos > mu - half) & (cos < mu + half)
        ref += int((sel & top & scattered[idx]).sum()); tra += int((sel & bot & scattered[idx]).sum())
        alive[idx[out]] = False
        keep = ~out
        k = idx[keep]
        absorbed = rng.random(len(k)) > w
        alive[k[absorbed]] = False
        k = k[~absorbed]
        c = sample_hg(g, rng, len(k))
        dirn[k] = frame_dir(dirn[k], c, rng)
        scattered[k] = True
    dom = 2 * np.pi * 2 * half
    return np.pi * ref / N / (mu * dom), np.pi * tra / N / (mu * dom)

if __name__ == '__main__':
    T0, mu0, mu = 2.0, 0.8, 0.7
    for w in (1.0, 0.8):
        R, T = exact_isotropic(T0, w, mu0, [mu])
        direct = np.exp(-T0 / mu0)
        print(f"exact isotropic T0={T0} w={w} mu0={mu0} mu={mu}: BRF reflected {R[mu]:.5f}, diffuse transmitted {T[mu]:.5f}, unscattered beam e^-T0/mu0 = {direct:.5f}")
        # single scattering closed form
        r1 = (w / 4) * (1 / (mu + mu0)) * (1 - np.exp(-T0 * (1 / mu + 1 / mu0)))
        print(f"   closed form of the first order, reflected: {r1:.5f}")
        for n in (1, 2, 3, 4, 8, 16, 40):
            r, e = estimate(T0, w, 0.0, mu0, mu, n, 'top', 'sun', paths=200000)
            t, e2 = estimate(T0, w, 0.0, mu0, mu, n, 'bottom', 'sun', paths=200000)
            print(f"   n={n:2d}: reflected {np.pi*r/mu0:.5f} (+-{np.pi*e/mu0:.5f})   transmitted {np.pi*t/mu0:.5f} (+-{np.pi*e2/mu0:.5f})")
    for g in (0.6, -0.5):
        a_r, a_t = analog(T0, 1.0, g, mu0, mu)
        print(f"g={g}, w=1: analog photons reflected {a_r:.4f} transmitted {a_t:.4f}")
        for n in (1, 40):
            r, e = estimate(T0, 1.0, g, mu0, mu, n, 'top', 'sun', paths=200000)
            t, e2 = estimate(T0, 1.0, g, mu0, mu, n, 'bottom', 'sun', paths=200000)
            print(f"   estimator n={n:2d}: reflected {np.pi*r/mu0:.4f} (+-{np.pi*e/mu0:.4f}) transmitted {np.pi*t/mu0:.4f} (+-{np.pi*e2/mu0:.4f})")
