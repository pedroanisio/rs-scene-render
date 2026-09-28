// Transfer functions, ids in the order of transferType:
// 0 auto, 1 srgb, 2 linear, 3 bt1886, 4 gamma22, 5 gamma26, 6 pq, 7 hlg, 8 slog3, 9 logc3,
// 10 logc4, 11 vlog, 12 clog3, 13 redlog3g10, 14 flog2, 15 nlog, 16 acescc, 17 acescct.
// Scene-linear results put diffuse white at 1.0 (PQ 203 cd/m², HLG 75 %), as color.rs does.

const PQ_M1: f32 = 0.1593017578125;
const PQ_M2: f32 = 78.84375;
const PQ_C1: f32 = 0.8359375;
const PQ_C2: f32 = 18.8515625;
const PQ_C3: f32 = 18.6875;
const HLG_A: f32 = 0.17883277;
const HLG_B: f32 = 0.28466892;
const HLG_C: f32 = 0.55991073;
const HLG_REF: f32 = 0.26496256;

fn p10(x: f32) -> f32 { return pow(10.0, x); }
fn log10_(x: f32) -> f32 { return log(x) / 2.302585093; }

fn tf_decode(id: u32, v: f32) -> f32 {
  switch (id) {
    case 1u: { if (v <= 0.04045) { return v / 12.92; } return pow((v + 0.055) / 1.055, 2.4); }
    case 3u: { return pow(max(v, 0.0), 2.4); }
    case 4u: { return pow(max(v, 0.0), 2.2); }
    case 5u: { return pow(max(v, 0.0), 2.6); }
    case 6u: {
      let p = pow(max(v, 0.0), 1.0 / PQ_M2);
      return pow(max(p - PQ_C1, 0.0) / (PQ_C2 - PQ_C3 * p), 1.0 / PQ_M1) * (10000.0 / 203.0);
    }
    case 7u: {
      var e = v * v / 3.0;
      if (v > 0.5) { e = (exp((v - HLG_C) / HLG_A) + HLG_B) / 12.0; }
      return e / HLG_REF;
    }
    case 8u: {
      if (v >= 171.2102946929 / 1023.0) { return p10((v * 1023.0 - 420.0) / 261.5) * 0.19 - 0.01; }
      return (v * 1023.0 - 95.0) * 0.01125 / (171.2102946929 - 95.0);
    }
    case 9u: {
      if (v > 5.367655 * 0.010591 + 0.092809) { return (p10((v - 0.385537) / 0.247190) - 0.052272) / 5.555556; }
      return (v - 0.092809) / 5.367655;
    }
    case 10u: {
      let a = (262144.0 - 16.0) / 117.45; let b = 928.0 / 1023.0; let c = 95.0 / 1023.0;
      let s = (7.0 * 0.6931471806 * exp2(7.0 - 14.0 * c / b)) / (a * b);
      let t = (exp2(14.0 * (-c / b) + 6.0) - 64.0) / a;
      if (v >= 0.0) { return (exp2(14.0 * (v - c) / b + 6.0) - 64.0) / a; }
      return v * s + t;
    }
    case 11u: { if (v < 0.181) { return (v - 0.125) / 5.6; } return p10((v - 0.598206) / 0.241514) - 0.00873; }
    case 12u: {
      var x: f32;
      if (v < 0.097465473) { x = -(p10((0.12783901 - v) / 0.36726845) - 1.0) / 14.98325; }
      else if (v <= 0.15277891) { x = (v - 0.12512219) / 1.9754798; }
      else { x = (p10((v - 0.12240537) / 0.36726845) - 1.0) / 14.98325; }
      return x * 0.9;
    }
    case 13u: { if (v < 0.0) { return v / 15.1927 - 0.01; } return (p10(v / 0.224282) - 1.0) / 155.975327 - 0.01; }
    case 14u: { if (v >= 0.100686685370811) { return (p10((v - 0.384316) / 0.245281) - 0.064829) / 5.555556; } return (v - 0.092864) / 8.799461; }
    case 15u: { if (v < 452.0 / 1023.0) { let q = v * 1023.0 / 650.0; return q * q * q - 0.0075; } return exp((v * 1023.0 - 619.0) / 150.0); }
    case 16u: {
      if (v < (9.72 - 15.0) / 17.52) { return (exp2(v * 17.52 - 9.72) - exp2(-16.0)) * 2.0; }
      return exp2(v * 17.52 - 9.72);
    }
    case 17u: { if (v <= 0.155251141552511) { return (v - 0.0729055341958355) / 10.5402377416545; } return exp2(v * 17.52 - 9.72); }
    default: { return v; }
  }
}

fn tf_encode(id: u32, v: f32) -> f32 {
  switch (id) {
    case 1u: { if (v <= 0.0031308) { return v * 12.92; } return 1.055 * pow(v, 1.0 / 2.4) - 0.055; }
    case 3u: { return pow(max(v, 0.0), 1.0 / 2.4); }
    case 4u: { return pow(max(v, 0.0), 1.0 / 2.2); }
    case 5u: { return pow(max(v, 0.0), 1.0 / 2.6); }
    case 6u: {
      let y = pow(max(v * (203.0 / 10000.0), 0.0), PQ_M1);
      return pow((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y), PQ_M2);
    }
    case 7u: {
      let e = max(v * HLG_REF, 0.0);
      if (e <= 1.0 / 12.0) { return sqrt(3.0 * e); }
      return HLG_A * log(12.0 * e - HLG_B) + HLG_C;
    }
    case 8u: {
      if (v >= 0.01125) { return (420.0 + log10_((v + 0.01) / 0.19) * 261.5) / 1023.0; }
      return (v * (171.2102946929 - 95.0) / 0.01125 + 95.0) / 1023.0;
    }
    case 9u: { if (v > 0.010591) { return 0.247190 * log10_(5.555556 * v + 0.052272) + 0.385537; } return 5.367655 * v + 0.092809; }
    case 10u: {
      let a = (262144.0 - 16.0) / 117.45; let b = 928.0 / 1023.0; let c = 95.0 / 1023.0;
      let s = (7.0 * 0.6931471806 * exp2(7.0 - 14.0 * c / b)) / (a * b);
      let t = (exp2(14.0 * (-c / b) + 6.0) - 64.0) / a;
      if (v >= t) { return (log2(a * v + 64.0) - 6.0) / 14.0 * b + c; }
      return (v - t) / s;
    }
    case 11u: { if (v < 0.01) { return 5.6 * v + 0.125; } return 0.241514 * log10_(v + 0.00873) + 0.598206; }
    case 12u: {
      let x = v / 0.9;
      if (x < -0.014) { return -0.36726845 * log10_(1.0 - 14.98325 * x) + 0.12783901; }
      if (x <= 0.014) { return 1.9754798 * x + 0.12512219; }
      return 0.36726845 * log10_(14.98325 * x + 1.0) + 0.12240537;
    }
    case 13u: { let x = v + 0.01; if (x < 0.0) { return x * 15.1927; } return 0.224282 * log10_(x * 155.975327 + 1.0); }
    case 14u: { if (v >= 0.000889) { return 0.245281 * log10_(5.555556 * v + 0.064829) + 0.384316; } return 8.799461 * v + 0.092864; }
    case 15u: { if (v < 0.328) { return 650.0 * pow(v + 0.0075, 1.0 / 3.0) / 1023.0; } return (150.0 * log(v) + 619.0) / 1023.0; }
    case 16u: {
      if (v <= 0.0) { return (-16.0 + 9.72) / 17.52; }
      if (v < exp2(-15.0)) { return (log2(exp2(-16.0) + v * 0.5) + 9.72) / 17.52; }
      return (log2(v) + 9.72) / 17.52;
    }
    case 17u: { if (v <= 0.0078125) { return 10.5402377416545 * v + 0.0729055341958355; } return (log2(v) + 9.72) / 17.52; }
    default: { return v; }
  }
}

fn tf_decode3(id: u32, c: vec3<f32>) -> vec3<f32> { return vec3(tf_decode(id, c.r), tf_decode(id, c.g), tf_decode(id, c.b)); }
fn tf_encode3(id: u32, c: vec3<f32>) -> vec3<f32> { return vec3(tf_encode(id, c.r), tf_encode(id, c.g), tf_encode(id, c.b)); }
