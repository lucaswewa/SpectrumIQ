// The live mosaic's viewport (P7.3, ADR-0059): the part of a view one page shows. Each invocation
// is a word of the view, four of its pixels side by side, the first in the low byte. A pixel takes
// the page's pixel its centre falls on, through the LUT, darkened by how little of it is covered;
// a pixel off the page is left as it is.

struct Params {
    // The view's corner from the page's, and a view pixel's side, in the page's own pixels.
    origin: vec2<f32>,
    step: f32,
    // The page's side, in its pixels.
    side: u32,
    // The view's width, and its words a row.
    width: u32,
    stride: u32,
    // The view's words and rows this page may show: the first, and how many.
    first: vec2<u32>,
    count: vec2<u32>,
}

@group(0) @binding(0) var<storage, read> page: array<u32>;
@group(0) @binding(1) var<storage, read> lut: array<u32>;
@group(0) @binding(2) var<storage, read_write> view: array<u32>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.count.x || id.y >= params.count.y) {
        return;
    }
    let word = params.first.x + id.x;
    let row = params.first.y + id.y;
    let at = row * params.stride + word;
    let side = f32(params.side);
    var out = view[at];
    for (var k = 0u; k < 4u; k = k + 1u) {
        let x = word * 4u + k;
        if (x >= params.width) {
            break;
        }
        let p = floor(params.origin + (vec2<f32>(f32(x), f32(row)) + vec2<f32>(0.5)) * params.step);
        if (p.x < 0.0 || p.y < 0.0 || p.x >= side || p.y >= side) {
            continue;
        }
        let pixel = page[u32(p.y) * params.side + u32(p.x)];
        let level = f32(lut[pixel & 0xffffu]) * f32(pixel >> 16u) / 65535.0;
        let shift = k * 8u;
        out = (out & ~(0xffu << shift)) | (u32(round(level)) << shift);
    }
    view[at] = out;
}
