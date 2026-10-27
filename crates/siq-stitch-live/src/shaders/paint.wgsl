// The live canvas's paint (P7.2, ADR-0059): a frame laid over one page. Each pixel of the
// rectangle painted is mapped back into the frame, sampled bilinearly, and composited over the
// page with an alpha that ramps from 0 at the frame's edge to 1 a feather in. A page's pixel is
// its value in the low half of a word, its coverage in the high half.

struct Params {
    // The page's pixels to the frame's: [a, b, tx] and [c, d, ty].
    row0: vec4<f32>,
    row1: vec4<f32>,
    frame_size: vec2<u32>,
    rect_origin: vec2<u32>,
    rect_size: vec2<u32>,
    // In frame pixels.
    feather: f32,
    // The page's width.
    page: u32,
}

@group(0) @binding(0) var<storage, read> frame: array<u32>;
@group(0) @binding(1) var<storage, read_write> canvas: array<u32>;
@group(0) @binding(2) var<uniform> params: Params;

// The frame's sample at a pixel, clamped to the frame: two to a word, the first in the low half.
fn sample_at(x: i32, y: i32) -> f32 {
    let cx = u32(clamp(x, 0, i32(params.frame_size.x) - 1));
    let cy = u32(clamp(y, 0, i32(params.frame_size.y) - 1));
    let i = cy * params.frame_size.x + cx;
    return f32((frame[i / 2u] >> ((i % 2u) * 16u)) & 0xffffu);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.rect_size.x || id.y >= params.rect_size.y) {
        return;
    }
    let px = params.rect_origin + id.xy;
    let centre = vec2<f32>(px) + vec2<f32>(0.5);
    let f = vec2<f32>(
        params.row0.x * centre.x + params.row0.y * centre.y + params.row0.z,
        params.row1.x * centre.x + params.row1.y * centre.y + params.row1.z,
    );
    let size = vec2<f32>(params.frame_size);
    let edge = min(min(f.x, size.x - f.x), min(f.y, size.y - f.y));
    if (edge <= 0.0) {
        return;
    }
    let alpha = min(edge / params.feather, 1.0);
    // Between the four pixel centres around `f`.
    let g = f - vec2<f32>(0.5);
    let base = floor(g);
    let t = g - base;
    let x0 = i32(base.x);
    let y0 = i32(base.y);
    let top = sample_at(x0, y0) * (1.0 - t.x) + sample_at(x0 + 1, y0) * t.x;
    let bottom = sample_at(x0, y0 + 1) * (1.0 - t.x) + sample_at(x0 + 1, y0 + 1) * t.x;
    let value = top * (1.0 - t.y) + bottom * t.y;
    let at = px.y * params.page + px.x;
    let old = canvas[at];
    let old_value = f32(old & 0xffffu);
    let old_cover = f32(old >> 16u) / 65535.0;
    let cover = alpha + old_cover * (1.0 - alpha);
    let blended = (value * alpha + old_value * old_cover * (1.0 - alpha)) / cover;
    canvas[at] = u32(round(min(blended, 65535.0))) | (u32(round(cover * 65535.0)) << 16u);
}
