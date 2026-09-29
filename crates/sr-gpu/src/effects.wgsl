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

fn hash21(p: vec2<f32>, s: f32) -> f32 {
    var q = fract(vec3(p.x, p.y, s) * vec3(0.1031, 0.1030, 0.0973));
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}
fn vnoise(p: vec2<f32>, s: f32) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i, s);
    let b = hash21(i + vec2(1.0, 0.0), s);
    let c = hash21(i + vec2(0.0, 1.0), s);
    let d = hash21(i + vec2(1.0, 1.0), s);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
fn fbm(p: vec2<f32>, s: f32) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var k = 0; k < 5; k++) {
        v += a * vnoise(q, s + f32(k) * 7.0);
        q = q * 2.03 + vec2(17.0, 9.0);
        a *= 0.5;
    }
    return v;
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
@fragment
fn fs_down(in: VOut) -> @location(0) vec4<f32> {
    let t = 1.0 / dims() * fx.v[0].x;
    var s = S(in.uv) * 4.0;
    s += S(in.uv + vec2(-t.x, -t.y)) + S(in.uv + vec2(t.x, t.y));
    s += S(in.uv + vec2(t.x, -t.y)) + S(in.uv + vec2(-t.x, t.y));
    return s / 8.0;
}

// Dual-Kawase upsample; v0.x = offset in source texels.
@fragment
fn fs_up(in: VOut) -> @location(0) vec4<f32> {
    let t = 1.0 / dims() * fx.v[0].x;
    var s = S(in.uv + vec2(-2.0 * t.x, 0.0)) + S(in.uv + vec2(2.0 * t.x, 0.0));
    s += S(in.uv + vec2(0.0, -2.0 * t.y)) + S(in.uv + vec2(0.0, 2.0 * t.y));
    s += (S(in.uv + vec2(-t.x, t.y)) + S(in.uv + vec2(t.x, t.y)) + S(in.uv + vec2(t.x, -t.y)) + S(in.uv + vec2(-t.x, -t.y))) * 2.0;
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
        case 21u: { // film grain: v0.x amount, v0.y size, v0.z seed
            let q = floor(px / max(v[0].y, 0.5));
            let n = (hash21(q, v[0].z) + hash21(q + 0.5, v[0].z + 3.0) - 1.0);
            let e = enc(c);
            let l = luma(e);
            c = dec(max(e + n * v[0].x * 0.25 * (1.0 - abs(l * 2.0 - 1.0) * 0.5), vec3(0.0)));
        }
        case 22u: { // noise: v0.x amount, v0.y seed, v0.z monochrome
            let n = vec3(hash21(px, v[0].y), hash21(px, v[0].y + 1.7), hash21(px, v[0].y + 3.1)) - 0.5;
            let m = select(n, vec3(n.x), v[0].z > 0.5);
            c = dec(max(enc(c) + m * v[0].x, vec3(0.0)));
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
        case 64u: { // vignette: v0 amount, size, softness; v1 colour; v2.xy centre (uv)
            let asp = vec2(d.x / d.y, 1.0);
            let r = length((in.uv - v[2].xy) * asp) / length(0.5 * asp);
            let w = smoothstep(v[0].y - v[0].z, v[0].y + v[0].z * 0.5, r) * v[0].x;
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
            let n = fbm(in.uv * 1.5 + vec2(t * 0.15, t * 0.05), v[0].z);
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
            let n = fbm(px / max(v[0].x, 1.0) * v[0].y + vec2(v[0].z), v[0].w) * v[1].x;
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
            let o = vec2(fbm(q + vec2(v[0].z), v[0].w), fbm(q + vec2(5.2, 1.3) + vec2(v[0].z), v[0].w)) - 0.5;
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
            let o = vec2(vnoise(q + vec2(0.0, v[0].z * 2.0), 3.0), vnoise(q + vec2(4.0, v[0].z * 2.0), 5.0)) - 0.5;
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
        case 14u, 15u: { // chromatic aberration (radial, v0.x px at edge) / rgb split (v0.xy px)
            var o = v[0].xy / d;
            if (fx.i.x == 14u) { o = (in.uv - c) * 2.0 * v[0].x / d; }
            let r = Sz(in.uv + o);
            let g = S(in.uv);
            let b = Sz(in.uv - o);
            return vec4(r.r, g.g, b.b, max(g.a, max(r.a, b.a)));
        }
        case 16u: { // glitch: v0 amount, seed, time bucket, block px
            let row = floor(in.uv.y * d.y / max(v[0].w, 2.0));
            let h = hash21(vec2(row, v[0].z), v[0].y);
            var o = 0.0;
            if (h < v[0].x * 0.5) { o = (hash21(vec2(row, v[0].z + 1.0), v[0].y) - 0.5) * 0.2 * v[0].x; }
            let split = v[0].x * 6.0 / d.x;
            let r = Sz(in.uv + vec2(o + split, 0.0));
            let g = Sz(in.uv + vec2(o, 0.0));
            let b = Sz(in.uv + vec2(o - split, 0.0));
            return vec4(r.r, g.g, b.b, max(g.a, max(r.a, b.a)));
        }
        case 17u: { // vhs: v0 amount, time, seed
            let row = in.uv.y * d.y;
            let j = (vnoise(vec2(row * 0.05, v[0].y * 8.0), v[0].z) - 0.5) * 6.0 * v[0].x / d.x;
            let bleed = 3.0 * v[0].x / d.x;
            let g = Sz(in.uv + vec2(j, 0.0));
            let r = Sz(in.uv + vec2(j + bleed, 0.0));
            let b = Sz(in.uv + vec2(j - bleed, 0.0));
            let n = (hash21(floor(in.uv * d), v[0].y * 60.0 + v[0].z) - 0.5) * 0.08 * v[0].x;
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
fn Fb(uv: vec2<f32>, o: vec2<f32>) -> vec4<f32> {
    let n = max(i32(fx.v[2].x), 1);
    var a = vec4(0.0);
    for (var k = 0; k < n; k++) { a += F(uv + o * ((f32(k) + 0.5) / f32(n) - 0.5)); }
    return a / f32(n);
}
fn Tb(uv: vec2<f32>, o: vec2<f32>) -> vec4<f32> {
    let n = max(i32(fx.v[2].x), 1);
    var a = vec4(0.0);
    for (var k = 0; k < n; k++) { a += T(uv + o * ((f32(k) + 0.5) / f32(n) - 0.5)); }
    return a / f32(n);
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
    let speed = 4.0 * p * (1.0 - p); // motion-blur length, peaks mid-transition
    switch fx.i.x {
        case 0u: { return select(F(uv), T(uv), p >= 0.5); }                       // cut
        case 1u: { return mix(F(uv), T(uv), p); }                                  // crossfade
        case 2u: { return min(F(uv) * min(1.0, 2.0 * (1.0 - p)) + T(uv) * min(1.0, 2.0 * p), vec4(1e4)); } // additive dissolve
        case 3u: { // dip to colour
            if (p < 0.5) { return mix(F(uv), col, p * 2.0); }
            return mix(col, T(uv), (p - 0.5) * 2.0);
        }
        case 4u: { // wipe: the edge travels along the direction, revealing from the opposite side
            let x = dot(uv - 0.5, dir) + 0.5;
            let w = edge(p * (1.0 + soft) - soft * 0.5 - x, soft);
            return mix(F(uv), T(uv), w);
        }
        case 5u, 6u, 7u, 8u: { // slide (incoming slides in), push (both move), cover, reveal (outgoing slides out)
            let o = dir * p;
            let blur = dir * speed * 0.08;
            if (fx.i.x == 5u || fx.i.x == 7u) { return over(Tb(uv + dir * (1.0 - p), blur), F(uv)); }
            if (fx.i.x == 6u) { return over(Tb(uv + dir * (1.0 - p), blur), Fb(uv - o, blur)); }
            return over(Fb(uv - o, blur), T(uv));
        }
        case 9u, 10u: { // zoom in / zoom out
            let zin = fx.i.x == 9u;
            let s1 = select(1.0 - 0.5 * p, 1.0 + p, zin);
            let s2 = select(2.0 - p, 0.5 + 0.5 * p, zin);
            let a = F(0.5 + (uv - 0.5) / s1);
            let b = T(0.5 + (uv - 0.5) / s2);
            return mix(a, b, smoothstep(0.3, 0.7, p));
        }
        case 11u: { // spin: rotate out and in about the centre
            let a = p * 2.0 * PI;
            let q = (uv - 0.5) * vec2(asp, 1.0);
            let r = vec2(q.x * cos(a) - q.y * sin(a), q.x * sin(a) + q.y * cos(a)) / vec2(asp, 1.0) + 0.5;
            let s = 1.0 - sin(p * PI) * 0.5;
            let rr = 0.5 + (r - 0.5) / s;
            return select(F(rr), T(rr), p >= 0.5);
        }
        case 12u: { // whip pan: fast push with heavy directional blur
            let e = p * p * (3.0 - 2.0 * p);
            let blur = dir * speed * 0.5;
            return over(Tb(uv + dir * (1.0 - e), blur), Fb(uv - dir * e, blur));
        }
        case 13u, 14u, 15u: { // circle open, circle close, iris (diamond)
            let q = (uv - 0.5) * vec2(asp, 1.0);
            let maxr = length(vec2(asp, 1.0)) * 0.5;
            var r = length(q);
            if (fx.i.x == 15u) { r = (abs(q.x) + abs(q.y)) * 0.7071; }
            if (fx.i.x == 14u) { let w = edge(r - (1.0 - p) * maxr * (1.0 + soft), soft * maxr); return mix(F(uv), T(uv), w); }
            let w = edge(p * maxr * (1.0 + soft) - r, soft * maxr);
            return mix(F(uv), T(uv), w);
        }
        case 16u: { // clock wipe from 12 o'clock
            let q = (uv - 0.5) * vec2(asp, 1.0);
            let a = fract(atan2(q.x, -q.y) / (2.0 * PI) + 1.0);
            return mix(F(uv), T(uv), edge(p - a, soft * 0.1));
        }
        case 17u: { // radial wipe (both directions from the top)
            let q = (uv - 0.5) * vec2(asp, 1.0);
            let a = abs(atan2(q.x, -q.y)) / PI;
            return mix(F(uv), T(uv), edge(p - a, soft * 0.1));
        }
        case 18u: { // barn door (horizontal unless direction is up/down)
            let vert = u32(fx.v[0].w) == 2u || u32(fx.v[0].w) == 3u;
            let x = abs(select(uv.x, uv.y, vert) - 0.5) * 2.0;
            return mix(F(uv), T(uv), edge(p * (1.0 + soft) - x, soft));
        }
        case 19u: { // blinds: 10 slats across the direction
            let x = fract(dot(uv, abs(dir)) * 10.0);
            return mix(F(uv), T(uv), edge(p * (1.0 + soft) - soft * 0.5 - x, soft));
        }
        case 20u: { // luma matte from aux2 (or the incoming luma)
            var l = luma(unpre(A2(uv)));
            if (fx.i.y == 0u) { l = luma(unpre(T(uv))); }
            return mix(F(uv), T(uv), edge(p * (1.0 + soft) - l, soft));
        }
        case 21u: { // blur through
            let r = sin(p * PI) * 0.02;
            var a = vec4(0.0);
            var b = vec4(0.0);
            for (var k = 0; k < 16; k++) {
                let ang = f32(k) * 2.39996;
                let o = vec2(cos(ang), sin(ang)) * r * sqrt(f32(k + 1) / 16.0) * vec2(1.0, asp);
                a += F(uv + o);
                b += T(uv + o);
            }
            return mix(a, b, p) / 16.0;
        }
        case 22u: { // glitch
            let row = floor(uv.y * 24.0);
            let h = hash21(vec2(row, floor(p * 20.0)), 7.0);
            let o = (h - 0.5) * 0.2 * sin(p * PI);
            let w = select(0.0, 1.0, h < p);
            return mix(F(uv + vec2(o, 0.0)), T(uv + vec2(o * 0.5, 0.0)), w);
        }
        case 23u: { // pixelize
            let cells = mix(256.0, 12.0, sin(p * PI));
            let q = (floor(uv * vec2(cells * asp, cells)) + 0.5) / vec2(cells * asp, cells);
            return mix(F(q), T(q), smoothstep(0.4, 0.6, p));
        }
        case 24u: { // flip around the vertical axis
            let s = abs(cos(p * PI));
            let x = (uv.x - 0.5) / max(s, 1e-3) + 0.5;
            let q = vec2(x, uv.y);
            return select(F(q), T(vec2(1.0 - x, uv.y) * vec2(-1.0, 1.0) + vec2(1.0, 0.0)), p >= 0.5);
        }
        case 25u: { // cube: two faces on a rotating box (perspective approximation)
            let a = p * PI * 0.5;
            let wf = cos(a);
            let wt = sin(a);
            let split = wf / (wf + wt);
            if (uv.x < split * 1.0 && wf > 1e-3) {
                let x = uv.x / split;
                let sh = 1.0 - 0.15 * wt * (1.0 - x);
                return F(vec2(x, 0.5 + (uv.y - 0.5) / sh));
            }
            let x = (uv.x - split) / max(1.0 - split, 1e-3);
            let sh = 1.0 - 0.15 * wf * x;
            return T(vec2(x, 0.5 + (uv.y - 0.5) / sh));
        }
        case 26u: { // page curl from the right edge
            let x = uv.x;
            let edge_x = 1.0 - p * 1.2;
            if (x > edge_x + 0.1) { return T(uv); }
            if (x > edge_x) {
                let f = (x - edge_x) / 0.1;
                let back = F(vec2(edge_x - (x - edge_x), uv.y));
                return over(vec4(back.rgb * (0.6 + 0.4 * f), back.a) * (1.0 - f * 0.2), T(uv));
            }
            let shadow = clamp((edge_x - x) * 8.0, 0.0, 1.0);
            return F(uv) * mix(0.7, 1.0, shadow);
        }
        case 27u: { // film roll: vertical scroll with frame gap
            let y = uv.y + p;
            let blur = vec2(0.0, speed * 0.1);
            if (y < 1.0) { return Fb(vec2(uv.x, y), blur); }
            return Tb(vec2(uv.x, y - 1.0), blur);
        }
        case 28u: { // stripes: alternating bands slide in
            let band = floor(uv.y * 8.0);
            let s = select(-1.0, 1.0, (i32(band) & 1) == 0);
            let o = (1.0 - p) * s;
            let q = vec2(uv.x + o, uv.y);
            return over(T(q), F(uv));
        }
        case 29u: { // squash: outgoing squashes to a line, incoming stretches out
            if (p < 0.5) { let s = 1.0 - p * 2.0; return F(vec2(uv.x, 0.5 + (uv.y - 0.5) / max(s, 1e-3))); }
            let s = (p - 0.5) * 2.0;
            return T(vec2(uv.x, 0.5 + (uv.y - 0.5) / max(s, 1e-3)));
        }
        case 30u: { // shuffle: outgoing slides out and under while incoming comes over
            let e = sin(p * PI);
            let a = F(uv - vec2(e * 0.5, 0.0));
            let b = T(uv + vec2(e * 0.5, 0.0));
            return select(over(a, b), over(b, a), p >= 0.5);
        }
        case 31u: { // carousel: outgoing moves left and shrinks, incoming arrives from the right
            let sa = 1.0 - 0.3 * p;
            let sb = 0.7 + 0.3 * p;
            let a = F(0.5 + (uv - vec2(0.5 - 0.6 * p, 0.5)) / sa);
            let b = T(0.5 + (uv - vec2(0.5 + 0.6 * (1.0 - p), 0.5)) / sb);
            return select(over(a, b), over(b, a), p >= 0.5);
        }
        case 32u: { // light leak: warm burn over a crossfade
            let n = fbm(uv * 2.0 + vec2(p, 0.0), 11.0);
            let burn = sin(p * PI) * smoothstep(0.3, 0.9, n + uv.x * 0.4);
            let base = mix(F(uv), T(uv), smoothstep(0.35, 0.65, p));
            return base + vec4(col.rgb * burn * 2.0, 0.0) * max(base.a, burn);
        }
        case 33u: { // morph: displace both by their luminance difference while crossfading
            let a = F(uv);
            let b = T(uv);
            let dlt = (luma(unpre(b)) - luma(unpre(a))) * 0.1;
            return mix(F(uv + vec2(dlt, dlt) * p), T(uv - vec2(dlt, dlt) * (1.0 - p)), p);
        }
        default: { return mix(F(uv), T(uv), p); }
    }
}
