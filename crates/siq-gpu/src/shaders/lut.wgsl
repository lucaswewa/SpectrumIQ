// The display LUT (P7.1, ADR-0058): each u16 sample to its u8, four to an output word. The
// samples come two to a u32, the first in the low half; the LUT has one u32 for each of the 65 536
// values, its u8 in the low byte.

struct Params {
    // The samples.
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<storage, read> samples: array<u32>;
@group(0) @binding(1) var<storage, read> lut: array<u32>;
@group(0) @binding(2) var<storage, read_write> mapped: array<u32>;
@group(0) @binding(3) var<uniform> params: Params;

const PER_GROUP: u32 = 256u;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    // One output word, four samples, an invocation: across, then down (`Gpu::groups`).
    let word = id.x + id.y * groups.x * PER_GROUP;
    let first = word * 4u;
    if (first >= params.count) {
        return;
    }
    var packed = 0u;
    for (var k = 0u; k < 4u; k++) {
        let i = first + k;
        if (i < params.count) {
            let sample = (samples[i / 2u] >> ((i % 2u) * 16u)) & 0xffffu;
            packed = packed | ((lut[sample] & 0xffu) << (k * 8u));
        }
    }
    mapped[word] = packed;
}
