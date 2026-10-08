// GPU radix sort of splats by view depth (back to front): 4 passes of 8 bits,
// each a per-block histogram, one exclusive scan, and a stable scatter.

const BLOCK: u32 = 256u;

struct SortParams {
    n: u32,
    blocks: u32,
    shift: u32,
    pad: u32,
    view: mat4x4<f32>,
};

struct Splat {
    pos: vec4<f32>,
    cov_a: vec4<f32>,
    cov_b: vec4<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> sp: SortParams;
@group(0) @binding(1) var<storage, read> splats: array<Splat>;
@group(0) @binding(2) var<storage, read_write> keys_in: array<u32>;
@group(0) @binding(3) var<storage, read_write> vals_in: array<u32>;
@group(0) @binding(4) var<storage, read_write> keys_out: array<u32>;
@group(0) @binding(5) var<storage, read_write> vals_out: array<u32>;
@group(0) @binding(6) var<storage, read_write> hist: array<u32>;

var<workgroup> local_hist: array<atomic<u32>, 256>;
var<workgroup> local_digits: array<u32, 256>;
var<workgroup> partial: array<u32, 256>;

@compute @workgroup_size(256)
fn cs_keys(@builtin(global_invocation_id) g: vec3<u32>) {
    let i = g.x + g.y * 65535u * BLOCK;
    if (i >= sp.n) { return; }
    let z = (sp.view * vec4(splats[i].pos.xyz, 1.0)).z;
    // far first: larger depth → smaller key; behind the camera sorts last
    keys_in[i] = select(0xFFFFFFFFu, ~bitcast<u32>(z), z > 0.0);
    vals_in[i] = i;
}

@compute @workgroup_size(256)
fn cs_hist(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let block = wg.x + wg.y * 65535u;
    atomicStore(&local_hist[t], 0u);
    workgroupBarrier();
    let i = block * BLOCK + t;
    if (i < sp.n && block < sp.blocks) {
        atomicAdd(&local_hist[(keys_in[i] >> sp.shift) & 255u], 1u);
    }
    workgroupBarrier();
    if (block < sp.blocks) {
        hist[t * sp.blocks + block] = atomicLoad(&local_hist[t]);
    }
}

// One workgroup: exclusive prefix sum over hist[256 × blocks] (digit-major).
@compute @workgroup_size(256)
fn cs_scan(@builtin(local_invocation_index) t: u32) {
    let total = 256u * sp.blocks;
    let chunk = (total + 255u) / 256u;
    let start = t * chunk;
    let end = min(start + chunk, total);
    var sum = 0u;
    for (var k = start; k < end; k++) { sum += hist[k]; }
    partial[t] = sum;
    workgroupBarrier();
    // Hillis–Steele inclusive scan of the chunk sums
    for (var off = 1u; off < 256u; off = off << 1u) {
        var v = 0u;
        if (t >= off) { v = partial[t - off]; }
        workgroupBarrier();
        partial[t] += v;
        workgroupBarrier();
    }
    var run = partial[t] - sum;
    for (var k = start; k < end; k++) {
        let h = hist[k];
        hist[k] = run;
        run += h;
    }
}

@compute @workgroup_size(256)
fn cs_scatter(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let block = wg.x + wg.y * 65535u;
    let i = block * BLOCK + t;
    let live = i < sp.n && block < sp.blocks;
    var digit = 256u;
    var key = 0u;
    if (live) {
        key = keys_in[i];
        digit = (key >> sp.shift) & 255u;
    }
    local_digits[t] = digit;
    workgroupBarrier();
    if (!live) { return; }
    // stable rank among earlier elements of the block with the same digit
    var rank = 0u;
    for (var k = 0u; k < t; k++) {
        rank += select(0u, 1u, local_digits[k] == digit);
    }
    let dst = hist[digit * sp.blocks + block] + rank;
    keys_out[dst] = key;
    vals_out[dst] = vals_in[i];
}

// Software adapters execute each block on CPU lanes. A serial counting scatter
// performs one rank lookup per item, rather than comparing all earlier lanes.
// Keep cs_keys/hist/scan unchanged so even rounded depth ties retain their order.
@compute @workgroup_size(1)
fn cs_scatter_cpu(@builtin(workgroup_id) wg: vec3<u32>) {
    let block = wg.x + wg.y * 65535u;
    if (block >= sp.blocks) { return; }
    var ranks: array<u32, 256>;
    let start = block * BLOCK;
    let end = min(start + BLOCK, sp.n);
    for (var i = start; i < end; i++) {
        let key = keys_in[i];
        let digit = (key >> sp.shift) & 255u;
        let dst = hist[digit * sp.blocks + block] + ranks[digit];
        ranks[digit] += 1u;
        keys_out[dst] = key;
        vals_out[dst] = vals_in[i];
    }
}
