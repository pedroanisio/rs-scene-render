//! SREP 75: escape-time fractal assets (`<fractal>`), at any depth of zoom.
//!
//! The SREP fixes what each pixel shows, not how it is computed: the escape count of the pixel's point in exact
//! arithmetic, allowing only for points within 2⁻²⁰ of a pixel (Semantics 3). This module computes it on the CPU, in
//! IEEE double arithmetic that is the same on every machine, by one of two methods:
//!
//! - **Direct f64**, while the pixel is large next to the rounding of the plane's coordinates: when
//!   2⁻⁵³ · |c| ≤ 2⁻²⁰ · p for every point c of the image (p is the pixel size), rounding c to a double moves it less
//!   than the allowance. That is p ≥ 2⁻³³ · max |c|.
//! - **Perturbation** (K. I. Martin, "SuperFractalThing Maths", 2013; cited from memory) below that: one reference orbit
//!   Z_n computed in binary fixed point with enough bits for the zoom, from the centre read exactly from its decimal
//!   text; each pixel iterates its difference δ from the reference in doubles, δ_{n+1} = 2 Z_n δ_n + δ_n² + Δc, and
//!   an extended exponent (a double with a separate integer exponent) once the pixel size leaves the range of doubles.
//!
//! **Glitches.** A pixel whose orbit runs close to 0 while the reference does not loses its precision in δ. This module
//! avoids them by rebasing (Zhuoran, fractalforums.org, 2021; cited from memory): whenever |z| < |δ| for the full value
//! z = Z_m + δ, or the reference ends, the pixel restarts on a reference orbit that starts at the critical point 0, with
//! δ := z − (that orbit's start). For a Mandelbrot asset that orbit is the reference itself (Z_0 = 0); for a Julia
//! asset it is a second high-precision orbit, of 0 under z² + k. No pixel then needs a glitch test or a second pass.
//!
//! The method leaves every count within Semantics 3 only up to the precision argument above; the SREP's own kit, whose
//! counts were computed in decimal arithmetic at 60 and 90 digits, is the check (`tests/srep75_fractal.rs`).

use num_bigint::BigInt;
use rayon::prelude::*;

/// What a pixel shows: the escape count n ≥ 1 and |z_n|, or inside (no escape within `maxIterations`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Escape {
    /// No n ≤ `maxIterations` with |z_n| > bailout.
    Inside,
    /// The smallest n ≥ 1 with |z_n| > bailout, and |z_n|.
    Out { n: u64, abs: f64 },
}

impl Escape {
    /// The count, or None inside.
    pub fn count(self) -> Option<u64> {
        match self {
            Escape::Inside => None,
            Escape::Out { n, .. } => Some(n),
        }
    }
}

/// The parameters of a fractal asset at one frame (its animated values evaluated).
#[derive(Debug, Clone, PartialEq)]
pub struct Fractal {
    /// `None` for a Mandelbrot asset; the Julia constant (juliaX, juliaY) as decimal text for a Julia asset.
    pub julia: Option<[String; 2]>,
    pub width: u32,
    pub height: u32,
    /// centerX and centerY as written: exact decimals.
    pub center: [String; 2],
    pub span: f64,
    pub zoom: f64,
    /// Degrees, clockwise on screen.
    pub rotation: f64,
    pub max_iterations: u64,
    pub bailout: f64,
}

/// Which method computed an image (for statistics and tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Direct iteration in doubles.
    Direct,
    /// Perturbation with double deltas.
    Perturbation,
    /// Perturbation with extended-exponent deltas.
    PerturbationExtended,
}

// ------------------------------------------------------------------ exact decimals in binary fixed point

/// A number `v / 2^bits` in binary fixed point.
#[derive(Debug, Clone, PartialEq)]
struct Fixed {
    v: BigInt,
}

/// Rounds the `xs:decimal` text `s` (an optional sign, digits, an optional point and digits, no exponent) to the
/// nearest multiple of 2^-bits, without passing through a binary float. None when `s` is not a decimal.
fn decimal_fixed(s: &str, bits: u64) -> Option<Fixed> {
    let s = s.trim();
    let (neg, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    if (int.is_empty() && frac.is_empty()) || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits: BigInt = format!("0{int}{frac}").parse().ok()?;
    let scale = BigInt::from(10u32).pow(frac.len() as u32);
    // round(digits · 2^bits / 10^k), half away from zero
    let num = (digits << bits as usize) * 2 + &scale;
    let mut v: BigInt = num / (scale * 2);
    if neg {
        v = -v;
    }
    Some(Fixed { v })
}

/// The nearest double to `v / 2^bits` (to within one rounding of the leading 64 bits).
fn fixed_to_f64(v: &BigInt, bits: u64) -> f64 {
    let len = v.bits();
    if len == 0 {
        return 0.0;
    }
    // keep the top 64 bits, then scale by the power of two (exact)
    let cut = len.saturating_sub(64);
    let top: i128 = i128::try_from(v >> cut as usize).unwrap_or(0);
    let e = cut as i64 - bits as i64;
    top as f64 * pow2(e)
}

/// 2^e as a double (0 or infinity outside the range).
fn pow2(e: i64) -> f64 {
    let mut x = 1.0f64;
    let mut e = e;
    // powi on parts that each stay in range
    while e > 1000 {
        x *= 2f64.powi(1000);
        e -= 1000;
    }
    while e < -1000 {
        x *= 2f64.powi(-1000);
        e += 1000;
    }
    x * 2f64.powi(e as i32)
}

/// A high-precision orbit, rounded to doubles: z_0 = `start`, z_{n+1} = z_n² + `add`, computed in fixed point with
/// `bits` fraction bits, until |z| exceeds `escape` (that point included) or `limit` points after the start.
fn orbit(start: (&BigInt, &BigInt), add: (&BigInt, &BigInt), bits: u64, limit: u64, escape: f64) -> Vec<(f64, f64)> {
    let (mut x, mut y) = (start.0.clone(), start.1.clone());
    let mut out = Vec::with_capacity(limit.min(1 << 20) as usize + 1);
    let r2 = escape * escape;
    out.push((fixed_to_f64(&x, bits), fixed_to_f64(&y, bits)));
    for _ in 0..limit {
        let xx = (&x * &x) >> bits as usize;
        let yy = (&y * &y) >> bits as usize;
        let xy = (&x * &y) >> (bits as usize - 1);
        x = xx - yy + add.0;
        y = xy + add.1;
        let z = (fixed_to_f64(&x, bits), fixed_to_f64(&y, bits));
        out.push(z);
        let r = z.0 * z.0 + z.1 * z.1;
        if r > r2 || r.is_nan() {
            break;
        }
    }
    out
}

// ------------------------------------------------------------------ the delta arithmetic

/// The real numbers the deltas are kept in: doubles, or doubles with an extended exponent.
trait Real: Copy + Send + Sync {
    fn from_f64(v: f64) -> Self;
    fn to_f64(self) -> f64;
    fn add(self, o: Self) -> Self;
    fn sub(self, o: Self) -> Self;
    fn mul(self, o: Self) -> Self;
    fn twice(self) -> Self;
    /// self < o, both non-negative.
    fn less(self, o: Self) -> bool;
}

impl Real for f64 {
    fn from_f64(v: f64) -> Self {
        v
    }
    fn to_f64(self) -> f64 {
        self
    }
    fn add(self, o: Self) -> Self {
        self + o
    }
    fn sub(self, o: Self) -> Self {
        self - o
    }
    fn mul(self, o: Self) -> Self {
        self * o
    }
    fn twice(self) -> Self {
        2.0 * self
    }
    fn less(self, o: Self) -> bool {
        self < o
    }
}

/// `m · 2^e` with |m| in [1, 2) or m = 0: a double's precision with an exponent that does not underflow.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FExp {
    m: f64,
    e: i64,
}

impl FExp {
    fn norm(m: f64, e: i64) -> FExp {
        if m == 0.0 || !m.is_finite() {
            return FExp { m, e: 0 };
        }
        let bits = m.to_bits();
        let be = ((bits >> 52) & 0x7ff) as i64;
        if be == 0 {
            // subnormal mantissa: scale it up first
            return FExp::norm(m * 2f64.powi(64), e - 64);
        }
        let shift = be - 1023;
        let m = f64::from_bits((bits & !(0x7ff << 52)) | (1023 << 52));
        FExp { m, e: e + shift }
    }

    /// Scaled by 2^e.
    fn of(v: f64, e: i64) -> FExp {
        FExp::norm(v, e)
    }
}

impl Real for FExp {
    fn from_f64(v: f64) -> Self {
        FExp::norm(v, 0)
    }
    fn to_f64(self) -> f64 {
        if self.m == 0.0 {
            return 0.0;
        }
        self.m * pow2(self.e)
    }
    fn add(self, o: Self) -> Self {
        if self.m == 0.0 {
            return o;
        }
        if o.m == 0.0 {
            return self;
        }
        let (a, b) = if self.e >= o.e { (self, o) } else { (o, self) };
        let d = a.e - b.e;
        if d > 120 {
            return a;
        }
        FExp::norm(a.m + b.m * 2f64.powi(-(d as i32)), a.e)
    }
    fn sub(self, o: Self) -> Self {
        self.add(FExp { m: -o.m, e: o.e })
    }
    fn mul(self, o: Self) -> Self {
        FExp::norm(self.m * o.m, self.e + o.e)
    }
    fn twice(self) -> Self {
        FExp { m: self.m, e: self.e + 1 }
    }
    fn less(self, o: Self) -> bool {
        if self.m == 0.0 {
            return o.m != 0.0;
        }
        if o.m == 0.0 {
            return false;
        }
        (self.e, self.m) < (o.e, o.m)
    }
}

#[derive(Clone, Copy)]
struct C<T> {
    re: T,
    im: T,
}

impl<T: Real> C<T> {
    fn norm2(self) -> T {
        self.re.mul(self.re).add(self.im.mul(self.im))
    }
}

// ------------------------------------------------------------------ the image

/// The pixel size p and the offset of pixel (i, j) from the centre, R(rotation) · ((i + 0.5 − W/2) p, −(j + 0.5 − H/2) p),
/// as (re, im) mantissas and a shared power of two: offset = (re, im) · 2^e.
struct Plane {
    /// p = p_m · 2^p_e
    p_m: f64,
    p_e: i64,
    cos: f64,
    sin: f64,
    w: u32,
    h: u32,
}

impl Plane {
    fn new(f: &Fractal) -> Plane {
        // log2 p = log2 span − zoom · log2 10 − log2 W, split into a mantissa in [1, 2) and an integer exponent
        let l = f.span.log2() - f.zoom * std::f64::consts::LOG2_10 - (f.width as f64).log2();
        let e = l.floor();
        let (cos, sin) = {
            let r = f.rotation.to_radians();
            (r.cos(), r.sin())
        };
        Plane { p_m: (l - e).exp2(), p_e: e as i64, cos, sin, w: f.width, h: f.height }
    }

    /// log2 of the pixel size.
    fn log2_p(&self) -> f64 {
        self.p_e as f64 + self.p_m.log2()
    }

    /// The offset of pixel (i, j) in pixel units, rotated: multiply by p for the plane.
    fn offset(&self, i: u32, j: u32) -> (f64, f64) {
        let u = i as f64 + 0.5 - self.w as f64 / 2.0;
        let v = -(j as f64 + 0.5 - self.h as f64 / 2.0);
        // clockwise on screen, where the imaginary axis points up
        (u * self.cos + v * self.sin, -u * self.sin + v * self.cos)
    }

    fn delta<T: Real>(&self, i: u32, j: u32) -> C<T>
    where
        T: FromScaled,
    {
        let (a, b) = self.offset(i, j);
        C { re: T::scaled(a * self.p_m, self.p_e), im: T::scaled(b * self.p_m, self.p_e) }
    }
}

/// `v · 2^e` in the delta arithmetic.
trait FromScaled {
    fn scaled(v: f64, e: i64) -> Self;
}

impl FromScaled for f64 {
    fn scaled(v: f64, e: i64) -> Self {
        v * pow2(e)
    }
}

impl FromScaled for FExp {
    fn scaled(v: f64, e: i64) -> Self {
        FExp::of(v, e)
    }
}

/// The method this module uses for `f`.
pub fn method(f: &Fractal) -> Method {
    let plane = Plane::new(f);
    let lp = plane.log2_p();
    let reach = centre_magnitude(f) + (f.width.max(f.height) as f64) * lp.exp2();
    if lp >= -33.0 + reach.max(1.0).log2() {
        Method::Direct
    } else if lp > -960.0 {
        Method::Perturbation
    } else {
        Method::PerturbationExtended
    }
}

/// max(|centerX|, |centerY|) + 1 as a double: the size of the plane's coordinates (for the Julia constant too).
fn centre_magnitude(f: &Fractal) -> f64 {
    let v = |s: &str| decimal_fixed(s, 64).map(|x| fixed_to_f64(&x.v, 64).abs()).unwrap_or(0.0);
    let mut m = v(&f.center[0]).max(v(&f.center[1]));
    if let Some(k) = &f.julia {
        m = m.max(v(&k[0])).max(v(&k[1]));
    }
    m
}

/// Every pixel's escape, row by row from the top-left.
pub fn escapes(f: &Fractal) -> Result<Vec<Escape>, String> {
    let rows: Vec<u32> = (0..f.height).collect();
    let solver = Solver::new(f)?;
    let out: Vec<Vec<Escape>> = rows.par_iter().map(|&j| (0..f.width).map(|i| solver.pixel(i, j)).collect()).collect();
    Ok(out.into_iter().flatten().collect())
}

/// The escapes of the pixels `at` alone, (i, j) from the top-left.
pub fn escapes_at(f: &Fractal, at: &[(u32, u32)]) -> Result<Vec<Escape>, String> {
    let solver = Solver::new(f)?;
    Ok(at.iter().map(|&(i, j)| solver.pixel(i, j)).collect())
}

/// What a pixel needs: the method, the plane, and the reference orbits of perturbation.
struct Solver {
    method: Method,
    plane: Plane,
    julia: bool,
    max: u64,
    bailout2: f64,
    /// The centre (direct method) and the Julia constant, as doubles.
    centre: (f64, f64),
    k: (f64, f64),
    /// The orbit of the centre: z_0 = 0, z_{n+1} = z_n² + centre (Mandelbrot); z_0 = centre, z_{n+1} = z_n² + k (Julia).
    reference: Vec<(f64, f64)>,
    /// The orbit of the critical point 0 under z² + k (Julia), which a pixel rebases onto.
    critical: Vec<(f64, f64)>,
}

impl Solver {
    fn new(f: &Fractal) -> Result<Solver, String> {
        if !(f.span > 0.0 && f.span.is_finite() && f.zoom.is_finite() && f.rotation.is_finite()) {
            return Err("fractal: span, zoom and rotation must be finite, span positive".into());
        }
        if !(f.bailout > 0.0 && f.bailout.is_finite()) {
            return Err("fractal: bailout must be positive and finite".into());
        }
        let plane = Plane::new(f);
        let method = method(f);
        // fraction bits: the pixel's own, the allowance of 2^-20 and a margin of 64 bits
        let bits = ((-plane.log2_p()).max(0.0).ceil() as u64) + 20 + 64;
        let parse = |s: &str, what: &str| {
            decimal_fixed(s, bits).ok_or_else(|| format!("fractal: {what} {s:?} is not a decimal"))
        };
        let cx = parse(&f.center[0], "centerX")?;
        let cy = parse(&f.center[1], "centerY")?;
        let k = match &f.julia {
            Some([x, y]) => Some((parse(x, "juliaX")?, parse(y, "juliaY")?)),
            None => None,
        };
        let centre = (fixed_to_f64(&cx.v, bits), fixed_to_f64(&cy.v, bits));
        let kf = k.as_ref().map(|(x, y)| (fixed_to_f64(&x.v, bits), fixed_to_f64(&y.v, bits))).unwrap_or((0.0, 0.0));
        // orbits stop once well past the bailout: a pixel near the reference escapes with it
        let escape = f.bailout.max(2.0) * 4.0;
        // a pixel rebases when it reaches the end of a reference, so a reference need not be as long as the iteration:
        // capped, it holds 2^22 points (64 MiB)
        let limit = f.max_iterations.min(1 << 22);
        let (mut reference, mut critical) = (Vec::new(), Vec::new());
        if method != Method::Direct {
            let zero = BigInt::from(0);
            match &k {
                None => reference = orbit((&zero, &zero), (&cx.v, &cy.v), bits, limit, escape),
                Some((kx, ky)) => {
                    reference = orbit((&cx.v, &cy.v), (&kx.v, &ky.v), bits, limit, escape);
                    critical = orbit((&zero, &zero), (&kx.v, &ky.v), bits, limit, escape);
                }
            }
        }
        Ok(Solver {
            method,
            plane,
            julia: f.julia.is_some(),
            max: f.max_iterations,
            bailout2: f.bailout * f.bailout,
            centre,
            k: kf,
            reference,
            critical,
        })
    }

    fn pixel(&self, i: u32, j: u32) -> Escape {
        match self.method {
            Method::Direct => self.direct(i, j),
            Method::Perturbation => self.perturbed::<f64>(i, j),
            Method::PerturbationExtended => self.perturbed::<FExp>(i, j),
        }
    }

    fn direct(&self, i: u32, j: u32) -> Escape {
        let (a, b) = self.plane.offset(i, j);
        let p = self.plane.p_m * pow2(self.plane.p_e);
        let c = (self.centre.0 + a * p, self.centre.1 + b * p);
        let (mut x, mut y, add) = if self.julia { (c.0, c.1, self.k) } else { (0.0, 0.0, c) };
        for n in 1..=self.max {
            let xn = x * x - y * y + add.0;
            y = 2.0 * x * y + add.1;
            x = xn;
            let r2 = x * x + y * y;
            if r2 > self.bailout2 {
                return Escape::Out { n, abs: r2.sqrt() };
            }
        }
        Escape::Inside
    }

    fn perturbed<T: Real + FromScaled>(&self, i: u32, j: u32) -> Escape {
        let offset: C<T> = self.plane.delta(i, j);
        let zero = T::from_f64(0.0);
        // Mandelbrot: δ_0 = 0 and Δc is the offset; Julia: δ_0 is the offset and nothing is added
        let (mut d, dc) =
            if self.julia { (offset, C { re: zero, im: zero }) } else { (C { re: zero, im: zero }, offset) };
        let mut orbit = &self.reference;
        let mut m = 0usize;
        for n in 1..=self.max {
            let (zr, zi) = orbit[m];
            let (zr, zi) = (T::from_f64(zr), T::from_f64(zi));
            // δ' = 2 Z δ + δ² + Δc
            let re = zr.mul(d.re).sub(zi.mul(d.im)).twice().add(d.re.mul(d.re).sub(d.im.mul(d.im))).add(dc.re);
            let im = zr.mul(d.im).add(zi.mul(d.re)).twice().add(d.re.mul(d.im).twice()).add(dc.im);
            d = C { re, im };
            m += 1;
            let (zr, zi) = orbit[m];
            let z = C { re: T::from_f64(zr).add(d.re), im: T::from_f64(zi).add(d.im) };
            let r2 = z.norm2().to_f64();
            if r2 > self.bailout2 {
                return Escape::Out { n, abs: r2.sqrt() };
            }
            // rebase onto the orbit of the critical point when z is nearer 0 than the reference, or when the
            // reference ends
            if z.norm2().less(d.norm2()) || m + 1 >= orbit.len() {
                orbit = if self.julia { &self.critical } else { &self.reference };
                // that orbit starts at 0: δ := z
                d = z;
                m = 0;
            }
        }
        Escape::Inside
    }
}

// ------------------------------------------------------------------ colour

/// How escape counts become colours (SREP 75 Semantics 4).
#[derive(Debug, Clone, PartialEq)]
pub struct Colouring {
    /// `smooth` (true) or `bands`.
    pub smooth: bool,
    /// Display-encoded RGBA in [0, 1].
    pub palette: Vec<[f64; 4]>,
    pub scale: f64,
    pub offset: f64,
    pub inside: [f64; 4],
    pub bailout: f64,
}

impl Colouring {
    /// The display-encoded colour of one pixel.
    pub fn colour(&self, e: Escape) -> [f64; 4] {
        let l = self.palette.len();
        let Escape::Out { n, abs } = e else { return self.inside };
        if l == 0 {
            return self.inside;
        }
        if !self.smooth {
            return self.palette[(n % l as u64) as usize];
        }
        // ν = n + 1 − log2(ln|z_n| / ln bailout), u = ν · scale + offset
        let nu = n as f64 + 1.0 - (abs.ln() / self.bailout.ln()).log2();
        let u = nu * self.scale + self.offset;
        if !u.is_finite() {
            return self.palette[(n % l as u64) as usize];
        }
        let fl = u.floor();
        let t = u - fl;
        let k = fl.rem_euclid(l as f64) as usize % l;
        let (a, b) = (self.palette[k], self.palette[(k + 1) % l]);
        std::array::from_fn(|c| a[c] + (b[c] - a[c]) * t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_are_read_exactly_into_fixed_point() {
        let x = decimal_fixed("-0.75", 8).unwrap();
        assert_eq!(x.v, BigInt::from(-192));
        assert_eq!(decimal_fixed("+1.", 4).unwrap().v, BigInt::from(16));
        assert_eq!(decimal_fixed(".5", 1).unwrap().v, BigInt::from(1));
        // 0.1 is not a binary fraction: the nearest multiple of 2^-8 is 26/256
        assert_eq!(decimal_fixed("0.1", 8).unwrap().v, BigInt::from(26));
        for bad in ["", ".", "1e5", "--1", "0x1", "1.2.3"] {
            assert!(decimal_fixed(bad, 8).is_none(), "{bad}");
        }
        // a long decimal keeps its digits beyond a double
        let a = decimal_fixed("0.1000000000000000000000000000001", 200).unwrap();
        let b = decimal_fixed("0.1", 200).unwrap();
        assert!(a.v > b.v);
        assert_eq!(fixed_to_f64(&a.v, 200), 0.1);
    }

    #[test]
    fn the_extended_exponent_keeps_numbers_below_the_range_of_doubles() {
        let tiny = FExp::of(1.5, -2000);
        let sq = tiny.mul(tiny);
        assert_eq!((sq.m, sq.e), (1.125, -3999));
        assert_eq!(tiny.add(tiny), FExp::of(1.5, -1999));
        assert!(sq.less(tiny));
        assert!(!tiny.less(sq));
        assert_eq!(FExp::from_f64(-3.0).to_f64(), -3.0);
        assert_eq!(FExp::of(1.0, 10).sub(FExp::of(1.0, 10)).to_f64(), 0.0);
        assert_eq!(FExp::from_f64(0.75).twice().to_f64(), 1.5);
    }

    fn mandelbrot(zoom: f64, max: u64) -> Fractal {
        Fractal {
            julia: None,
            width: 64,
            height: 36,
            center: ["-0.743643887037158704752191506114774".into(), "0.131825904205311970493132056385139".into()],
            span: 4.0,
            zoom,
            rotation: 0.0,
            max_iterations: max,
            bailout: 2.0,
        }
    }

    #[test]
    fn the_method_follows_the_pixel_size() {
        assert_eq!(method(&mandelbrot(0.0, 10)), Method::Direct);
        assert_eq!(method(&mandelbrot(6.0, 10)), Method::Direct);
        assert_eq!(method(&mandelbrot(12.0, 10)), Method::Perturbation);
        assert_eq!(method(&mandelbrot(20.0, 10)), Method::Perturbation);
        assert_eq!(method(&mandelbrot(400.0, 10)), Method::PerturbationExtended);
    }

    #[test]
    fn perturbation_agrees_with_direct_iteration_where_both_hold() {
        // at zoom 3 the direct method is exact to the allowance; perturbation, forced, gives the same counts
        let f = mandelbrot(3.0, 2000);
        let direct = Solver::new(&f).unwrap();
        assert_eq!(direct.method, Method::Direct);
        let mut forced = Solver::new(&mandelbrot(12.0, 2000)).unwrap();
        forced.plane = Plane::new(&f);
        let mut forced_ext = Solver::new(&mandelbrot(12.0, 2000)).unwrap();
        forced_ext.plane = Plane::new(&f);
        let mut differ = 0;
        for j in (0..36).step_by(5) {
            for i in (0..64).step_by(7) {
                let a = direct.direct(i, j).count();
                let b = forced.perturbed::<f64>(i, j).count();
                let c = forced_ext.perturbed::<FExp>(i, j).count();
                assert_eq!(b, c, "({i}, {j})");
                differ += usize::from(a != b);
            }
        }
        // the orbits differ only for points on the boundary, where rounding decides
        assert!(differ <= 1, "{differ} pixels differ");
    }

    #[test]
    fn a_julia_set_by_perturbation_matches_direct_iteration() {
        let mut f = mandelbrot(1.0, 500);
        f.julia = Some(["-0.8".into(), "0.156".into()]);
        f.center = ["0".into(), "0".into()];
        let direct = Solver::new(&f).unwrap();
        assert_eq!(direct.method, Method::Direct);
        let mut deep = f.clone();
        deep.zoom = 30.0;
        let mut forced = Solver::new(&deep).unwrap();
        forced.plane = Plane::new(&f);
        let mut differ = 0;
        for j in (0..36).step_by(3) {
            for i in (0..64).step_by(3) {
                differ += usize::from(direct.direct(i, j).count() != forced.perturbed::<f64>(i, j).count());
            }
        }
        assert!(differ <= 2, "{differ} pixels differ");
    }

    #[test]
    fn bands_and_smooth_colours() {
        let c = Colouring {
            smooth: false,
            palette: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]],
            scale: 0.05,
            offset: 0.0,
            inside: [0.0, 0.0, 0.0, 1.0],
            bailout: 2.0,
        };
        assert_eq!(c.colour(Escape::Out { n: 223, abs: 3.0 }), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(c.colour(Escape::Out { n: 1308, abs: 3.0 }), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(c.colour(Escape::Inside), [0.0, 0.0, 0.0, 1.0]);
        // smooth: |z_n| = bailout² gives ν = n, so u = n · scale
        let s = Colouring { smooth: true, scale: 0.25, ..c };
        let col = s.colour(Escape::Out { n: 2, abs: 4.0 });
        // u = 0.5: halfway from red to blue
        assert_eq!(col, [0.5, 0.0, 0.5, 1.0]);
    }
}
