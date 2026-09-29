// Effect, transition and finishing passes. Every pass draws a full-screen
// triangle into its output; inputs are premultiplied working-space RGBA.
// fx.i.w is the working storage transfer id + 1 (0: linear storage).

struct Fx {
    v: array<vec4<f32>, 8>,
    i: vec4<u32>,
};

@group(0) @binding(0) var<uniform> fx: Fx;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var aux: texture_2d<f32>;
@group(0) @binding(3) var smp: sampler;
@group(0) @binding(4) var lut3: texture_3d<f32>;
@group(0) @binding(5) var aux2: texture_2d<f32>;
@group(0) @binding(6) var flowt: texture_2d<f32>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VOut {
    var o: VOut;
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    o.pos = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2(p.x, 1.0 - p.y);
    return o;
}

const PI: f32 = 3.14159265;

fn S(uv: vec2<f32>) -> vec4<f32> { return textureSampleLevel(src, smp, uv, 0.0); }
fn A(uv: vec2<f32>) -> vec4<f32> { return textureSampleLevel(aux, smp, uv, 0.0); }
fn A2(uv: vec2<f32>) -> vec4<f32> { return textureSampleLevel(aux2, smp, uv, 0.0); }
fn dims() -> vec2<f32> { return vec2<f32>(textureDimensions(src)); }
fn in_unit(uv: vec2<f32>) -> bool { return all(uv >= vec2(0.0)) && all(uv <= vec2(1.0)); }
fn Sz(uv: vec2<f32>) -> vec4<f32> { return select(vec4(0.0), S(uv), in_unit(uv)); }
// Bilinear sample of src with transparent texels beyond its edges.
fn St(uv: vec2<f32>) -> vec4<f32> {
    let d = vec2<i32>(textureDimensions(src));
    let q = uv * vec2<f32>(d) - 0.5;
    let i = vec2<i32>(floor(q));
    let f = q - floor(q);
    var acc = vec4(0.0);
    for (var k = 0; k < 4; k++) {
        let o = vec2(k & 1, k >> 1u);
        let p = i + o;
        if (all(p >= vec2(0)) && all(p < d)) {
            let w = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
            acc += textureLoad(src, p, 0) * w;
        }
    }
    return acc;
}

fn unpre(c: vec4<f32>) -> vec3<f32> { return select(vec3(0.0), c.rgb / c.a, c.a > 1e-6); }
fn lin(c: vec3<f32>) -> vec3<f32> {
    if (fx.i.w == 0u) { return c; }
    return tf_decode3(fx.i.w - 1u, c);
}
fn stored(c: vec3<f32>) -> vec3<f32> {
    if (fx.i.w == 0u) { return c; }
    return tf_encode3(fx.i.w - 1u, c);
}
fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }
fn enc(c: vec3<f32>) -> vec3<f32> { return tf_encode3(1u, max(c, vec3(0.0))); }
fn dec(c: vec3<f32>) -> vec3<f32> { return tf_decode3(1u, c); }

// The effect's D24 seed (a u64 in i.yz, or for transitions in the bits of v3.xy).
fn fx_seed() -> vec2<u32> { return vec2(fx.i.y, fx.i.z); }
fn tr_seed() -> vec2<u32> { return vec2(bitcast<u32>(fx.v[3].x), bitcast<u32>(fx.v[3].y)); }

// A u64 seed plus a signed offset.
fn seed_plus(seed: vec2<u32>, k: i32) -> vec2<u32> { return u64_add(seed, u64_of_i32(k)); }

// Smooth lattice noise in [0, 1): U(seed, 0, pack(i, j)) at the lattice points, smoothstep
// between (the Python renderer's effects.value_noise, CONVENTIONS 5.19).
fn d24_value(p: vec2<f32>, seed: vec2<u32>) -> f32 {
    let i = floor(p);
    var f = p - i;
    f = f * f * (3.0 - 2.0 * f);
    let ix = i32(i.x); let iy = i32(i.y);
    let l = vec4(d24_unit(seed, vec2(0u), d24_pack(ix, iy, 0)), d24_unit(seed, vec2(0u), d24_pack(ix + 1, iy, 0)),
                 d24_unit(seed, vec2(0u), d24_pack(ix, iy + 1, 0)), d24_unit(seed, vec2(0u), d24_pack(ix + 1, iy + 1, 0)));
    return mix(mix(l.x, l.y, f.x), mix(l.z, l.w, f.x), f.y);
}

// The Python renderer's fields.fractal (basic): octave i at 2^i weighs roughness^i, on seed
// + 101 i + 7919 · phase, cross-faded (smoothstep) to the next evolution phase; normalised.
fn d24_fbm(p: vec2<f32>, seed: vec2<u32>, octaves: f32, evolution: f32) -> f32 {
    let oc = clamp(octaves, 1.0, 12.0);
    let phase = floor(evolution);
    var f = evolution - phase;
    f = f * f * (3.0 - 2.0 * f);
    var out = 0.0; var total = 0.0;
    for (var i = 0; i < i32(ceil(oc)); i++) {
        let w = pow(0.5, f32(i)) * min(1.0, oc - f32(i));
        let q = p * exp2(f32(i));
        let base = i * 101 + i32(phase) * 7919;
        let z = mix(d24_value(q, seed_plus(seed, base)), d24_value(q, seed_plus(seed, base + 7919)), f);
        out += z * w;
        total += w;
    }
    return out / max(total, 1e-7);
}

fn rgb2hsv(c: vec3<f32>) -> vec3<f32> {
    let mx = max(c.r, max(c.g, c.b));
    let mn = min(c.r, min(c.g, c.b));
    let d = mx - mn;
    var h = 0.0;
    if (d > 1e-6) {
        if (mx == c.r) { h = (c.g - c.b) / d; }
        else if (mx == c.g) { h = 2.0 + (c.b - c.r) / d; }
        else { h = 4.0 + (c.r - c.g) / d; }
        h = fract(h / 6.0 + 1.0);
    }
    return vec3(h, select(0.0, d / mx, mx > 1e-6), mx);
}
fn hsv2rgb(c: vec3<f32>) -> vec3<f32> {
    let k = vec3(1.0, 2.0 / 3.0, 1.0 / 3.0);
    let p = abs(fract(vec3(c.x) + k) * 6.0 - 3.0);
    return c.z * mix(vec3(1.0), clamp(p - 1.0, vec3(0.0), vec3(1.0)), c.y);
}

// ---------------------------------------------------------------- basics

@fragment
fn fs_copy(in: VOut) -> @location(0) vec4<f32> { return S(in.uv); }

// Dual-Kawase downsample (output half size); v0.x = offset in source texels.
// The blur pyramid reads the input bilinearly with transparent texels outside it (CONVENTIONS
// 5.7: samples outside the effect's input are transparent, never clamped to the edge).
fn Z(uv: vec2<f32>) -> vec4<f32> { return bil(src, uv * dims()); }

@fragment
fn fs_down(in: VOut) -> @location(0) vec4<f32> {
    let t = 1.0 / dims() * fx.v[0].x;
    var s = Z(in.uv) * 4.0;
    s += Z(in.uv + vec2(-t.x, -t.y)) + Z(in.uv + vec2(t.x, t.y));
    s += Z(in.uv + vec2(t.x, -t.y)) + Z(in.uv + vec2(-t.x, t.y));
    return s / 8.0;
}

// Dual-Kawase upsample; v0.x = offset in source texels.
@fragment
fn fs_up(in: VOut) -> @location(0) vec4<f32> {
    let t = 1.0 / dims() * fx.v[0].x;
    var s = Z(in.uv + vec2(-2.0 * t.x, 0.0)) + Z(in.uv + vec2(2.0 * t.x, 0.0));
    s += Z(in.uv + vec2(0.0, -2.0 * t.y)) + Z(in.uv + vec2(0.0, 2.0 * t.y));
    s += (Z(in.uv + vec2(-t.x, t.y)) + Z(in.uv + vec2(t.x, t.y)) + Z(in.uv + vec2(t.x, -t.y)) + Z(in.uv + vec2(-t.x, -t.y))) * 2.0;
    return s / 12.0;
}

// Prepass: i.x = 0 bright pass (v0.x threshold, v0.y knee, v1 tint rgb);
// 1 tinted alpha (shadow/glow source: v1 colour, v0.zw uv offset);
// 2 inverted alpha (inner shadow/glow source).
@fragment
fn fs_pre(in: VOut) -> @location(0) vec4<f32> {
    let c = S(in.uv);
    switch fx.i.x {
        case 0u: {
            let l = luma(lin(unpre(c)));
            let th = fx.v[0].x;
            let k = max(fx.v[0].y, 1e-4);
            let w = clamp((l - th + k) / (2.0 * k), 0.0, 1.0);
            let soft = w * w * k * 0.5 * 2.0;
            let f = max(soft, l - th) / max(l, 1e-4);
            return vec4(c.rgb * clamp(f, 0.0, 1e3) * fx.v[1].rgb, c.a * clamp(f, 0.0, 1.0));
        }
        case 1u: {
            let a = Sz(in.uv - fx.v[0].zw).a;
            return vec4(fx.v[1].rgb * fx.v[1].a, fx.v[1].a) * a;
        }
        default: {
            let a = 1.0 - Sz(in.uv - fx.v[0].zw).a;
            return vec4(fx.v[1].rgb * fx.v[1].a, fx.v[1].a) * a;
        }
    }
}

// Combine src (original) with aux (processed); v0.x amount; v1 tint.
// i.x: 0 mix, 1 add, 2 aux behind src, 3 src over aux... (on-top), 4 aux only,
// 5 inner (aux inside src alpha), 6 unsharp (src + (src - aux) * amount, v0.y threshold),
// 7 screen add clipped to src alpha (inner glow), 8 accumulate (aux * amount), 9 keep aux where src alpha (outline only)
@fragment
fn fs_combine(in: VOut) -> @location(0) vec4<f32> {
    let s = S(in.uv);
    let p = A(in.uv);
    let k = fx.v[0].x;
    switch fx.i.x {
        case 0u: { return mix(s, p, k); }
        case 1u: { return vec4(s.rgb + p.rgb * k * fx.v[1].rgb, max(s.a, min(1.0, s.a + p.a * k))); }
        case 2u: { let q = p * k; return s + q * (1.0 - s.a); }
        case 3u: { let q = p * k; return q + s * (1.0 - q.a); }
        case 4u: { return p * k; }
        case 5u: {
            let q = p * k * s.a;
            return vec4(q.rgb + s.rgb * (1.0 - q.a), s.a);
        }
        case 6u: {
            let d = s - p;
            let m = select(1.0, 0.0, luma(abs(d.rgb)) < fx.v[0].y);
            return max(s + d * k * m, vec4(0.0));
        }
        case 7u: { let q = p * k * s.a; return vec4(s.rgb + q.rgb * (1.0 - s.rgb * 0.0), s.a); }
        case 8u: { return p * k; }
        default: { return s; }
    }
}

// ------------------------------------------------------------ colour and generators

fn parse_curve(ch: f32, x: f32) -> f32 {
    // curves live in aux as 256×1 RGBA (per channel), sampled at x
    return textureSampleLevel(aux, smp, vec2(clamp(x, 0.0, 1.0) * (255.0 / 256.0) + 0.5 / 256.0, 0.5), 0.0)[u32(ch)];
}

fn tonemap(c: vec3<f32>, op: u32) -> vec3<f32> {
    switch op {
        case 0u: { // ACES (Narkowicz fit)
            return clamp((c * (2.51 * c + 0.03)) / (c * (2.43 * c + 0.59) + 0.14), vec3(0.0), vec3(1.0));
        }
        case 1u: { // AgX (analytic approximation of the default look)
            let m = mat3x3<f32>(vec3(0.842479, 0.0784336, 0.0792237), vec3(0.0423303, 0.878468, 0.0791661), vec3(0.0423757, 0.0784336, 0.879142));
            var x = m * max(c, vec3(1e-10));
            x = clamp((log2(x) + 12.47393) / 16.5, vec3(0.0), vec3(1.0));
            let x2 = x * x;
            let x4 = x2 * x2;
            let y = 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
            let mi = mat3x3<f32>(vec3(1.19687900512017, -0.0980208811401368, -0.0990297440797205), vec3(-0.0528968517574562, 1.15190312990417, -0.0989611768448433), vec3(-0.0529716355144438, -0.0980434501171241, 1.15107367264116));
            return max(dec(clamp(mi * y, vec3(0.0), vec3(1.0))), vec3(0.0));
        }
        case 2u: { // filmic (Hejl-Burgess-Dawson, returns display-encoded)
            let x = max(vec3(0.0), c - 0.004);
            return dec((x * (6.2 * x + 0.5)) / (x * (6.2 * x + 1.7) + 0.06));
        }
        case 3u: { return c / (1.0 + c); }
        case 4u: { // Hable / Uncharted 2
            let W = 11.2;
            let a = 0.15; let b = 0.50; let cc = 0.10; let d = 0.20; let e = 0.02; let f = 0.30;
            let x = c * 2.0;
            let num = ((x * (a * x + cc * b) + d * e) / (x * (a * x + b) + d * f)) - e / f;
            let wn = ((W * (a * W + cc * b) + d * e) / (W * (a * W + b) + d * f)) - e / f;
            return num / wn;
        }
        default: { // PQ-range (10,000 nits = 100) to SDR: BT.2390-style knee at 1.0
            let l = max(luma(c), 1e-6);
            let ks = 0.75;
            var o = l;
            if (l > ks) {
                let t = (l - ks) / (1.0 - ks);
                o = ks + (1.0 - ks) * (1.0 - exp(-t));
            }
            return c * (o / l);
        }
    }
}

// Bilinear sampling at pixel coordinates (texel i at i), transparent outside: the Python
// renderer's effects.sample.
fn sample_px(p: vec2<f32>) -> vec4<f32> {
    let d = vec2<i32>(dims());
    let f0 = floor(p);
    let f = p - f0;
    let i0 = vec2<i32>(f0);
    var acc = vec4(0.0);
    for (var j = 0; j < 2; j++) {
        for (var i = 0; i < 2; i++) {
            let q = i0 + vec2(i, j);
            let w = select(1.0 - f.x, f.x, i == 1) * select(1.0 - f.y, f.y, j == 1);
            if (all(q >= vec2(0)) && all(q < d) && w > 0.0) { acc += textureLoad(src, q, 0) * w; }
        }
    }
    return acc;
}

// Film grain's three D24 normals at pixel px: draws (y · width + x) · 3 + c of the effect's seed
// (i.yz) on the frame's channel (v0.z).
fn grain_draws(px: vec2<f32>) -> vec3<f32> {
    let xy = vec2<u32>(floor(px));
    let base = (xy.y * u32(dims().x) + xy.x) * 3u;
    let seed = vec2(fx.i.y, fx.i.z);
    let ch = vec2(u32(fx.v[0].z), 0u);
    return vec3(d24_gaussian(seed, ch, vec2(base, 0u)), d24_gaussian(seed, ch, vec2(base + 1u, 0u)),
                d24_gaussian(seed, ch, vec2(base + 2u, 0u)));
}

fn grade(c3: vec3<f32>, uv: vec2<f32>, px: vec2<f32>) -> vec3<f32> {
    var c = c3;
    let v = fx.v;
    switch fx.i.x {
        case 1u: { // color-grade: v0 saturation, contrast, brightness
            var e = enc(c);
            e = (e - 0.5) * v[0].y + 0.5 + v[0].z;
            c = dec(e);
            let l = luma(c);
            c = mix(vec3(l), c, v[0].x);
        }
        case 2u: { // lift gamma gain (v0 lift, v1 gamma, v2 gain; on encoded values)
            var e = enc(c);
            e = e * v[2].rgb + v[0].rgb * (1.0 - e);
            e = pow(max(e, vec3(0.0)), 1.0 / max(v[1].rgb, vec3(1e-3)));
            c = dec(e);
        }
        case 3u: { // ASC CDL on encoded values: v0 slope, v1 offset, v2 power, v3.x saturation
            var e = enc(c);
            e = pow(max(e * v[0].rgb + v[1].rgb, vec3(0.0)), v[2].rgb);
            let l = dot(e, vec3(0.2126, 0.7152, 0.0722));
            e = l + v[3].x * (e - l);
            c = dec(e);
        }
        case 4u: { // curves (aux 256×1); v0.x channel: 0 rgb (master in alpha), 1 r, 2 g, 3 b, 5 luma
            var e = enc(c);
            let ch = u32(v[0].x);
            if (ch == 0u) { e = vec3(parse_curve(3.0, e.r), parse_curve(3.0, e.g), parse_curve(3.0, e.b)); }
            else if (ch <= 3u) { e[ch - 1u] = parse_curve(f32(ch - 1u), e[ch - 1u]); }
            else if (ch == 5u) { let l = luma(e); let n = parse_curve(3.0, l); e = e + (n - l); }
            c = dec(e);
        }
        case 5u: { // levels: v0 inBlack, inWhite, gamma, _; v1 outBlack, outWhite, channel
            var e = enc(c);
            var r = clamp((e - v[0].x) / max(v[0].y - v[0].x, 1e-4), vec3(0.0), vec3(1.0));
            r = pow(r, vec3(1.0 / max(v[0].z, 1e-3)));
            r = mix(vec3(v[1].x), vec3(v[1].y), r);
            let ch = u32(v[1].z);
            if (ch == 0u || ch >= 4u) { e = r; } else { e[ch - 1u] = r[ch - 1u]; }
            c = dec(e);
        }
        case 6u: { // white balance: v0.x temperature, v0.y tint (−100…100), linear gains
            let t = v[0].x / 100.0;
            let g = v[0].y / 100.0;
            c = c * vec3(1.0 + 0.3 * t, 1.0 - 0.3 * g, 1.0 - 0.3 * t);
        }
        case 7u: { c = c * exp2(v[0].x) + v[0].y; } // exposure (stops), offset
        case 8u: { // hue/saturation/lightness: v0 hue deg, saturation, brightness
            var h = rgb2hsv(enc(c));
            h.x = fract(h.x + v[0].x / 360.0 + 1.0);
            h.y = clamp(h.y * v[0].y, 0.0, 1.0);
            h.z = h.z + v[0].z;
            c = dec(hsv2rgb(h));
        }
        case 9u: { c = tonemap(c * exp2(v[0].x), u32(v[0].y)); } // tonemap
        case 10u: { // tint: v1 colour, v0.x amount
            let l = luma(c);
            c = mix(c, l * v[1].rgb, v[0].x);
        }
        case 11u: { // tritone: shadows v1, midtones v2, highlights v3; v0.x amount
            let l = enc(vec3(luma(c))).x;
            var t = select(mix(v[2].rgb, v[3].rgb, (l - 0.5) * 2.0), mix(v[1].rgb, v[2].rgb, l * 2.0), l < 0.5);
            c = mix(c, t, v[0].x);
        }
        case 12u: { // gradient map from aux (256×1), v0.x amount
            let l = clamp(enc(vec3(luma(c))).x, 0.0, 1.0);
            let g = textureSampleLevel(aux, smp, vec2(l * (255.0 / 256.0) + 0.5 / 256.0, 0.5), 0.0);
            c = mix(c, unpre(g), v[0].x);
        }
        case 13u: { c = mix(c, vec3(luma(c)), v[0].x); } // grayscale
        case 14u: { // sepia
            let e = enc(c);
            let s = vec3(dot(e, vec3(0.393, 0.769, 0.189)), dot(e, vec3(0.349, 0.686, 0.168)), dot(e, vec3(0.272, 0.534, 0.131)));
            c = mix(c, dec(min(s, vec3(1.0))), v[0].x);
        }
        case 15u: { // invert (encoded); v0.y channel
            var e = enc(c);
            let ch = u32(v[0].y);
            var n = 1.0 - e;
            if (ch == 5u) { let l = luma(e); n = e + (1.0 - 2.0 * l); }
            else if (ch >= 1u && ch <= 3u) { n = e; n[ch - 1u] = 1.0 - e[ch - 1u]; }
            c = dec(mix(e, n, v[0].x));
        }
        case 16u: { // posterize (levels)
            let n = max(v[0].x - 1.0, 1.0);
            c = dec(floor(enc(c) * n + 0.5) / n);
        }
        case 17u: { c = vec3(select(0.0, 1.0, enc(vec3(luma(c))).x >= v[0].x)); } // threshold
        case 18u: { c = mix(c, v[1].rgb, v[0].x); } // colour overlay / fill (colour in v1)
        case 19u: { // gradient overlay: aux 256×1 along angle v0.y, amount v0.x
            let a = radians(v[0].y);
            let d = vec2(cos(a), sin(a));
            let t = clamp(dot(uv - 0.5, d) / (abs(d.x) + abs(d.y)) + 0.5, 0.0, 1.0);
            let g = textureSampleLevel(aux, smp, vec2(t * (255.0 / 256.0) + 0.5 / 256.0, 0.5), 0.0);
            c = mix(c, unpre(g), v[0].x * g.a);
        }
        case 20u: { // selective colour: hue centre v0.x deg, tolerance v0.y, saturation v0.z, brightness v0.w
            var h = rgb2hsv(enc(c));
            let d = abs(fract(h.x - v[0].x / 360.0 + 0.5) - 0.5);
            let w = 1.0 - smoothstep(v[0].y * 0.5, v[0].y * 0.5 + 0.05, d);
            h.y = clamp(h.y * mix(1.0, v[0].z, w), 0.0, 1.0);
            h.z = h.z + v[0].w * w;
            c = dec(hsv2rgb(h));
        }
        case 21u: { // film grain: v0 0.05·amount, _, frame, field scale (0: draw here); v1 strength, response
            // D24 normals per sample, (y · width + x) · 3 + channel, seed in i.yz, the frame as the channel
            var n: vec3<f32>;
            if (v[0].w == 0.0) {
                n = grain_draws(px);
            } else {
                n = A(uv).rgb * v[0].w;
            }
            let k = clamp(c, vec3(0.0), vec3(1.0));
            let wgt = pow(max(4.0 * k * (1.0 - k), vec3(0.0)), vec3(v[1].w));
            c = max(c + n * v[0].x * wgt * v[1].rgb, vec3(0.0));
        }
        case 22u: { // noise: v0 amount, frame, channels (1 or 3); D24 uniforms in [-1, 1) per sample in display values
            let xy = vec2<u32>(floor(px));
            let nc = u32(v[0].z);
            let base = (xy.y * u32(dims().x) + xy.x) * nc;
            let ch = vec2(u32(v[0].y), 0u);
            var n = vec3(-1.0 + 2.0 * d24_unit(fx_seed(), ch, vec2(base, 0u)));
            if (nc == 3u) {
                n.y = -1.0 + 2.0 * d24_unit(fx_seed(), ch, vec2(base + 1u, 0u));
                n.z = -1.0 + 2.0 * d24_unit(fx_seed(), ch, vec2(base + 2u, 0u));
            }
            c = dec(max(enc(c) + n * v[0].x, vec3(0.0)));
        }
        case 23u: { // spill suppress: v1 key colour, v0.x amount
            let k = v[1].rgb;
            let ch = select(select(2u, 1u, k.g >= k.b), 0u, k.r >= max(k.g, k.b));
            var e = c;
            let o1 = (ch + 1u) % 3u;
            let o2 = (ch + 2u) % 3u;
            let lim = (e[o1] + e[o2]) * 0.5;
            e[ch] = mix(e[ch], min(e[ch], lim), v[0].x);
            c = e;
        }
        case 24u: { // LUT: v4..v6 working→LUT matrix rows, v7.x transfer id, v7.z output transfer id + 1 (0: v7.x), v0.x mix, v0.y size
            let m = mat3x3<f32>(v[4].xyz, v[5].xyz, v[6].xyz);
            var x = transpose(m) * c;
            x = tf_encode3(u32(v[7].x), x);
            let n = v[0].y;
            let coord = clamp(x, vec3(0.0), vec3(1.0)) * ((n - 1.0) / n) + 0.5 / n;
            var y = textureSampleLevel(lut3, smp, coord, 0.0).rgb;
            let tf_out = select(u32(v[7].x), u32(v[7].z) - 1u, v[7].z > 0.5);
            y = tf_decode3(tf_out, y);
            let mi = mat3x3<f32>(vec3(v[4].w, v[5].w, v[6].w), vec3(v[1].x, v[1].y, v[1].z), vec3(v[2].x, v[2].y, v[2].z));
            c = mix(c, transpose(mi) * y, v[0].x);
        }
        default: {}
    }
    return c;
}

// Per-pixel family. Colour ops (i.x < 64) run on straight linear colour;
// i.x ≥ 64 are generators and keyers.
@fragment
fn fs_color(in: VOut) -> @location(0) vec4<f32> {
    let s = S(in.uv);
    let d = dims();
    let px = in.uv * d;
    let v = fx.v;
    if (fx.i.x < 64u) {
        if (s.a <= 0.0) { return s; }
        let c = grade(lin(unpre(s)), in.uv, px);
        return vec4(stored(c) * s.a, s.a);
    }
    switch fx.i.x {
        case 75u: { return vec4(grain_draws(px), 1.0); } // film grain's field, before its blur
        case 76u: { // glitch, rows and blocks: v0 amount px, row height, block weight, blocks; v1 frame, rows
            let seed = vec2(fx.i.y, fx.i.z);
            let ch = vec2(u32(v[1].x), 0u);
            let xy = vec2<i32>(floor(px));
            let row = u32(floor(f32(xy.y) / v[0].y));
            let rows = u32(v[1].y);
            let keep = d24_unit(seed, ch, vec2(rows + row, 0u)) < 0.25;
            let shift = select(0.0, -1.0 + 2.0 * d24_unit(seed, ch, vec2(row, 0u)), keep) * v[0].x;
            var o = sample_px(vec2(f32(xy.x) + shift, f32(xy.y)));
            for (var k = 0; k < i32(v[0].w); k++) {
                let t0 = textureLoad(aux, vec2(3 * k, 0), 0);
                let t1 = textureLoad(aux, vec2(3 * k + 1, 0), 0);
                let t2 = textureLoad(aux, vec2(3 * k + 2, 0), 0);
                let bx = i32(t0.x * 256.0 + t0.y); let by = i32(t0.z * 256.0 + t0.w);
                let bw = i32(t1.x * 256.0 + t1.y); let bh = i32(t1.z * 256.0 + t1.w);
                let sx = i32(t2.x * 256.0 + t2.y); let sy = i32(t2.z * 256.0 + t2.w);
                if (xy.x >= bx && xy.x < bx + bw && xy.y >= by && xy.y < by + bh) {
                    let src_px = textureLoad(src, vec2(sx + xy.x - bx, sy + xy.y - by), 0);
                    o = mix(o, src_px, v[0].z);
                }
            }
            return o;
        }
        case 77u: { // glitch, channel delay and quantisation: v0 delay px, levels
            let xy = floor(px);
            let r = sample_px(vec2(xy.x - v[0].x, xy.y));
            let b = sample_px(vec2(xy.x + v[0].x, xy.y));
            let g = textureLoad(src, vec2<i32>(xy), 0);
            let a = max(max(r.a, g.a), b.a);
            let st = select(vec3(0.0), vec3(r.r, g.g, b.b) / a, a > 1e-6);
            let l = v[0].y - 1.0;
            return vec4(round(st * l) / l * a, a);
        }
        case 64u: { // vignette (D9): v0 amount, r₀, softness (fractions of the half diagonal); v1 colour; v2.xy centre (uv)
            let r = length((in.uv - v[2].xy) * d) / (0.5 * length(d));
            var k = select(0.0, 1.0, r >= v[0].y);
            if (v[0].z > 0.0) { k = smoothstep(v[0].y, v[0].y + v[0].z, r); }
            let w = k * v[0].x;
            return vec4(mix(s.rgb, v[1].rgb * s.a, w), s.a);
        }
        case 65u: { // letterbox: v0.x target aspect, v1 colour
            let a = d.x / d.y;
            var inside = true;
            if (v[0].x > a) { let h = a / v[0].x; inside = abs(in.uv.y - 0.5) <= h * 0.5; }
            else { let w = v[0].x / a; inside = abs(in.uv.x - 0.5) <= w * 0.5; }
            return select(vec4(v[1].rgb, 1.0) * v[1].a, s, inside);
        }
        case 66u: { // scanlines: v0 amount, period px, thickness px
            let y = fract(px.y / max(v[0].y, 1.0));
            let m = select(1.0, 1.0 - v[0].x, y < v[0].z / max(v[0].y, 1.0));
            return vec4(s.rgb * m, s.a);
        }
        case 67u: { // light leak: v0 intensity, time*speed, seed; v1 colour
            let t = v[0].y;
            let n = d24_fbm(in.uv * 1.5 + vec2(t * 0.15, t * 0.05), fx_seed(), 5.0, 0.0);
            let edge = pow(1.0 - clamp(length(in.uv - vec2(0.9, 0.1)) * 1.1, 0.0, 1.0), 2.0);
            let w = clamp(n * edge * 2.0, 0.0, 1.0) * v[0].x;
            return s + vec4(v[1].rgb * w, 0.0) * max(s.a, 0.0);
        }
        case 68u: { // light sweep: v0 intensity, band position (uv along angle), width (uv), angle deg; v1 colour
            let a = radians(v[0].w);
            let dir = vec2(cos(a), sin(a));
            let t = dot(in.uv, dir);
            let w = exp(-pow((t - v[0].y) / max(v[0].z, 1e-3), 2.0)) * v[0].x;
            return vec4(s.rgb + v[1].rgb * w * s.a, s.a);
        }
        case 69u: { // lens flare: v0 intensity, size, _; v1 colour; v2.xy source (uv)
            let asp = vec2(d.x / d.y, 1.0);
            let p = (in.uv - v[2].xy) * asp;
            var l = 0.02 * v[0].y / (length(p) + 0.02 * v[0].y);
            let ax = (vec2(0.5) - v[2].xy);
            for (var k = 1; k <= 4; k++) {
                let gp = v[2].xy + ax * (0.6 * f32(k));
                let r = 0.02 * f32(k) * v[0].y;
                l += 0.25 * smoothstep(r, r * 0.6, length((in.uv - gp) * asp)) / f32(k);
            }
            let streak = exp(-abs(p.y) * 200.0 / max(v[0].y, 0.1)) * exp(-abs(p.x) * 3.0) * 0.5;
            let add = v[1].rgb * (l + streak) * v[0].x;
            return vec4(s.rgb + add, max(s.a, min(1.0, luma(add))));
        }
        case 70u: { // fractal noise: v0 scale px, frequency, time*speed, seed; v1 intensity, mix
            let n = d24_fbm(px / max(v[0].x, 1.0) * v[0].y, fx_seed(), 6.0, v[0].z) * v[1].x;
            let g = vec4(vec3(n), 1.0);
            return mix(s, g, v[1].y);
        }
        case 71u: { // chroma key: v1 key colour; v0 tolerance, softness, spill
            let c = enc(lin(unpre(s)));
            let k = enc(v[1].rgb);
            // distance in a YCbCr-like chroma plane
            let cc = vec2(dot(c, vec3(-0.1146, -0.3854, 0.5)), dot(c, vec3(0.5, -0.4542, -0.0458)));
            let kc = vec2(dot(k, vec3(-0.1146, -0.3854, 0.5)), dot(k, vec3(0.5, -0.4542, -0.0458)));
            let dd = length(cc - kc) / 0.7;
            let a = smoothstep(v[0].x, v[0].x + max(v[0].y, 1e-4), dd);
            var o = unpre(s);
            let ch = select(select(2u, 1u, v[1].g >= v[1].b), 0u, v[1].r >= max(v[1].g, v[1].b));
            let lim = (o[(ch + 1u) % 3u] + o[(ch + 2u) % 3u]) * 0.5;
            o[ch] = mix(o[ch], min(o[ch], lim), v[0].z);
            let na = s.a * a;
            return vec4(o * na, na);
        }
        case 72u: { // luma key: v0 threshold, softness, invert
            let l = enc(vec3(luma(lin(unpre(s))))).x;
            var a = smoothstep(v[0].x - v[0].y * 0.5, v[0].x + v[0].y * 0.5 + 1e-4, l);
            if (v[0].z > 0.5) { a = 1.0 - a; }
            return s * a;
        }
        case 73u: { // difference key against aux (the clean plate): v0 tolerance, softness
            let p = A(in.uv);
            let dd = length(enc(unpre(s)) - enc(unpre(p)));
            let a = smoothstep(v[0].x, v[0].x + max(v[0].y, 1e-4), dd);
            return s * a;
        }
        case 74u: { // halftone: v0 cell px, angle deg, amount
            let a = radians(v[0].y);
            let r = mat2x2<f32>(vec2(cos(a), sin(a)), vec2(-sin(a), cos(a)));
            let q = r * px / max(v[0].x, 2.0);
            let cell = floor(q) + 0.5;
            let cpx = transpose(r) * (cell * max(v[0].x, 2.0));
            let sc = S(cpx / d);
            let l = enc(vec3(luma(unpre(sc)))).x;
            let rad = sqrt(1.0 - l) * 0.7071;
            let inside = 1.0 - smoothstep(rad - 0.05, rad + 0.05, length(q - cell));
            let o = vec4(vec3(0.0), sc.a) * inside + vec4(vec3(1.0), 1.0) * (1.0 - inside) * sc.a;
            return mix(s, o, v[0].z);
        }
        default: { return s; }
    }
}

// ------------------------------------------------------------ warps

// i.x op; v2.xy centre (uv); v0 params; v1 params; aux = displacement map; v7.x time
@fragment
fn fs_warp(in: VOut) -> @location(0) vec4<f32> {
    let d = dims();
    let v = fx.v;
    var uv = in.uv;
    let c = v[2].xy;
    let asp = vec2(d.x / d.y, 1.0);
    let t = v[7].x;
    switch fx.i.x {
        case 0u: { // displacement map: v0.x amount px (horizontal from red, vertical from green)
            let m = A(in.uv);
            let o = (unpre(m).rg - 0.5) * 2.0 * v[0].x * m.a;
            uv = uv - o / d;
        }
        case 1u: { // turbulent displace: v0 amount px, size px, time, seed
            let q = in.uv * d / max(v[0].y, 1.0);
            let o = vec2(d24_fbm(q, fx_seed(), 6.0, v[0].z), d24_fbm(q + vec2(37.0, 17.0), seed_plus(fx_seed(), 7), 6.0, v[0].z)) - 0.5;
            uv = uv + o * 2.0 * v[0].x / d;
        }
        case 2u: { // wave warp: v0 amplitude px, wavelength px, phase (rad), angle deg
            let a = radians(v[0].w);
            let dir = vec2(cos(a), sin(a));
            let nrm = vec2(-dir.y, dir.x);
            let p = in.uv * d;
            let o = sin(dot(p, dir) / max(v[0].y, 1.0) * 2.0 * PI + v[0].z) * v[0].x;
            uv = uv + nrm * o / d;
        }
        case 3u: { // ripple: v0 amplitude px, wavelength px, phase, radius px
            let p = (in.uv - c) * d;
            let r = length(p);
            let fall = select(1.0, clamp(1.0 - r / v[0].w, 0.0, 1.0), v[0].w > 0.0);
            let o = sin(r / max(v[0].y, 1.0) * 2.0 * PI - v[0].z) * v[0].x * fall;
            uv = uv + select(vec2(0.0), p / r, r > 1e-3) * o / d;
        }
        case 4u: { // twirl: v0 angle deg, radius px
            let p = (in.uv - c) * d;
            let r = length(p);
            let w = clamp(1.0 - r / max(v[0].y, 1.0), 0.0, 1.0);
            let a = radians(v[0].x) * w * w;
            let rp = vec2(p.x * cos(a) - p.y * sin(a), p.x * sin(a) + p.y * cos(a));
            uv = c + rp / d;
        }
        case 5u, 6u: { // spherize / bulge: v0 amount, radius px
            let p = (in.uv - c) * d;
            let r = length(p) / max(v[0].y, 1.0);
            if (r < 1.0) {
                var k = 0.0;
                if (fx.i.x == 5u) { k = mix(r, asin(r) * 2.0 / PI, v[0].x) / max(r, 1e-4); }
                else { k = pow(r, v[0].x) / max(r, 1e-4); k = select(k, 1.0, r < 1e-4); }
                uv = c + p * k / d;
            }
        }
        case 7u: { // lens distortion: v0.x k1 (barrel < 0 < pincushion)
            let p = (in.uv - 0.5) * asp;
            let r2 = dot(p, p);
            uv = 0.5 + p * (1.0 + v[0].x * r2) / asp;
        }
        case 8u: { // heat haze: v0 amount px, scale px, time
            let q = in.uv * d / max(v[0].y, 1.0);
            let q2 = q - vec2(0.0, v[0].z);  // rising cells (the Python renderer's heat haze)
            let o = vec2(d24_fbm(q2, fx_seed(), 6.0, v[0].z), d24_fbm(q2 + vec2(37.0, 17.0), seed_plus(fx_seed(), 7), 6.0, v[0].z)) - 0.5;
            uv = uv + o * v[0].x / d;
        }
        case 9u: { // mirror across a line through c at angle v0.x
            let a = radians(v[0].x);
            let n = vec2(cos(a), sin(a));
            let p = (in.uv - c) * d;
            let s = dot(p, n);
            if (s < 0.0) { uv = c + (p - 2.0 * s * n) / d; }
        }
        case 10u: { // kaleidoscope: v0 segments, angle deg
            let p = (in.uv - c) * d;
            let seg = 2.0 * PI / max(v[0].x, 1.0);
            var a = atan2(p.y, p.x) - radians(v[0].y);
            a = abs(a - seg * floor(a / seg + 0.5));
            let r = length(p);
            uv = c + vec2(cos(a + radians(v[0].y)), sin(a + radians(v[0].y))) * r / d;
        }
        case 11u: { // tile: v0 repeats, mirror edges
            let q = in.uv * max(v[0].x, 1.0);
            var f = fract(q);
            if (v[0].y > 0.5) { let cell = floor(q); f = select(f, 1.0 - f, (vec2<i32>(cell) & vec2(1)) == vec2(1)); }
            uv = f;
        }
        case 12u: { // pixelate: v0.x cell px
            let s = max(v[0].x, 1.0);
            uv = (floor(in.uv * d / s) + 0.5) * s / d;
        }
        case 13u: { // mosaic: average of 4×4 samples in each cell
            let s = max(v[0].x, 1.0);
            let base = floor(in.uv * d / s) * s;
            var acc = vec4(0.0);
            for (var y = 0; y < 4; y++) {
                for (var x = 0; x < 4; x++) {
                    acc += S((base + (vec2(f32(x), f32(y)) + 0.5) * s / 4.0) / d);
                }
            }
            return acc / 16.0;
        }
        case 14u: { // chromatic aberration: v0.x px of shift at the farthest corner (CONVENTIONS 5.18)
            // red magnifies by 1 − l and blue by 1 + l about the centre v1.xy (px), l = amount / reach
            // (v1.z px, the distance to the input's farthest corner); outside the input is transparent
            let pc = v[1].xy;
            let x = in.uv * d;
            let l = v[0].x / max(v[1].z, 1.0);
            let r = St((pc + (x - pc) / max(1.0 - l, 0.01)) / d);
            let g = S(in.uv);
            let b = St((pc + (x - pc) / max(1.0 + l, 0.01)) / d);
            return vec4(r.r, g.g, b.b, max(g.a, max(r.a, b.a)));
        }
        case 15u: { // rgb split (v0.xy px)
            let o = v[0].xy / d;
            let r = Sz(in.uv + o);
            let g = S(in.uv);
            let b = Sz(in.uv - o);
            return vec4(r.r, g.g, b.b, max(g.a, max(r.a, b.a)));
        }
        case 17u: { // vhs: v0 amount, time, seed
            let row = in.uv.y * d.y;
            let ch = vec2(u32(v[0].z), 0u);
            let j = d24_gaussian(fx_seed(), ch, vec2(u32(row), 0u)) * 0.4 * 6.0 * v[0].x / d.x;
            let bleed = 3.0 * v[0].x / d.x;
            let g = Sz(in.uv + vec2(j, 0.0));
            let r = Sz(in.uv + vec2(j + bleed, 0.0));
            let b = Sz(in.uv + vec2(j - bleed, 0.0));
            let xy = vec2<u32>(floor(in.uv * d));
            let n = d24_gaussian(fx_seed(), ch, vec2(u32(d.y) + xy.y * u32(d.x) + xy.x, 0u)) * 0.018 * v[0].x;
            let line = 1.0 - 0.15 * v[0].x * step(0.5, fract(row * 0.5));
            return vec4((vec3(r.r, g.g, b.b) + n * g.a) * line, max(g.a, max(r.a, b.a)));
        }
        default: {}
    }
    return Sz(uv);
}

// ------------------------------------------------------------ line sampling

// i.x: 0 directional (v0.xy step px, v0.z samples), 1 radial spin (v0.x angle rad, v0.z samples),
// 2 zoom (v0.x amount, v0.z samples), 3 god rays (v0.x length fraction, v0.z samples, v0.w decay; aux = bright pass),
// 4 long shadow (v0.xy step px, v0.z samples, v1 colour); v2.xy centre uv
@fragment
fn fs_line(in: VOut) -> @location(0) vec4<f32> {
    let d = dims();
    let v = fx.v;
    let n = max(i32(v[0].z), 1);
    let c = v[2].xy;
    var acc = vec4(0.0);
    var wsum = 0.0;
    switch fx.i.x {
        case 0u: {
            for (var k = 0; k < n; k++) {
                let t = (f32(k) + 0.5) / f32(n) - 0.5;
                acc += Sz(in.uv + v[0].xy * t / d);
            }
            return acc / f32(n);
        }
        case 1u: {
            let p = (in.uv - c) * d;
            for (var k = 0; k < n; k++) {
                let a = v[0].x * ((f32(k) + 0.5) / f32(n) - 0.5);
                acc += Sz(c + vec2(p.x * cos(a) - p.y * sin(a), p.x * sin(a) + p.y * cos(a)) / d);
            }
            return acc / f32(n);
        }
        case 2u: {
            for (var k = 0; k < n; k++) {
                let s = 1.0 - v[0].x * (f32(k) + 0.5) / f32(n);
                acc += Sz(c + (in.uv - c) * s);
            }
            return acc / f32(n);
        }
        case 3u: {
            var w = 1.0;
            for (var k = 0; k < n; k++) {
                let s = 1.0 - v[0].x * f32(k) / f32(n);
                acc += textureSampleLevel(aux, smp, c + (in.uv - c) * s, 0.0) * w;
                wsum += w;
                w *= v[0].w;
            }
            let rays = acc / max(wsum, 1e-4) * v[1].x;
            let s0 = S(in.uv);
            return vec4(s0.rgb + rays.rgb, max(s0.a, min(1.0, s0.a + rays.a)));
        }
        default: {
            var a = 0.0;
            for (var k = 1; k <= n; k++) {
                a = max(a, Sz(in.uv - v[0].xy * f32(k) / d).a * (1.0 - f32(k) / f32(n + 1) * v[1].w));
            }
            let s0 = S(in.uv);
            let sh = vec4(v[1].rgb, 1.0) * a;
            return s0 + sh * (1.0 - s0.a);
        }
    }
}

// ------------------------------------------------------------ bokeh

// Gather over a polygonal aperture. v0: radius px, blades (0 = round), rotation rad, rings;
// v1: highlight threshold, gain; i.x = 1 tilt-shift: radius scales with distance from the
// focus line (v2.x centre y uv, v2.y half band uv, v2.z angle rad).
@fragment
fn fs_bokeh(in: VOut) -> @location(0) vec4<f32> {
    let d = dims();
    let v = fx.v;
    var r = v[0].x;
    if (fx.i.x == 1u) {
        let a = v[2].z;
        let dn = abs(dot(in.uv - vec2(0.5, v[2].x), vec2(-sin(a), cos(a))));
        r = r * smoothstep(v[2].y, v[2].y + 0.25, dn);
    }
    if (r < 0.5) { return S(in.uv); }
    let rings = max(i32(v[0].w), 1);
    var acc = S(in.uv);
    var wsum = 1.0;
    let blades = v[0].y;
    for (var ring = 1; ring <= rings; ring++) {
        let rr = f32(ring) / f32(rings);
        let cnt = ring * 6;
        for (var k = 0; k < cnt; k++) {
            let a = f32(k) / f32(cnt) * 2.0 * PI;
            var scale = 1.0;
            if (blades >= 3.0) {
                // polygon aperture: distance to the edge of a regular polygon
                let seg = 2.0 * PI / blades;
                let aa = a - v[0].z;
                let local = aa - seg * floor(aa / seg) - seg * 0.5;
                scale = cos(seg * 0.5) / cos(local);
            }
            let o = vec2(cos(a), sin(a)) * rr * r * scale;
            let s = S(in.uv + o / d);
            let l = luma(unpre(s));
            let w = 1.0 + max(l - v[1].x, 0.0) * v[1].y;
            acc += s * w;
            wsum += w;
        }
    }
    return acc / wsum;
}

// ------------------------------------------------------------ 3×3 convolutions and relief

// i.x: 0 sharpen (v0.x amount), 1 emboss (v0.x angle rad, v0.y relief, v0.z amount),
// 2 bevel (v0.x angle rad, v0.y size px, v0.z intensity), 3 lighting (v0.x angle rad, v0.y relief,
// v0.z intensity, v1 colour; up to 4 point lights in v3..v6: xy uv, z intensity, w radius uv)
@fragment
fn fs_conv(in: VOut) -> @location(0) vec4<f32> {
    let d = dims();
    let t = 1.0 / d;
    let v = fx.v;
    let s = S(in.uv);
    switch fx.i.x {
        case 0u: {
            let n = S(in.uv + vec2(0.0, -t.y)) + S(in.uv + vec2(0.0, t.y)) + S(in.uv + vec2(-t.x, 0.0)) + S(in.uv + vec2(t.x, 0.0));
            return max(s + (s * 4.0 - n) * v[0].x * 0.25, vec4(0.0));
        }
        case 1u: {
            let dir = vec2(cos(v[0].x), sin(v[0].x)) * t * max(v[0].y, 1.0);
            let a = luma(unpre(S(in.uv - dir)));
            let b = luma(unpre(S(in.uv + dir)));
            let e = vec3(0.5 + (a - b) * 2.0);
            return mix(s, vec4(dec(e) * s.a, s.a), v[0].z);
        }
        default: {
            let k = max(v[0].y, 1.0);
            let hl = Sz(in.uv - vec2(t.x * k, 0.0)).a;
            let hr = Sz(in.uv + vec2(t.x * k, 0.0)).a;
            let hu = Sz(in.uv - vec2(0.0, t.y * k)).a;
            let hd = Sz(in.uv + vec2(0.0, t.y * k)).a;
            var h = vec2(hr - hl, hd - hu);
            if (fx.i.x == 3u) {
                let l0 = luma(unpre(S(in.uv)));
                let lx = luma(unpre(S(in.uv + vec2(t.x, 0.0))));
                let ly = luma(unpre(S(in.uv + vec2(0.0, t.y))));
                h = vec2(lx - l0, ly - l0) * v[0].y * 20.0;
            }
            let nrm = normalize(vec3(-h * 2.0, 1.0));
            if (fx.i.x == 2u) {
                let l = normalize(vec3(cos(v[0].x), sin(v[0].x), 0.7));
                let dd = dot(nrm, l) - l.z;
                let add = dd * v[0].z;
                return vec4(max(s.rgb + vec3(add) * s.a, vec3(0.0)), s.a);
            }
            // lighting: ambient + directional + point lights
            var light = 0.25 + max(dot(nrm, normalize(vec3(cos(v[0].x), sin(v[0].x), 0.8))), 0.0) * 0.75;
            for (var k2 = 0; k2 < 4; k2++) {
                let pl = v[3 + k2];
                if (pl.z > 0.0) {
                    let dv = vec3((pl.xy - in.uv) * vec2(d.x / d.y, 1.0), 0.15);
                    let fall = clamp(1.0 - length(dv.xy) / max(pl.w, 1e-3), 0.0, 1.0);
                    light += max(dot(nrm, normalize(dv)), 0.0) * pl.z * fall * fall;
                }
            }
            let lit = s.rgb * mix(1.0, light, v[0].z) * mix(vec3(1.0), v[1].rgb, v[0].z);
            return vec4(lit, s.a);
        }
    }
}

// ------------------------------------------------------------ alpha morphology

// Max (dilate) or min (erode) of alpha over a disc. v0: radius px, mode (0 dilate, 1 erode),
// softness px; i.x: 0 plain result, 1 stroke ring (v1 colour, v0.w position: 0 outside, 1 inside, 2 centre),
// 2 outline only.
@fragment
fn fs_morph(in: VOut) -> @location(0) vec4<f32> {
    let d = dims();
    let v = fx.v;
    let s = S(in.uv);
    let r = max(v[0].x, 0.0);
    var mx = s.a;
    var mn = s.a;
    let rings = clamp(i32(ceil(r / 1.5)), 1, 12);
    for (var ring = 1; ring <= rings; ring++) {
        let rr = r * f32(ring) / f32(rings);
        let cnt = 8 + ring * 4;
        for (var k = 0; k < cnt; k++) {
            let a = f32(k) / f32(cnt) * 2.0 * PI;
            let q = Sz(in.uv + vec2(cos(a), sin(a)) * rr / d).a;
            mx = max(mx, q);
            mn = min(mn, q);
        }
    }
    if (fx.i.x == 0u) {
        let na = select(mx, mn, v[0].y > 0.5);
        let f = select(0.0, na / s.a, s.a > 1e-5);
        return select(vec4(unpre(s) * na, na), s * f, s.a > 1e-5 && v[0].y > 0.5);
    }
    var ring = 0.0;
    let pos = u32(v[0].w);
    if (pos == 0u) { ring = clamp(mx - s.a, 0.0, 1.0); }
    else if (pos == 1u) { ring = clamp(s.a - mn, 0.0, 1.0); }
    else { ring = clamp(mx - mn, 0.0, 1.0); }
    let col = vec4(v[1].rgb, 1.0) * v[1].a * ring;
    if (fx.i.x == 2u) { return col; }
    if (pos == 0u) { return s + col * (1.0 - s.a); }
    return col + s * (1.0 - col.a);
}

// ------------------------------------------------------------ flow-guided blur

// aux: optical flow (rg, source texels per frame, from the previous frame); v0.x shutter fraction, v0.y samples.
@fragment
fn fs_flow(in: VOut) -> @location(0) vec4<f32> {
    let d = dims();
    let fd = vec2<f32>(textureDimensions(flowt));
    let q = clamp(vec2<i32>(in.uv * fd), vec2(0), vec2<i32>(fd) - 1);
    // flow is in pixels of its own (half) resolution
    let f = textureLoad(flowt, q, 0).xy / fd * d;
    let n = max(i32(fx.v[0].y), 1);
    var acc = vec4(0.0);
    for (var k = 0; k < n; k++) {
        let t = ((f32(k) + 0.5) / f32(n) - 0.5) * fx.v[0].x;
        acc += Sz(in.uv + f * t / d);
    }
    return acc / f32(n);
}

// ------------------------------------------------------------ transitions

// src = outgoing, aux = incoming, aux2 = luma matte. v0: progress, softness, angle rad, direction (0 l, 1 r, 2 u, 3 d, 4 angle);
// v1 colour; v2.x motion-blur samples. i.x kind (order of the schema enumeration).
// The frame size in pixels (v3.zw): a missing side is a 1×1 texture.
fn fdims() -> vec2<f32> { return fx.v[3].zw; }
fn dir_vec() -> vec2<f32> {
    let dd = u32(fx.v[0].w);
    switch dd {
        case 0u: { return vec2(-1.0, 0.0); }
        case 1u: { return vec2(1.0, 0.0); }
        case 2u: { return vec2(0.0, -1.0); }
        case 3u: { return vec2(0.0, 1.0); }
        default: { return vec2(cos(fx.v[0].z), sin(fx.v[0].z)); }
    }
}
fn F(uv: vec2<f32>) -> vec4<f32> { return select(vec4(0.0), S(uv), in_unit(uv)); }
fn T(uv: vec2<f32>) -> vec4<f32> { return select(vec4(0.0), A(uv), in_unit(uv)); }
fn over(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> { return a + b * (1.0 - a.a); }
fn edge(x: f32, soft: f32) -> f32 { return smoothstep(-soft * 0.5 - 1e-4, soft * 0.5 + 1e-4, x); }
// D19 geometry, in frame pixels. The share of b at coordinate s: b where s < e − w, a beyond e,
// smoothstep across [e − w, e], with e = p (1 + w) and w the softness.
fn d19_b(s: f32, p: f32, soft: f32) -> f32 {
    let w = max(soft, 1e-4);
    let e = p * (1.0 + w);
    return 1.0 - smoothstep(e - w, e, s);
}
// The coordinate along the travel, 0 on the side the edge starts from and 1 on the far side.
fn d19_axis(uv: vec2<f32>, dir: vec2<f32>) -> f32 {
    let d = fdims();
    let proj = dot(uv * d, dir);
    let c = vec4(0.0, d.x * dir.x, d.y * dir.y, d.x * dir.x + d.y * dir.y);
    let lo = min(min(c.x, c.y), min(c.z, c.w));
    let hi = max(max(c.x, c.y), max(c.z, c.w));
    return (proj - lo) / max(hi - lo, 1e-6);
}
// Distance from the centre over half the diagonal.
// With <param> cx, cy the centre moves (v4.xy, pixels) and the radius is over the distance to the
// farthest corner (v4.z), as in the Python renderer.
fn d19_radius(uv: vec2<f32>) -> f32 {
    return length(uv * fdims() - fx.v[4].xy) / max(fx.v[4].z, 1e-6);
}
// The angle clockwise on screen from the ray at `start` radians (clockwise from +x), in turns.
fn d19_angle(uv: vec2<f32>, start: f32) -> f32 {
    let q = uv * fdims() - fx.v[4].xy;
    return fract((atan2(q.y, q.x) - start) / (2.0 * PI) + 2.0);
}
// The travel of a picture moved fully off the frame along dir, in uv units.
fn d19_travel(dir: vec2<f32>) -> vec2<f32> {
    let d = fdims();
    return dir * (abs(dir.x) * d.x + abs(dir.y) * d.y) / d;
}
// ---- the Python renderer's transitions (CONVENTIONS 5.17), in frame pixels (centres at i + ½)

// x snapped to a nearby integer: GPU division is not correctly rounded, and slats and stripes
// must switch exactly where the float64 reference does.
fn snap(x: f32) -> f32 {
    let r = round(x);
    return select(x, r, abs(x - r) < 1e-5 * max(1.0, abs(x)));
}

// Bilinear sample of t at pixel coordinates q, transparent outside (scenerender.transitions.sample).
fn bil(t: texture_2d<f32>, q: vec2<f32>) -> vec4<f32> {
    let dd = vec2<i32>(textureDimensions(t));
    let pp = q - 0.5;
    let f0 = floor(pp);
    let f = pp - f0;
    let i0 = vec2<i32>(f0);
    var acc = vec4(0.0);
    for (var j = 0; j < 2; j++) {
        for (var i = 0; i < 2; i++) {
            let c = i0 + vec2(i, j);
            let w = select(1.0 - f.x, f.x, i == 1) * select(1.0 - f.y, f.y, j == 1);
            if (w > 0.0 && all(c >= vec2(0)) && all(c < dd)) { acc += textureLoad(t, c, 0) * w; }
        }
    }
    return acc;
}
// Sides at pixel coordinates, optionally transposed (up/down run the horizontal code transposed).
fn PA(q: vec2<f32>, tr: bool) -> vec4<f32> { return bil(src, select(q, q.yx, tr)); }
fn PB(q: vec2<f32>, tr: bool) -> vec4<f32> { return bil(aux, select(q, q.yx, tr)); }

// translate(px, o) then streak(length bl along d): scenerender.transitions.moved. The translated
// picture is a frame-sized image, so the streak reads nothing outside the frame.
fn in_frame(q: vec2<f32>) -> bool { return all(q >= vec2(0.0)) && all(q < fdims()); }
fn moved(t: texture_2d<f32>, q: vec2<f32>, o: vec2<f32>, bl: f32, d: vec2<f32>) -> vec4<f32> {
    let n = round(bl);
    if (bl < 1.0 || n < 2.0) { return bil(t, q - o); }
    if (abs(d.x) > 0.999 || abs(d.y) > 0.999) {
        let w = i32(n) | 1;
        let r = w / 2;
        let ax = select(vec2(0.0, 1.0), vec2(1.0, 0.0), abs(d.x) > 0.999);
        var acc = vec4(0.0);
        for (var j = -r; j <= r; j++) {
            let qq = q - ax * f32(j);
            if (in_frame(qq)) { acc += bil(t, qq - o); }
        }
        return acc / f32(w);
    }
    let k = i32(min(24.0, max(3.0, ceil(bl / 3.0))));
    var acc = vec4(0.0);
    for (var i = 0; i < k; i++) {
        let qq = q - d * ((f32(i) / f32(k - 1) - 0.5) * bl);
        if (in_frame(qq)) { acc += bil(t, qq - o); }
    }
    return acc / f32(k);
}

// A picture of size wh turned by (cos, sin) of its yaw about its vertical centre line, centred at
// (x0, 0, z0), seen by a pinhole camera at distance cam (scenerender.transitions.plane_homography),
// sampled at output pixel q; transparent behind the camera.
fn plane_px(t: texture_2d<f32>, q: vec2<f32>, pl: vec4<f32>, wh: vec2<f32>, cam: f32, tr: bool, shade: f32) -> vec4<f32> {
    let c = pl.x; let s = pl.y; let x0 = pl.z; let z0 = pl.w;
    // H = P · A with A = [[c, 0, x0 − c w/2], [0, 1, −h/2], [s, 0, z0 − s w/2 + cam]]
    let a0 = vec3(c, 0.0, x0 - c * wh.x * 0.5);
    let a1 = vec3(0.0, 1.0, -wh.y * 0.5);
    let a2 = vec3(s, 0.0, z0 - s * wh.x * 0.5 + cam);
    let h0 = a0 * cam + a2 * (wh.x * 0.5);
    let h1 = a1 * cam + a2 * (wh.y * 0.5);
    let h2 = a2;
    // solve H · (sx, sy, 1)ᵀ ∝ (qx, qy, 1)ᵀ by Cramer's rule on the rows
    let g0 = h0 - h2 * q.x;
    let g1 = h1 - h2 * q.y;
    let den = g0.x * g1.y - g0.y * g1.x;
    if (abs(den) < 1e-12) { return vec4(0.0); }
    let sx = (-g0.z * g1.y + g0.y * g1.z) / den;
    let sy = (-g0.x * g1.z + g0.z * g1.x) / den;
    let fw = h2.x * sx + h2.y * sy + h2.z;
    if (fw <= 1e-6) { return vec4(0.0); }
    let v = bil(t, select(vec2(sx, sy), vec2(sy, sx), tr));
    return vec4(v.rgb * shade, v.a);
}

// Film roll's strip at time t (scenerender.transitions.fx.film_roll's render), in the
// (transposed) frame of size wh; v4 gap, border, curvature, holes; v5.x sign.
fn film_strip(q: vec2<f32>, t: f32, wh: vec2<f32>, tr: bool, col: vec4<f32>) -> vec4<f32> {
    let w = wh.x; let h = wh.y;
    let gap = fx.v[4].x; let border = fx.v[4].y; let bend = fx.v[4].z; let holes = fx.v[4].w;
    let sgn = fx.v[5].x;
    let strength = sin(PI * t);
    let k = bend * strength;
    let v = (q.x / w - 0.5) * 2.0;
    var curved = q.x;
    if (k > 1e-7) { curved = w * (0.5 + 0.5 * asin(clamp(v * k, -1.0, 1.0)) / max(asin(k), 1e-7)); }
    let length = w * (1.0 + gap);
    let coord = curved + sgn * t * length;
    let rail = border * h * strength;
    let sy = (q.y - rail) / max(h - 2.0 * rail, 1.0) * h;
    let interior = q.y >= rail && q.y < h - rail;
    var movie = vec4(0.0);
    let ua = coord;
    let ub = coord - sgn * length;
    if (ua >= 0.0 && ua < w && interior) { movie += PA(vec2(ua, sy), tr); }
    if (ub >= 0.0 && ub < w && interior) { movie += PB(vec2(ub, sy), tr); }
    let in_frame = (ua >= 0.0 && ua < w) || (ub >= 0.0 && ub < w);
    if (!(interior && in_frame)) { movie += col; }
    let pitch = length / holes;
    let m = coord + pitch * 0.5;
    let hx = abs(m - floor(m / pitch) * pitch - pitch * 0.5) < pitch * 0.22;
    let hy = abs(q.y - rail * 0.5) < rail * 0.24 || abs(q.y - (h - rail * 0.5)) < rail * 0.24;
    if (hx && hy) { movie = vec4(0.0); }
    let shade = sqrt(max(1.0 - (v * k) * (v * k), 0.0));
    return vec4(movie.rgb * (1.0 - 0.3 * strength * (1.0 - shade)), movie.a);
}

// A u16 stored as two bytes in a data texel pair (hi, lo).
fn u16_at(t: vec4<f32>, k: u32) -> f32 {
    if (k == 0u) { return round(t.x) * 256.0 + round(t.y); }
    return round(t.z) * 256.0 + round(t.w);
}
// Glitch: the picture before its RGB split at pixel xy (i32 coordinates, wrapped in x).
fn glitch_res(x: i32, y: i32, wh: vec2<i32>, swap: bool) -> vec4<f32> {
    let xw = ((x % wh.x) + wh.x) % wh.x;
    var o = select(textureLoad(src, vec2(xw, y), 0), textureLoad(aux, vec2(xw, y), 0), swap);
    let nsl = i32(fx.v[4].x);
    for (var k = 0; k < nsl; k++) {
        let t0 = textureLoad(aux2, vec2(2 * k, 0), 0);
        let t1 = textureLoad(aux2, vec2(2 * k + 1, 0), 0);
        let y0 = i32(u16_at(t0, 0u)); let hh = i32(u16_at(t0, 1u));
        let off = i32(u16_at(t1, 0u)); let other = u16_at(t1, 1u) > 0.5;
        if (y >= y0 && y < y0 + hh) {
            let sx = (((xw - off) % wh.x) + wh.x) % wh.x;
            o = select(textureLoad(src, vec2(sx, y), 0), textureLoad(aux, vec2(sx, y), 0), other != swap);
        }
    }
    let nb = i32(fx.v[4].y);
    let base = 2 * nsl;
    for (var k = 0; k < nb; k++) {
        let t0 = textureLoad(aux2, vec2(base + 4 * k, 0), 0);
        let t1 = textureLoad(aux2, vec2(base + 4 * k + 1, 0), 0);
        let t2 = textureLoad(aux2, vec2(base + 4 * k + 2, 0), 0);
        let t3 = textureLoad(aux2, vec2(base + 4 * k + 3, 0), 0);
        let x0 = i32(u16_at(t0, 0u)); let y0 = i32(u16_at(t0, 1u));
        let bw = i32(u16_at(t1, 0u)); let bh = i32(u16_at(t1, 1u));
        let sx = i32(u16_at(t2, 0u)); let sy = i32(u16_at(t2, 1u));
        let other = u16_at(t3, 0u) > 0.5;
        if (xw >= x0 && xw < x0 + bw && y >= y0 && y < y0 + bh) {
            let c = vec2(sx + xw - x0, sy + y - y0);
            o = select(textureLoad(src, c, 0), textureLoad(aux, c, 0), other != swap);
        }
    }
    return o;
}

@fragment
fn fs_trans(in: VOut) -> @location(0) vec4<f32> {
    let p = clamp(fx.v[0].x, 0.0, 1.0);
    let soft = fx.v[0].y;
    let uv = in.uv;
    let d = dims();
    let asp = d.x / d.y;
    let dir = dir_vec();
    let col = vec4(fx.v[1].rgb, 1.0) * fx.v[1].a;
    switch fx.i.x {
        case 0u: { return select(F(uv), T(uv), p >= 0.5); }                       // cut
        case 1u: { return mix(F(uv), T(uv), p); }                                  // crossfade
        case 2u: { return min(F(uv) * min(1.0, 2.0 * (1.0 - p)) + T(uv) * min(1.0, 2.0 * p), vec4(1.0)); } // additive dissolve
        case 3u: { // dip to colour
            if (p < 0.5) { return mix(F(uv), col, p * 2.0); }
            return mix(col, T(uv), (p - 0.5) * 2.0);
        }
        case 4u: { return mix(F(uv), T(uv), d19_b(d19_axis(uv, dir), p, soft)); } // wipe (D19)
        case 5u, 6u, 7u, 8u: { // slide (= cover), push, cover, reveal (D19); v4.x streak px, v4.y span
            let q = uv * fdims();
            let bl = fx.v[4].x;
            let e = fx.v[4].y;
            if (fx.i.x == 6u) { return over(moved(aux, q, dir * e * (p - 1.0), bl, dir), moved(src, q, dir * e * p, bl, dir)); }
            if (fx.i.x == 8u) { return over(moved(src, q, dir * e * p, bl, dir), bil(aux, q)); }
            return over(moved(aux, q, dir * e * (p - 1.0), bl, dir), bil(src, q));
        }
        case 9u, 10u: { // zoom in / out: v4 centre, ka, kb; v5 m, wa
            let q = uv * fdims();
            let c = fx.v[4].xy;
            var a = vec4(0.0);
            var b = vec4(0.0);
            if (fx.v[5].y > 0.0) { a = bil(src, c + (q - c) / fx.v[4].z) * fx.v[5].y; }
            if (fx.v[5].x > 0.0) { b = bil(aux, c + (q - c) / fx.v[4].w); }
            return over(b * fx.v[5].x, a);
        }
        case 11u: { // spin: v4 centre, rotation of a and b (rad); v5 ka, kb, m, wa
            let q = uv * fdims() - fx.v[4].xy;
            let ra = -fx.v[4].z; let rb = -fx.v[4].w;
            let qa = vec2(q.x * cos(ra) - q.y * sin(ra), q.x * sin(ra) + q.y * cos(ra)) / fx.v[5].x + fx.v[4].xy;
            let qb = vec2(q.x * cos(rb) - q.y * sin(rb), q.x * sin(rb) + q.y * cos(rb)) / fx.v[5].y + fx.v[4].xy;
            var a = vec4(0.0);
            var b = vec4(0.0);
            if (fx.v[5].w > 0.0) { a = bil(src, qa) * fx.v[5].w; }
            if (fx.v[5].z > 0.0) { b = bil(aux, qb); }
            return over(b * fx.v[5].z, a);
        }
        case 12u: { // whip pan: v4 streak px, span, eased travel q
            let q = uv * fdims();
            let bl = fx.v[4].x; let e = fx.v[4].y; let tq = fx.v[4].z;
            return over(moved(aux, q, dir * e * (tq - 1.0), bl, dir), moved(src, q, dir * e * tq, bl, dir));
        }
        case 13u, 15u: { return mix(F(uv), T(uv), d19_b(d19_radius(uv), p, soft)); } // circle open, iris (D19)
        case 14u: { return mix(F(uv), T(uv), 1.0 - d19_b(d19_radius(uv), 1.0 - p, soft)); } // circle close: a closes
        case 16u: { return mix(F(uv), T(uv), d19_b(d19_angle(uv, -0.5 * PI), p, soft)); } // clock wipe from 12
        case 17u: { return mix(F(uv), T(uv), d19_b(d19_angle(uv, fx.v[0].z), p, soft)); } // radial wipe from @angle
        case 18u: { return mix(F(uv), T(uv), d19_b(abs(d19_axis(uv, dir) - 0.5) * 2.0, p, soft)); } // barn door
        case 20u: { // luma (D19): the matte's working-space Rec. 709 luminance; without one, a wipe
            var l = d19_axis(uv, dir);
            if (fx.i.y != 0u) {
                l = clamp(luma(A2(uv).rgb), 0.0, 1.0);
                if (fx.v[4].w >= 0.5) { l = 1.0 - l; } // <param name="invert">
            }
            return mix(F(uv), T(uv), d19_b(l, p, soft));
        }
        case 19u: { // blinds: v4.x slats
            let t = snap(d19_axis(uv, dir) * fx.v[4].x);
            return mix(F(uv), T(uv), d19_b(t - floor(t), p, soft));
        }
        case 22u: { // glitch: v4 slices, blocks, RGB split px, swap; slices and blocks in aux2
            let wh = vec2<i32>(fdims());
            let xy = vec2<i32>(floor(uv * fdims()));
            if (fx.v[4].x < 0.0) { return select(textureLoad(src, xy, 0), textureLoad(aux, xy, 0), fx.v[4].w > 0.5); }
            let sw = fx.v[4].w > 0.5;
            let sh = i32(fx.v[4].z);
            let o = glitch_res(xy.x, xy.y, wh, sw);
            if (sh == 0) { return o; }
            let r = glitch_res(xy.x - sh, xy.y, wh, sw).r;
            let b = glitch_res(xy.x + sh, xy.y, wh, sw).b;
            return vec4(min(vec3(r, o.g, b), vec3(max(o.a, 0.0))), o.a);
        }
        case 24u, 25u, 31u: { // flip, cube, carousel: planes in perspective (v4, v5; v6 shades, cam, flags)
            let tr = fx.v[6].w >= 16.0;
            let fl = u32(fx.v[6].w);
            var wh = fdims();
            if (tr) { wh = wh.yx; }
            var q = uv * fdims();
            if (tr) { q = q.yx; }
            var acc = vec4(0.0);
            // planes far to near: bit 2 says plane 1 is farther; bits 0, 1 visibility; bits 3 which side plane 0 shows
            let first_is_1 = (fl & 4u) != 0u;
            for (var k = 0; k < 2; k++) {
                let idx = select(k, 1 - k, first_is_1);
                if ((fl & (1u << u32(idx))) == 0u) { continue; }
                let pl = select(fx.v[4], fx.v[5], idx == 1);
                let shade = select(fx.v[6].x, fx.v[6].y, idx == 1);
                let is_b = select(idx == 1, (fl & 8u) != 0u, fx.i.x == 24u);
                var c: vec4<f32>;
                if (is_b) { c = plane_px(aux, q, pl, wh, fx.v[6].z, tr, shade); } else { c = plane_px(src, q, pl, wh, fx.v[6].z, tr, shade); }
                acc = over(c, acc);
            }
            return acc;
        }
        case 26u: { // page curl: v4 span, radius, axis position, shadow strength
            let q = uv * fdims();
            let e = fx.v[4].x; let r = fx.v[4].y; let c = fx.v[4].z;
            let u = e * (1.0 - d19_axis(uv, dir));
            var res = bil(aux, q);
            let beyond = max((u - (c + r)) / (0.8 * r), 0.0);
            let shadow = 1.0 - fx.v[4].w * exp(-beyond) * select(0.0, 1.0, u > c);
            res = vec4(res.rgb * shadow, res.a);
            let flat_ = clamp(c - u + 0.5, 0.0, 1.0);
            if (flat_ > 0.0) { res = over(bil(src, q) * flat_, res); }
            if (u >= c - 0.5 && u <= c + r) {
                let t = clamp((u - c) / r, 0.0, 1.0);
                let phi1 = asin(t);
                let du = c + r * phi1 - u;
                var front = bil(src, q - du * dir);
                front = vec4(front.rgb * (0.55 + 0.45 * cos(phi1)), front.a);
                res = over(front, res);
            }
            let phi2 = PI - asin(clamp((u - c) / r, 0.0, 1.0));
            let ub = select(c + PI * r + (c - u), c + r * phi2, u >= c);
            let back = bil(src, q - (ub - u) * dir);
            if (u <= c + r) {
                let ba = back.a;
                let shade = select(0.95, 0.75 + 0.25 * sin(clamp(phi2, 0.0, PI)), u >= c);
                let paper = vec3(0.82, 0.82, 0.8);
                res = over(vec4((back.rgb * 0.18 + paper * ba * 0.82) * shade, ba), res);
            }
            return res;
        }
        case 27u: { // film roll: v4 gap, border, curvature, holes; v5 sign, transposed, shutter, samples
            let tr = fx.v[5].y > 0.5;
            var wh = fdims();
            var q = uv * fdims();
            if (tr) { wh = wh.yx; q = q.yx; }
            if (p <= 0.0) { return PA(q, tr); }
            if (p >= 1.0) { return PB(q, tr); }
            let n = i32(fx.v[5].w);
            var acc = vec4(0.0);
            for (var i = 0; i < n; i++) {
                let dt = ((f32(i) + 0.5) / f32(n) - 0.5) * fx.v[5].z;
                acc += film_strip(q, clamp(p + dt, 0.0, 1.0), wh, tr, col);
            }
            return acc / f32(n);
        }
        case 28u: { // stripe: v4 count, stagger
            let n = fx.v[4].x; let st = fx.v[4].y;
            var t = d19_axis(uv, dir);
            let k = min(floor(snap(d19_axis(uv, vec2(-dir.y, dir.x)) * n)), n - 1.0);
            if (k - 2.0 * floor(k / 2.0) > 0.5) { t = 1.0 - t; }
            let v = (t + st * k / max(1.0, n - 1.0)) / (1.0 + st);
            return mix(F(uv), T(uv), d19_b(v, p, soft));
        }
        case 29u: { // squash: v4 lead, ka, (unused); v5 trail, kb
            let q = uv * fdims();
            var a = vec4(0.0);
            var b = vec4(0.0);
            let ka = fx.v[4].z; let kb = fx.v[5].z;
            if (ka > 1e-8) { a = bil(src, fx.v[4].xy + (q - fx.v[4].xy) + dir * dot(q - fx.v[4].xy, dir) * (1.0 / ka - 1.0)); }
            if (kb > 1e-8) { b = bil(aux, fx.v[5].xy + (q - fx.v[5].xy) + dir * dot(q - fx.v[5].xy, dir) * (1.0 / kb - 1.0)); }
            return over(b, a);
        }
        case 30u: { // shuffle: v4 offset, ka, kb, a on top
            let q = uv * fdims();
            let c = fdims() * 0.5;
            let o = dir * fx.v[4].x;
            let a = bil(src, c + (q - o - c) / fx.v[4].y);
            let b = bil(aux, c + (q + o - c) / fx.v[4].z);
            return select(over(b, a), over(a, b), fx.v[4].w > 0.5);
        }
        case 32u: { // light leak: v4..v6 blobs (centre, σ, strength), v7 m, k; colours in aux2
            let q = uv * fdims();
            let base = mix(bil(src, q), bil(aux, q), fx.v[7].x);
            if (fx.v[7].y < 1e-3) { return base; }
            var l = vec3(0.0);
            for (var i = 0; i < 3; i++) {
                let bl = fx.v[4 + i];
                let dq = q - bl.xy;
                let blob = exp(-dot(dq, dq) / (2.0 * bl.z * bl.z));
                l += blob * textureLoad(aux2, vec2(i, 0), 0).rgb * bl.w;
            }
            l = clamp(l * fx.v[7].y, vec3(0.0), vec3(1.0));
            return vec4(base.rgb + l * (base.a - base.rgb), base.a);
        }
        case 34u: { // pixelize, block averages (output: one texel per block): v4 block size, offsets
            let bs = i32(fx.v[4].x);
            let o = vec2<i32>(fx.v[4].yz);
            let wh = vec2<i32>(fdims());
            let b0 = vec2<i32>(in.pos.xy) * bs - o;
            var acc = vec4(0.0);
            var cnt = 0.0;
            for (var j = 0; j < bs; j++) {
                for (var i = 0; i < bs; i++) {
                    let c = b0 + vec2(i, j);
                    if (all(c >= vec2(0)) && all(c < wh)) { acc += textureLoad(src, c, 0); cnt += 1.0; }
                }
            }
            return acc / max(cnt, 1.0);
        }
        case 36u: { // box filter of the Python renderer's gaussian: v4 width (odd), axis (0 x, 1 y), clamp
            let r = i32(fx.v[4].x) / 2;
            let ax = select(vec2(1, 0), vec2(0, 1), fx.v[4].y > 0.5);
            let dd = vec2<i32>(textureDimensions(src));
            let xy = vec2<i32>(in.pos.xy);
            var acc = vec4(0.0);
            for (var j = -r; j <= r; j++) {
                var c = xy + ax * j;
                if (fx.v[4].z > 0.5) { c = clamp(c, vec2(0), dd - 1); }
                if (all(c >= vec2(0)) && all(c < dd)) { acc += textureLoad(src, c, 0); }
            }
            return acc / fx.v[4].x;
        }
        case 39u: { // sampled Gaussian along an axis, zero outside: v4 σ, axis (0 x, 1 y)
            let sg = fx.v[4].x;
            let r = max(1, i32(ceil(sg * 3.0)));
            let ax = select(vec2(1, 0), vec2(0, 1), fx.v[4].y > 0.5);
            let dd = vec2<i32>(textureDimensions(src));
            let xy = vec2<i32>(in.pos.xy);
            var acc = vec4(0.0);
            var norm = 0.0;
            for (var j = -r; j <= r; j++) {
                let w = exp(-f32(j * j) / (2.0 * sg * sg));
                norm += w;
                let c = xy + ax * j;
                if (all(c >= vec2(0)) && all(c < dd)) { acc += textureLoad(src, c, 0) * w; }
            }
            return acc / norm;
        }
        case 37u: { // block mean by factor v4.x, the edge extended (downsampling for large blurs)
            let f = i32(fx.v[4].x);
            let dd = vec2<i32>(textureDimensions(src));
            let b0 = vec2<i32>(in.pos.xy) * f;
            var acc = vec4(0.0);
            for (var j = 0; j < f; j++) {
                for (var i = 0; i < f; i++) { acc += textureLoad(src, clamp(b0 + vec2(i, j), vec2(0), dd - 1), 0); }
            }
            return acc / f32(f * f);
        }
        case 38u: { // separable bilinear upsampling by v4.x, edge clamped
            let f = fx.v[4].x;
            let dd = vec2<i32>(textureDimensions(src));
            let c = clamp((floor(in.pos.xy) + 0.5) / f - 0.5, vec2(0.0), vec2<f32>(dd - 1));
            let i0 = vec2<i32>(floor(c));
            let i1 = min(i0 + 1, dd - 1);
            let fr = c - vec2<f32>(i0);
            let top = mix(textureLoad(src, i0, 0), textureLoad(src, vec2(i1.x, i0.y), 0), fr.x);
            let bot = mix(textureLoad(src, vec2(i0.x, i1.y), 0), textureLoad(src, i1, 0), fr.x);
            return mix(top, bot, fr.y);
        }
        case 35u: { // pixelize, expansion: src is the block texture; v4 block size, offsets
            let bs = i32(fx.v[4].x);
            let o = vec2<i32>(fx.v[4].yz);
            let xy = vec2<i32>(floor(uv * fdims()));
            return textureLoad(src, (xy + o) / bs, 0);
        }
        default: { return mix(F(uv), T(uv), p); }
    }
}
